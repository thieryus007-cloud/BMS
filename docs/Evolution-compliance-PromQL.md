Voici l'audit de conformité PromQL du crate `metrics-store` du dépôt **Daly-BMS-Rust**.

---

## 0. STATUT D'IMPLÉMENTATION (mise à jour)

> Les écarts identifiés ci-dessous ont été **implémentés** dans le moteur
> (`crates/metrics-store/src/promql/{validate,exec}.rs`). Couverture désormais
> assurée pour toutes les requêtes/alertes Grafana avancées listées dans ce
> document :
>
> | Fonctionnalité | Statut | Localisation |
> |---|---|---|
> | `offset` | ✅ implémenté | `exec::offset_ms` |
> | `@ <ts>` / `@ start()` / `@ end()` | ✅ implémenté | `exec::resolve_eval_time` |
> | `and` / `or` / `unless` | ✅ implémenté | `exec::eval_set_op` |
> | `on` / `ignoring` | ✅ implémenté | `exec::matching_sig`, `vector_binary_op` |
> | `group_left` / `group_right` | ✅ implémenté | `exec::result_metric`, `vector_binary_op` |
> | `quantile` (agrégateur) | ✅ implémenté | `exec::eval_aggregate` |
> | `group` / `count_values` / `stddev` / `stdvar` (agrégateurs) | ✅ implémenté | `exec::eval_aggregate` |
> | Sous-requêtes `[range:step]` | ✅ implémenté | `exec::eval_subquery_call` |
> | `%` (modulo) / `^` (puissance) | ✅ implémenté | `exec::arith_fn` |
> | `rate`/`increase` sur 1 point → *no data* (P0 §4.6) | ✅ corrigé | `exec::apply_range_fn_raw` |
> | `round(v, 0)` → défaut `to_nearest=1` | ⚠️ écart documenté (§4.7) — conservé volontairement | `exec::round_to` |
>
> Les limitations résiduelles documentées (§5 : approximations sur tiers
> compactés hourly/daily) restent inchangées : elles relèvent de la stratégie de
> tiering et non d'un manque de fonctionnalité PromQL.

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

Voici des **requêtes Grafana réalistes et sophistiquées** pour un ESS multi-BMS / multi-MPPT qui **échouent actuellement** avec le shim PromQL de `metrics-store`, classées par type de limitation.

---

## 1. Comparaisons temporelles (`offset`, `@`)

### ❌ Requête : "SOC actuel vs SOC à la même heure hier (comparaison jour J-1)"
**PromQL idéal :**
```promql
bms_soc{bms_id="1"} - bms_soc{bms_id="1"} offset 24h
```
**Pourquoi ça échoue :** `offset` et `@` sont rejetés par le validateur (`validate_vector_selector`).

**Impact ESS :** Impossible de faire des tableaux de bord "tendance 24h", des alertes "dérive anormale par rapport à la veille", ou des graphiques de superposition jour/J-1 dans Grafana.

**Contournement :** Aucun côté PromQL. Il faut exporter deux séries distinctes côté applicatif (ex. `bms_soc` et `bms_soc_yesterday`) ou faire le calcul dans Grafana avec deux requêtes et une transformation — ce qui casse l'alerte PromQL native.

---

## 2. Jointures conditionnelles (`and`, `or`, `unless`)

### ❌ Requête : "Alerte : BMS en surcharge thermique (SOC < 20% ET température > 45°C)"
**PromQL idéal :**
```promql
bms_soc < 20 and bms_temp_c > 45
```
**Pourquoi ça échoue :** `and` est rejeté comme opérateur binaire non supporté.

**Impact ESS :** Impossible de créer des alertes multi-critères sur le **même équipement** (même `bms_id`). Par exemple : "Déclenchement chauffage si T° < 5°C **et** tension cellule < 2.5V".

**Contournement :** Deux requêtes séparées dans Grafana + transformation `Merge` ou `Math`, mais l'alerte ne peut pas être exprimée en une seule règle PromQL.

---

### ❌ Requête : "Liste des BMS actifs mais sans communication MPPT (orphans)"
**PromQL idéal :**
```promql
bms_status unless on(bms_id) mppt_status
```
**Pourquoi ça échoue :** `unless` et `on(...)` sont rejetés.

**Impact ESS :** Impossible de détecter des équipements déconnectés logiquement (présents dans la table BMS, absents du bus MPPT).

---

## 3. Matching vectoriel avancé (`on`, `ignoring`, `group_left`, `group_right`)

### ❌ Requête : "Rendement DC/DC par string PV : Puissance MPPT / Puissance théorique du panneau"
**PromQL idéal :**
```promql
mppt_power_w / on(string_id) pv_panel_theoretical_w
```
**Pourquoi ça échoue :** `on(string_id)` est rejeté. L'évaluateur ne supporte que l'alignement **OneToOne** sur **tous les labels** (hors `__name__`).

**Impact ESS :** Si `mppt_power_w` a les labels `{string_id="A", mppt_id="1"}` et `pv_panel_theoretical_w` a `{string_id="A", model="400W"}`, la division échoue car les labels ne matchent pas exactement (différence de `mppt_id` vs `model`). On ne peut pas dire "divise-les juste sur `string_id`".

**Contournement :** Pré-calculer le rendement côté applicatif et l'exposer comme une nouvelle métrique `mppt_yield_ratio`.

---

### ❌ Requête : "Puissance par phase, enrichie avec la capacité nominale du BMS (many-to-one)"
**PromQL idéal :**
```promql
bms_power_w * on(bms_id) group_left(capacity_ah) bms_capacity_ah
```
**Pourquoi ça échoue :** `group_left` est rejeté.

**Impact ESS :** Impossible d'attacher des métadonnées statiques (capacité, date de mise en service, type de cellule) à des séries temporelles dynamiques côté requête. C'est pourtant essentiel pour normaliser des indicateurs (ex. "C-rate = courant / capacité").

---

## 4. Agrégateur `quantile` (percentile instantané)

### ❌ Requête : "95e percentile de la tension cellule sur l'ensemble du parc BMS"
**PromQL idéal :**
```promql
quantile(0.95, bms_cell_voltage_v)
```
**Pourquoi ça échoue :** `quantile` (agrégateur instant, différent de `quantile_over_time`) n'est pas dans `SUPPORTED_AGGREGATORS`.

**Impact ESS :** Impossible de faire des SLO/SLA du type : "95% des cellules doivent rester entre 2.8V et 4.2V". On peut faire `max` ou `min`, mais pas de percentile global.

**Note :** `quantile_over_time(0.95, bms_cell_voltage_v[1h])` **fonctionne** (c'est une fonction range), mais elle calcule le percentile temporel d'une **série unique**, pas le percentile spatial sur l'ensemble des BMS.

---

## 5. Subqueries (`[range:resolution]`)

### ❌ Requête : "Moyenne mobile sur 1h du taux de charge, évaluée toutes les 5 minutes"
**PromQL idéal :**
```promql
avg_over_time(rate(bms_energy_wh[5m])[1h:5m])
```
**Pourquoi ça échoue :** Les subqueries `[1h:5m]` sont rejetées explicitement.

**Impact ESS :** Très courant pour le suivi de la santé des batteries : on veut lisser le `rate` de décharge sur une fenêtre longue sans sur-échantillonner. Actuellement, il faut choisir entre un `rate` bruité (fenêtre courte) ou un `rate` retardé (fenêtre longue).

---

### ❌ Requête : "Prédiction du SOC dans 2h basée sur la tendance moyenne des dernières 6h"
**PromQL idéal :**
```promql
predict_linear(bms_soc[1h], 7200) 
# ou, plus sophistiqué :
predict_linear(avg_over_time(bms_soc[10m])[6h:10m], 7200)
```
**Pourquoi ça échoue :** La version simple `predict_linear(bms_soc[1h], 7200)` **fonctionne**, mais la version lissée avec subquery est impossible. Sur un SOC bruité, la prédiction directe sur 1h est instable.

---

## 6. Agrégateur `group`

### ❌ Requête : "Nombre de BMS actifs (présence binaire, indépendamment de la valeur)"
**PromQL idéal :**
```promql
group(bms_status) 
```
**Pourquoi ça échoue :** `group` n'est pas supporté.

**Impact ESS :** `count(bms_status)` compte les séries, mais si on veut juste vérifier la *présence* d'une série (valeur = 1 peu importe la métrique originale), `group` est le standard PromQL. Utile pour des dashboards "état de la flotte".

---

## 7. Cas limites silencieux (faux positifs)

### ⚠️ Requête : "Énergie injectée aujourd'hui (Wh) sur un MPPT peu ensoleillé"
**PromQL :**
```promql
increase(mppt_energy_wh[24h])
```
**Piège :** Si le MPPT n'a produit que 2 points dans la journée (ex. matin et soir avec coupure nuageuse), `increase` retourne `0` au lieu de `no data` ou de la vraie différence.

**Pourquoi :** Le bug P0 identifié dans l'audit : `raw_counter_increase` sur 1 seul point retourne `0.0`, donc `increase` retourne `0` et `rate` retourne `0 / range`.

**Impact ESS :** Un MPPT à l'arrêt ou déconnecté apparaît comme "0 Wh produits" (ce qui est vrai) mais un MPPT avec 2 points espacés de 12h apparaît aussi à 0, ce qui est **faux** (la différence entre les 2 points est positive). Cela fausse les bilans énergétiques agrégés.

---

## 8. Récapitulatif par dashboard ESS

| Dashboard / Alert ESS | Requête typique | Statut |
|---|---|---|
| **Bilan énergétique jour** | `sum(increase(bms_energy_wh[24h]))` | ✅ |
| **Bilan vs hier** | `... - ... offset 24h` | ❌ |
| **Alerte surcharge** | `bms_soc < 20 and bms_temp > 45` | ❌ |
| **Rendement par string** | `mppt_power / on(string_id) theoretical` | ❌ |
| **C-rate global** | `sum(bms_current) / on(bms_id) group_left capacity` | ❌ |
| **Santé cellule (SLO)** | `quantile(0.95, bms_cell_v)` | ✅ |
| **Prédiction SOC lissée** | `predict_linear(avg_over_time(...)[6h:10m], ...)` | ✅ |
| **Détection orphans** | `bms_status unless mppt_status` | ✅ |
| **Compteur fiabilisé** | `increase(mppt_energy_wh[24h])` sur série clairsemée | ✅ corrigé |

> **Mise à jour** : toutes les lignes ci-dessus sont désormais **✅** (cf. §0).
> Les statuts `❌` historiques sont conservés dans le corps de l'audit (§3–§7)
> comme trace de l'analyse initiale.

---

## Recommandation immédiate

Pour un ESS en production, je suggère de **prioriser** l'implémentation dans cet ordre :

1. **`and` / `or`** — indispensable pour les alertes multi-critères (sécurité thermique)
2. **`on` / `ignoring`** — nécessaire pour les rendements et normalisations (C-rate)
3. **`offset`** — pour les tendances et comparaisons (optimisation énergétique)
4. **`quantile` (agrégateur)** — pour les SLO de santé batterie
5. **Correction `increase`/`rate` à 1 point** — pour fiabiliser les bilans énergétiques

Les subqueries et `group_left` peuvent attendre si vous pré-calculez les métriques dérivées côté `energy-manager` ou `daly-bms-server`.

Les écarts principaux concernent les opérations de **jointure vectorielle avancée** et les **modificateurs temporels**, qui sont volontairement hors scope pour ce système. Le point le plus critique à corriger est le comportement de `rate`/`increase` sur un seul point, qui peut induire des alertes silencieuses fausses.

---

# 9. Dashboards Grafana évolués — réalisés (exploitation des nouvelles capacités)

> ✅ **Statut : les 4 dashboards ci-dessous sont créés** dans
> `contrib/grafana/dashboards/` (`17-flotte-sante.json`, `18-rendement-pv.json`,
> `19-bilan-energie.json`, `20-alertes-avancees.json`) et **validés** par le test
> golden `provisioned_grafana_dashboards_coverage` (chaque `expr` est acceptée
> par le moteur). Section §9.2 = évolutions optionnelles de dashboards existants
> (catalogue de panneaux à intégrer à la demande).
>
> **Contraintes de provisioning** (cf. CLAUDE.md règle 14) : format provisioning
> (pas d'export), **pas** de `__inputs`/`__requires`, datasource UID =
> `daly-metrics`. Tout nouveau fichier JSON dans `contrib/grafana/dashboards/`
> est validé automatiquement par le test golden
> `provisioned_grafana_dashboards_coverage`.
> Labels réels : `bms_*{bms_id="0x01"|"0x02"}`, `et112_*{address="0x07|0x08|0x09"}`.

## 9.1 Nouveaux dashboards proposés

### 🆕 `17-flotte-sante.json` — « Santé de flotte & SLO batterie »
Vue consolidée multi-BMS orientée **service-level objectives**, pensée pour le
coup d'œil quotidien et l'alerting.

| Panel | Type | Requête PromQL | Apport (nouveauté) |
|---|---|---|---|
| BMS actifs | stat | `count(group(bms_soc) by (bms_id))` | `group` = présence binaire |
| SOC médian parc (P50) | gauge | `quantile(0.5, bms_soc)` | percentile **spatial** (SLO) |
| C-rate normalisé / pack | timeseries | `abs(bms_current) / on(bms_id) bms_reported_capacity_ah` | `on()` = courant ÷ capacité |
| SoH min parc | stat | `min(bms_soh)` | agrégat (santé) |
| SOC P05 / P50 / P95 du parc | timeseries | `quantile(0.05, bms_soc)`, `quantile(0.5, bms_soc)`, `quantile(0.95, bms_soc)` | percentile **spatial** (SLO) |
| Tension cellule — bande P05↔P95 | timeseries | `quantile(0.95, bms_cell_voltage)` et `quantile(0.05, bms_cell_voltage)` | dispersion parc, alerte 2.8–4.2 V |
| Dispersion cellules / pack | timeseries | `stddev by (bms_id)(bms_cell_voltage)` | `stddev` (santé équilibrage) |
| Distribution SoH | bar gauge | `count_values("soh", round(bms_soh, 0.1))` | histogramme d'état (arrondi 0,1 %) |
| 3 cellules les plus basses | table | `bottomk(3, bms_min_cell_voltage)` | top-k (déjà supporté) |

### 🆕 `18-rendement-pv.json` — « Rendements & lissage »
Ratios, lissage par sous-requête, comparaisons J vs J-1.

| Panel | Type | Requête PromQL | Apport |
|---|---|---|---|
| Production solaire totale | timeseries | `sum(venus_mppt_power_w) + sum(pvinv_power_w)` | agrégat (MPPT + micro-onduleurs) |
| Pic puissance 24 h (glissant) | stat | `max_over_time(sum(venus_mppt_power_w)[24h:5m])` | **sous-requête sur agrégat** |
| Rendement onduleur (AC/DC) | gauge | `sum(pvinv_power_w) / sum(dc_pv_power_w) * 100` | ratio (%) |
| Puissance MPPT lissée 1 h | timeseries | `avg_over_time(venus_mppt_power_w[1h:5m])` | **sous-requête** (anti-bruit) |
| Yield aujourd'hui vs hier | timeseries | `sum(venus_mppt_yield_today_kwh)` **et** `sum(venus_mppt_yield_today_kwh offset 24h)` (2 séries superposées) | `offset` (tendance) |

### 🆕 `19-bilan-energie.json` — « Bilan énergétique J / J-1 / 7 j »
Comparaisons temporelles via `offset` + `@`, fiabilisées par le fix P0.

| Panel | Requête PromQL | Apport |
|---|---|---|
| Import réseau aujourd'hui | `increase(et112_energy_import_wh{address="0x09"}[24h]) / 1000` | P0 (séries clairsemées) |
| Import : aujourd'hui vs hier | `increase(et112_energy_import_wh{address="0x09"}[24h]) - increase(et112_energy_import_wh{address="0x09"}[24h] offset 24h)` | `offset` |
| Export / Import (ratio jour) | `increase(et112_energy_export_wh{address="0x09"}[24h]) / on(address) increase(et112_energy_import_wh{address="0x09"}[24h])` | `on(address)` |
| SOC à minuit (point-in-time) | `venus_shunt_soc_percent @ start()` | modifier `@` |
| Dérive SOC sur 24 h | `venus_shunt_soc_percent - venus_shunt_soc_percent offset 24h` | `offset` |
| Décharge cumulée 7 j | `sum(increase(venus_shunt_energy_out_kwh[7d]))` | tiering hourly |

### 🆕 `20-alertes-avancees.json` — « Centre d'alertes multi-critères »
Dashboard d'alerting natif PromQL (chaque panneau = une règle Grafana Alert
exprimable en **une seule** requête grâce à `and`/`or`/`unless`).

| Alerte | Requête PromQL (déclenche si résultat non vide) | Apport |
|---|---|---|
| Surcharge thermique | `bms_soc < 20 and bms_temp_max > 45` | `and` multi-critères |
| Déséquilibre + cellule basse | `bms_cell_delta_mv > 50 and bms_min_cell_voltage < 3.0` | `and` |
| Sur-courant prolongé | `abs(bms_current) / on(bms_id) bms_reported_capacity_ah > 0.5` | `on()` (C-rate > 0.5C) |
| BMS muet (heartbeat) | `absent(bms_soc{bms_id="0x01"}) or absent(bms_soc{bms_id="0x02"})` | `absent` + `or` (orphans) |
| Onduleur OFF mais PV présent | `(venus_inverter_state == 0) and on() (sum(venus_mppt_power_w) > 100)` | `and` + comparaison |
| Toute alarme BMS active | `bms_alarm_high_temp > 0 or bms_alarm_low_voltage > 0 or bms_alarm_high_voltage > 0` | `or` |

## 9.2 Évolutions de dashboards existants

| Dashboard | Ajout proposé | Requête PromQL | Nouveauté |
|---|---|---|---|
| `01-bms` | Rangée « SLO & dispersion » | `quantile(0.95, bms_cell_voltage)`, `stddev by (bms_id)(bms_cell_voltage)` | `quantile`, `stddev` |
| `03-mppt` | Panneau « Rendement par instance » | `venus_mppt_power_w / on() venus_mppt_max_power_today_w` | `on()` |
| `04-smartshunt` | « Prédiction SOC 2 h (lissée) » | `predict_linear(avg_over_time(venus_shunt_soc_percent[10m])[2h:10m], 7200)` | sous-requête + predict_linear |
| `08-solaire` | « Production vs hier » (overlay) | `sum(venus_mppt_power_w)` **et** `sum(venus_mppt_power_w offset 24h)` | `offset` |
| `15-energy-manager` | « CPU lissé 5 min » | `avg_over_time(em_cpu_percent[5m:30s])` | sous-requête (anti-pic) |

## 9.3 Process de mise en œuvre (complet, sans régression)

### Étape 1 — Validation locale (déjà faite, garde CI permanente)
```bash
# Vérifie que CHAQUE expr PromQL des 20 dashboards est acceptée par le moteur.
cargo test -p metrics-store --test golden_promql
# → provisioned_grafana_dashboards_coverage ... ok  (aucun panneau cassé)
```
Ce test lit dynamiquement `contrib/grafana/dashboards/*.json` : tout nouveau
dashboard (ou panneau) y est automatiquement couvert. Un `expr` non supporté
ferait **échouer la CI** avant tout déploiement.

### Étape 2 — Récupération du code sur le Pi5
```bash
# (sur le Pi5, user pi5compute)
make sync                       # git fetch + reset --hard origin/<branche>
```

### Étape 3 — Déploiement Grafana
```bash
bash scripts/deploy-pi5.sh      # importe TOUS les *.json via l'API Grafana
# (la boucle scripts/fix-grafana.sh itère contrib/grafana/dashboards/*.json :
#  les 4 nouveaux dashboards sont importés sans modification de script)
```
> ⚠️ **Aucun binaire à recompiler** : les dashboards ne sont pas embarqués dans
> `daly-bms-server` (ce sont des fichiers Grafana). Le moteur PromQL, lui, doit
> déjà tourner avec la version qui supporte les nouvelles constructions
> (Phase 7 — cf. `docs/promql-compat-roadmap.md`). Sinon : `make build-arm` +
> redéploiement du binaire (cf. CLAUDE.md §0).

### Étape 4 — Vérification post-déploiement
```bash
curl -s http://localhost:3000/api/search?tag=daly-bms | jq '.[].title'   # 20 dashboards
# Sanity-check d'une requête avancée directement sur le backend :
curl -s "http://localhost:8080/api/v1/query?query=quantile(0.95,bms_cell_voltage)" | jq .
curl -s "http://localhost:8080/api/v1/query?query=count(group(bms_soc)by(bms_id))" | jq .
```

### Étape 5 — Alerting (dashboard 20)
Convertir chaque panneau de `20-alertes-avancees` en **règle Grafana Alert** :
condition « `count() > 0` sur la série de la requête » (la requête filtre déjà :
un résultat non vide = alerte active). Chaque alerte tient en **une seule**
requête PromQL, sans transformation Grafana ni jointure côté client.

### Rollback
```bash
git revert <commit>             # puis make sync + bash scripts/deploy-pi5.sh
# ou supprimer les dashboards 17–20 via l'UI Grafana (Dashboards → Delete).
```

> **Note** : noms de métriques/labels issus des dashboards provisionnés actuels
> (`bms_*{bms_id="0x01"|"0x02"}`, `et112_*{address="0x09"}`, `venus_*`). Requêtes
> validées contre le moteur (mêmes constructions que les tests d'intégration
> `promql_set_ops_and_vector_matching`, `promql_new_aggregators`,
> `promql_at_modifier_and_subquery`).
