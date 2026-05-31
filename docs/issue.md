Voici l'audit de conformité PromQL du crate `metrics-store` du dépôt **Daly-BMS-Rust**.

---

## 1. Résumé exécutif

Le crate `metrics-store` implémente un **shim PromQL personnalisé** exécuté sur une base redb (TSDB embarquée). Il repose sur le parser externe `promql-parser` v0.9.0 (dernière version stable, compatible Prometheus v2.45.0), sur lequel est construit un évaluateur maison avec une couche de validation par liste blanche.

**Verdict global :**  
✅ **Conforme pour un sous-ensemble fonctionnel bien délimité** (golden set + extensions).  
⚠️ **Non conforme** sur les opérations avancées de matching vectoriel, les sous-requêtes, les modificateurs temporels et certains agrégateurs.  
⚠️ **Approximations documentées** sur les données tierées (hourly/daily) pour les fonctions statistiques et les compteurs.

---

## 2. Méthodologie

L'audit a porté sur les fichiers suivants :
- `src/promql/mod.rs` — orchestration parse/validate/exec
- `src/promql/exec.rs` — évaluateur (`Evaluator`, `eval_range`, `eval_instant`)
- `src/promql/validate.rs` — liste blanche et rejet des constructions non supportées
- `src/promql/error.rs` — format d'erreur Prometheus
- `src/lib.rs` — tests d'intégration PromQL

Référence de conformité : sémantique Prometheus v2.45+ (PromQL officiel).

---

## 3. Matrice de conformité PromQL

| Catégorie | Fonction/Opérateur | Statut | Notes |
|---|---|---|---|
| **Sélecteurs** | `metric{}` instant | ✅ | Lookback 5 min configurable |
| | `= != =~ !~` | ✅ | Gestion correcte des valeurs vides pour `!=` et `!~` |
| | `offset` | ❌ | Rejeté par validate |
| | `@` | ❌ | Rejeté par validate |
| **Arithmétique** | `+ - * /` (vec×scalar) | ✅ | |
| | `+ - * /` (vec×vec) | ✅ | Alignement exact par labels (hors `__name__`) |
| **Comparaisons** | `== != > < >= <=` | ✅ | Filtre ou `bool` ; NaN toujours faux |
| **Agrégations** | `sum max min avg count` | ✅ | `by`/`without` supportés |
| | `topk` / `bottomk` | ✅ | Labels d'origine conservés (dont `__name__`) |
| | `quantile` | ❌ | Non supporté |
| | `group` | ❌ | Non supporté |
| | `count_values` | ❌ | Non supporté |
| **Set ops** | `and` / `or` / `unless` | ❌ | Rejetées |
| **Matching vectoriel** | `on` / `ignoring` | ❌ | Rejeté |
| | `group_left` / `group_right` | ❌ | Rejeté |
| **Fonctions fenêtre** | `increase` / `rate` | ✅ | Gestion des resets intermédiaires |
| | `irate` | ✅ | Raw uniquement ; approximé sur tier compacté |
| | `delta` | ✅ | |
| | `deriv` / `predict_linear` | ✅ | Régression linéaire moindres carrés |
| | `changes` / `resets` | ✅ | NaN consécutifs ignorés |
| | `avg/sum/min/max/count_over_time` | ✅ | |
| | `last_over_time` | ✅ | |
| | `stddev/stdvar_over_time` | ✅ | Approximé sur tiers compactés (moyennes) |
| | `quantile_over_time` | ✅ | Interpolation linéaire |
| | `absent_over_time` | ✅ | |
| **Fonctions instant** | `abs ceil floor round` | ✅ | `round(v, to_nearest)` honoré |
| | `clamp clamp_min clamp_max` | ✅ | NaN et `min>max` → NaN |
| | `sqrt exp ln log2 log10` | ✅ | |
| | `sgn` | ✅ | `sgn(0)=0`, `sgn(NaN)=NaN` |
| | `absent` | ✅ | |
| **Labels** | `label_replace` | ✅ | Regex ancrée `^(?:…)$` |
| | `label_join` | ✅ | |
| **Sous-requêtes** | `[range:step]` | ❌ | Rejetées |
| **Exposition** | `prom_text` | 🔶 | Hors scope de cet audit |

---

## 4. Écarts majeurs (non-conformités)

### 4.1 Opérateurs de matching vectoriel absents
Les opérateurs `on(...)`, `ignoring(...)`, `group_left` et `group_right` sont rejetés. L'évaluateur ne supporte que l'alignement **OneToOne** strict sur tous les labels (hors `__name__`).  
**Impact :** Impossible de faire des jointures partielles ou des opérations many-to-one/one-to-many.

### 4.2 Opérateurs ensemblistes
`and`, `or`, `unless` ne sont pas implémentés.  
**Impact :** Les requêtes de filtrage croisé (ex. `foo and bar`) doivent être décomposées côté client.

### 4.3 Sous-requêtes (`subquery`)
Rejetées avec message explicite.  
**Impact :** Les requêtes de type `avg_over_time(rate(foo[5m])[1h:1m])` ne passent pas.

### 4.4 Modificateurs temporels
`offset` et `@` sont rejetés.  
**Impact :** Pas de requêtes historiques point-in-time ni de décalage temporel.

### 4.5 Agrégateurs manquants
- `quantile(0.9, ...)` (agrégateur instant, différent de `quantile_over_time`)
- `group(...)`
- `count_values("label", ...)`

### 4.6 `rate` / `increase` avec un seul point
Dans Prometheus, `rate` et `increase` nécessitent **au moins 2 points** sous la fenêtre ; avec un seul point, ils retournent *no data*.  
Dans `exec.rs`, `raw_counter_increase` sur 1 point retourne `0.0`, donc `rate` retourne `0 / range` et `increase` retourne `0`.  
**Impact :** Faux positifs silencieux sur les séries très peu denses.

### 4.7 `round(v, 0)` — déviation sémantique
Prometheus : `round(v, 0)` → `NaN` (division par zéro).  
Le code : `to_nearest == 0.0` est remplacé par `1.0` (défaut défensif).  
**Impact :** Résultat différent de Prometheus pour ce cas limite.

---

## 5. Approximations et limitations documentées

### 5.1 Tiering (raw → hourly → daily)
L'évaluateur sélectionne automatiquement le tier selon la durée de la fenêtre :
- ≤ 7 j → raw
- ≤ 90 j → hourly
- > 90 j → daily

**Fonctions approximées sur tiers compactés :**
- `stddev_over_time` / `stdvar_over_time` : calculées sur les **moyennes** des buckets, pas sur les valeurs brutes. La variance des moyennes ≠ variance de population.
- `deriv` / `predict_linear` : régression sur les `avg` des buckets.
- `changes` / `resets` : séquence `first→last` par bucket ; les oscillations intra-bucket sont invisibles.
- `irate` : approximé par les deux derniers buckets.

**Reset de compteur invisible :** un reset à l'intérieur d'un bucket horaire/journalier est perdu (les points raw ont été purgés). Documenté en commentaire.

### 5.2 `absent()` sur expression complexe
Prometheus exige un `VectorSelector` simple. L'évaluateur accepte n'importe quelle expression valide, mais si ce n'est pas un sélecteur simple, les labels du résultat seront vides (car `vs_opt` est `None`). Comportement plus permissif mais légèrement différent.

---

## 6. Points forts et bonnes pratiques

1. **Validation explicite** : whitelist claire avec messages d'erreur formatés au standard Prometheus (`status=error`, `errorType=bad_data`/`execution`).
2. **Gestion des resets** : `counter_increase` gère correctement les resets intermédiaires (testé `increase` sur `[10, 20, 5, 15]` → `25`).
3. **NaN** : comparaisons avec NaN toujours fausses (conforme PromQL) ; `clamp` propage NaN.
4. **`__name__`** : retrait correct dans les agrégations et comparaisons ; conservation dans `topk`/`bottomk`.
5. **Optimisations mémoire** : transaction redb partagée, catalogue de séries chargé 1×, cache de matching par pointeur (`*const VectorSelector`), `Arc<<Labels>` pour éviter les clones.
6. **Tests exhaustifs** : couverture des cas limites (intervalles inversés, réutilisation d'`Evaluator`, compaction idempotente, fusion de buckets).
7. **`label_replace`** : regex ancrée correctement (`^(?:…)$`) avec expansion `$1`.

---

## 7. Recommandations prioritaires

| Priorité | Recommandation | Fichier concerné |
|---|---|---|
| 🔴 **P0** | Corriger `rate`/`increase` pour retourner `None` (pas `0`) quand il n'y a qu'**1 seul point** sous la fenêtre | `exec.rs` (`apply_range_fn_raw`) |
| 🔴 **P0** | Aligner `round(v, 0)` sur Prometheus (`NaN`) ou documenter explicitement l'écart | `exec.rs` (`round_to`) |
| 🟡 **P1** | Implémenter `and` / `or` / `unless` pour les alertes Grafana courantes | `validate.rs`, `exec.rs` |
| 🟡 **P1** | Implémenter `on` / `ignoring` (matching vectoriel restreint) | `validate.rs`, `exec.rs` |
| 🟡 **P1** | Ajouter `offset` (décalage temporel simple) | `validate.rs`, `exec.rs` |
| 🟢 **P2** | Ajouter l'agrégateur `quantile` (utilisé pour les SLO) | `validate.rs`, `exec.rs` |
| 🟢 **P2** | Documenter dans l'API HTTP les écarts de précision sur les tiers compactés | Documentation utilisateur |
| 🔵 **P3** | Mettre à jour `promql-parser` si une v0.10+ sort avec des correctifs | `Cargo.toml` |

---

## 8. Verdict

Le crate `metrics-store` offre une **implémentation PromQL robuste et bien testée pour un usage embarqué (ESS)**. La couverture du "golden set" est complète et les extensions (math, labels, prédiction, `absent`) sont bien intégrées.

**Conformité estimée : ~70 %** du langage PromQL standard, avec une conformité **~95 %** sur le sous-ensemble déclaré supporté.

Les écarts principaux concernent les opérations de **jointure vectorielle avancée** et les **modificateurs temporels**, qui sont volontairement hors scope pour ce système. Le point le plus critique à corriger est le comportement de `rate`/`increase` sur un seul point, qui peut induire des alertes silencieuses fausses.
