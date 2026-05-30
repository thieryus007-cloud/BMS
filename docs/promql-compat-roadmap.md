# PromQL compatibility roadmap — metrics-store

> **✅ ÉTAT : phases 1, 2, 3a, 3b, 3c + Phase 4 (math & labels) implémentées
> et testées.** Le sous-ensemble PromQL supporté inclut désormais le
> groupement `by`/`without`, les comparaisons (+ `bool`), `topk`/`bottomk`,
> `irate`, les fonctions math (`sqrt exp ln log2 log10 sgn clamp`) et la
> manipulation de labels (`label_replace`, `label_join`), en plus du rejet
> explicite du vector matching non trivial. Voir la matrice de compat à jour
> dans `docs/plan_migration_vm_redb.md` §6.4-6.5. Le document ci-dessous reste
> comme référence de conception.

> **But du document** : plan d'implémentation **autoportant** pour élargir la
> compatibilité PromQL du moteur `metrics-store` (le shim que Grafana
> interroge directement). Conçu pour être **repris dans une conversation
> neuve** : tout le contexte, les fichiers, les formes exactes de l'AST, les
> esquisses de code et les tests y sont.
>
> Branche de travail : 
> max, `main` + 1 active). Workflow par phase : commit → push → PR vers `main`
> → après merge `make sync && bash scripts/deploy-pi5.sh`.

---

## 0. Contexte (architecture réelle)

Grafana (:3000) et le dashboard interne SSR interrogent **directement**
`daly-bms-server` (:8080), qui implémente une **API compatible Prometheus**
adossée à `metrics-store` (redb). **Il n'y a NI Prometheus NI scraping** (migré
depuis VictoriaMetrics, Phase 5). redb est **mono-processus** ⇒ un plugin
Grafana→redb direct est impossible ; le shim PromQL est la bonne approche
(décision validée).

```
RS485 (BMS Daly + ET112 + irradiance) ─┐ (lecture directe)
Victron → NanoPi (dbus-mqtt-venus) ─────┼─► daly-bms-server (Rust) ─► metrics-store / redb
energy-manager ─────────────────────────┘ (MQTT)   │   (tiering raw→hourly→daily)
                                                    └─► API PromQL-compat :8080 ◄── Grafana (PromQL)
```

### Fichiers clés
| Rôle | Chemin |
|------|--------|
| Validation (liste blanche AST) | `crates/metrics-store/src/promql/validate.rs` |
| Évaluateur | `crates/metrics-store/src/promql/exec.rs` |
| Erreurs (format Prometheus) | `crates/metrics-store/src/promql/error.rs` |
| Reader redb (helpers de scan) | `crates/metrics-store/src/reader.rs` |
| Tests golden (dashboards) | `crates/metrics-store/tests/golden_promql.rs` |
| Endpoints HTTP consommateurs | `crates/daly-bms-server/src/api/redb.rs`, `state.rs` |

### Sous-ensemble supporté aujourd'hui (état de départ)
- Sélecteurs : `=` `!=` `=~` `!~` (pas `offset`, pas `@`).
- Binaires arith : `+ - * /` (vec/scalar ; vec/vec **aligné exact tous-labels**
  via `align_and_op`, drop `__name__`).
- Agrégateurs : `sum max min avg count` **sans `by`/`without`** (collapse total,
  labels vides).
- Fonctions fenêtre : `rate increase delta {avg,sum,min,max,count,last}_over_time`
  (increase/rate gèrent les resets de compteur via `counter_increase`,
  `raw_counter_increase`, `buckets_counter_increase`).
- Fonctions instant : `abs ceil floor round clamp_min clamp_max`.
- **Non supporté** : subqueries `[r:s]`, `offset`, `@`, comparaisons, `bool`,
  set ops (`and/or/unless`), `by/without`, `on/ignoring/group_left/right`,
  `topk/bottomk/quantile/stddev/group/count_values`, `histogram_quantile`,
  `label_replace/label_join/vector/scalar`, `irate/idelta/deriv/predict_linear`,
  `%` `^`, fonctions math/date.

### ⚠️ Risque actuel (motive la Phase 1)
`validate_aggregate` **ne rejette pas** le modifier `by/without`, et
`eval_aggregate` l'**ignore** → `sum by (bms_id)(m)` renvoie une valeur
**collapsée fausse, sans erreur**. Idem `on()/ignoring()/group_left` dans les
binaires (alignement exact tous-labels au lieu du matching demandé). Nos 16
dashboards n'utilisent **aucune** de ces constructions (vérifié : 9 agrégations,
toutes nues) ⇒ **aucune régression** à durcir ce comportement.

### AST `promql-parser` 0.9 — formes EXACTES (vérifiées dans la crate)
```rust
// --- Agrégation ---
pub struct AggregateExpr {
    pub op: TokenType,                  // op.to_string() => "sum","max","topk",...
    pub expr: Box<Expr>,
    pub param: Option<Box<Expr>>,       // k de topk/bottomk/quantile
    pub modifier: Option<LabelModifier>,
    pub mod_span: ...,
}
pub enum LabelModifier { Include(Labels), Exclude(Labels) } // Include=by, Exclude=without
impl LabelModifier { pub fn labels(&self) -> &Labels; pub fn is_include(&self) -> bool; }

// --- Liste de labels de groupement ---
// NB : c'est le type `promql_parser::label::Labels`, PAS le BTreeMap de exec.rs.
pub struct Labels { pub labels: Vec<Label> }   // Label = String (nom de label)

// --- Binaire ---
pub struct BinaryExpr {
    pub op: TokenType, pub lhs: Box<Expr>, pub rhs: Box<Expr>,
    pub modifier: Option<BinModifier>,
}
pub struct BinModifier {
    pub card: VectorMatchCardinality,
    pub matching: Option<LabelModifier>,   // on(...) / ignoring(...)
    pub return_bool: bool,
}
pub enum VectorMatchCardinality { OneToOne, ManyToOne(Labels), OneToMany(Labels), ManyToMany }
```
> Dans `exec.rs`, le type de labels d'un échantillon est l'alias
> `type Labels = BTreeMap<String,String>` — **à ne pas confondre** avec
> `promql_parser::label::Labels` (la liste de noms du modifier). Désambiguïser
> les imports.

### Règles transverses (toutes phases)
1. **Aucune régression** : `provisioned_grafana_dashboards_coverage` (211 expr,
   16 dashboards) + tous les tests existants restent verts.
2. 1 phase = **1 PR** dédiée : tests unitaires + golden + `cargo build -p
   daly-bms-server` (API publique intacte) + `cargo clippy -p metrics-store`.
3. Sémantique : agrégation (sauf topk/bottomk) et comparaisons **droppent
   `__name__`** ; topk/bottomk **conservent** tous les labels d'origine.
4. Déploiement : changement embarqué dans le binaire ⇒ `make sync && bash
   scripts/deploy-pi5.sh` (pas de migration redb, pas de changement de config).

---

## Phase 1 — Sécurisation (rejeter ce qui est silencieusement ignoré)

**Objectif** : transformer les résultats faux silencieux en **erreurs claires**
(`bad_data`). Petit, sans risque, haute valeur. **À FAIRE EN PREMIER.**

### `validate.rs`
1. `validate_aggregate` — rejeter le modifier tant que Phase 2 n'est pas faite :
```rust
fn validate_aggregate(a: &AggregateExpr) -> Result<(), PromQlError> {
    let op_str = a.op.to_string();
    if !SUPPORTED_AGGREGATORS.contains(&op_str.as_str()) {
        return unsupported(&format!("aggregator: {op_str}"));
    }
    if a.param.is_some() {
        return unsupported(&format!("parameterized aggregator: {op_str}"));
    }
    if a.modifier.is_some() {                      // Phase 1
        return unsupported("aggregation grouping (by/without) — non encore supporté");
    }
    validate(&a.expr)
}
```
2. `validate_binary` — rejeter le matching de vecteurs non trivial :
```rust
use promql_parser::parser::VectorMatchCardinality;
// ... dans validate_binary, après le check return_bool existant :
if let Some(m) = &b.modifier {
    if m.return_bool { return unsupported("bool modifier"); }
    if m.matching.is_some() || !matches!(m.card, VectorMatchCardinality::OneToOne) {
        return unsupported("vector matching (on/ignoring/group_left/group_right) — non supporté");
    }
}
```

### Tests (`validate.rs` #[cfg(test)])
```rust
ko("sum by (bms_id)(bms_voltage)", "grouping");
ko("sum without (x)(m)",           "grouping");
ko("a / on(x) b",                  "vector matching");
ko("a * on(x) group_left b",       "vector matching");
```
Golden inchangé (nos dashboards n'utilisent pas ces formes).

**Effort** ~1–2 h. **PR** : `feat(promql): rejette by/without et vector matching (anti-résultats faux)`.

---

## Phase 2 — Groupement `by` / `without`

**Objectif** : **le plus gros gain** de compatibilité Grafana réelle.

### Sémantique
- `op by (l1,…)(vec)` → grouper par les **valeurs** de `l1,…` ; 1 sample/groupe ;
  labels de sortie = `{l1:…,…}` (drop le reste + `__name__`).
- `op without (l1,…)(vec)` → grouper par **tous les labels sauf** `l1,…` **et**
  `__name__` ; sortie conserve ces labels.
- Sans modifier → comportement actuel (1 seul groupe, labels vides).
- `avg`=sum/cnt par groupe ; `count`=cnt du groupe.

### `exec.rs::eval_aggregate` (remplacer le corps)
```rust
fn eval_aggregate(&self, a: &AggregateExpr, t: i64) -> Result<Value, PromQlError> {
    let inner = match self.eval_at(&a.expr, t)? {
        Value::Vector(v) => v,
        Value::Scalar(s) => return Ok(Value::Scalar(s)),
    };
    if inner.is_empty() { return Ok(Value::Vector(vec![])); }
    let op = a.op.to_string();

    // Noms de labels du modifier (promql_parser::label::Labels { labels: Vec<String> })
    let grp_names: Vec<String> = match &a.modifier {
        Some(m) => m.labels().labels.iter().map(|l| l.to_string()).collect(),
        None => Vec::new(),
    };
    let is_by = matches!(&a.modifier, Some(LabelModifier::Include(_)));

    // Clé de groupe (exec::Labels = BTreeMap<String,String>)
    let group_key = |labels: &Labels| -> Labels {
        let mut g = Labels::new();
        match &a.modifier {
            None => {}                                            // collapse total
            Some(LabelModifier::Include(_)) => {                  // by (...)
                for k in &grp_names {
                    if let Some(v) = labels.get(k) { g.insert(k.clone(), v.clone()); }
                }
            }
            Some(LabelModifier::Exclude(_)) => {                  // without (...)
                for (k, v) in labels.iter() {
                    if k == "__name__" || grp_names.contains(k) { continue; }
                    g.insert(k.clone(), v.clone());
                }
            }
        }
        let _ = is_by; // (gardé si besoin de distinguer plus tard)
        g
    };

    use std::collections::BTreeMap;
    struct Acc { sum: f64, min: f64, max: f64, cnt: u64 }
    let mut groups: BTreeMap<Labels, Acc> = BTreeMap::new();
    for s in &inner {
        let e = groups.entry(group_key(&s.labels))
            .or_insert(Acc{sum:0.0,min:f64::INFINITY,max:f64::NEG_INFINITY,cnt:0});
        e.sum += s.value; e.min = e.min.min(s.value); e.max = e.max.max(s.value); e.cnt += 1;
    }
    let mut out = Vec::with_capacity(groups.len());
    for (labels, acc) in groups {
        let value = match op.as_str() {
            "sum" => acc.sum, "min" => acc.min, "max" => acc.max,
            "avg" => acc.sum / acc.cnt as f64, "count" => acc.cnt as f64,
            other => return Err(PromQlError::Unsupported(format!("aggregator {other}"))),
        };
        out.push(InstantSample { labels: Arc::new(labels), value });
    }
    Ok(Value::Vector(out))
}
```

### `validate.rs`
Retirer le rejet Phase 1 du **modifier d'agrégation** (garder celui du matching
binaire — non implémenté).

### Tests (intégration, lib.rs — voir motifs existants `promql_smoke_*`)
Écrire `bms_v{bms_id="1"}=1`, `bms_v{bms_id="2"}=2`, `bms_v{bms_id="1",phase="a"}=3` :
```rust
// sum by (bms_id)        → 2 séries : {bms_id=1}=4, {bms_id=2}=2
// avg by (bms_id)        → {bms_id=1}=2, {bms_id=2}=2
// count by (bms_id)      → {bms_id=1}=2, {bms_id=2}=1
// max by (bms_id)        → {bms_id=1}=3, {bms_id=2}=2
// sum without (phase)    → {bms_id=1}=4, {bms_id=2}=2  (+ __name__ retiré)
// sum (sans modifier)    → 6 (inchangé)
// vérifier l'ABSENCE de "__name__" dans les labels de sortie
```
Golden reste vert.

**Effort** ~½ j. **PR** : `feat(promql): groupement by/without des agrégations`.

---

## Phase 3 — Extensions à la demande (3 sous-lots indépendants, 1 PR chacun)

À implémenter seulement si un dashboard en a besoin. Indépendants entre eux.

### 3a — Comparaisons `== != > < >= <=` (+ `bool`)
**Sémantique** :
- `vec OP scalar` **sans `bool`** → **filtre** : ne garder que les samples où la
  condition est vraie, **valeur inchangée**.
- avec `bool` → garder tous les samples, valeur = `1.0`/`0.0`.
- `scalar OP scalar` → PromQL **exige `bool`** (sinon le parser refuse).
- `vec OP vec` aligné → filtre/bool par paire alignée (étendre `align_and_op`).

**Changements** :
- `validate.rs` : nouveau set `SUPPORTED_CMP_OPS = ["==","!=",">","<",">=","<="]`,
  accepté dans `validate_binary` ; ne plus rejeter `bool` quand l'op est une
  comparaison ; `bool` reste rejeté sur les binaires arithmétiques.
- `exec.rs::eval_binary` : si l'op est une comparaison, router vers une logique
  dédiée (filtre vs bool) au lieu de `scalar_fn`. Gérer les 4 combinaisons
  (scalar/scalar requiert bool, vec/scalar, scalar/vec, vec/vec). Drop `__name__`.
- Attention NaN (toute comparaison avec NaN est fausse).

**Tests** : `bms_v > 1.5` (filtre), `bms_v > bool 1.5` (0/1 — ⚠️ le mot-clé
`bool` se place **après l'opérateur**, pas après le rhs), `bms_v == 2`,
vec/vec.

**Effort** ~½–1 j. **PR** : `feat(promql): opérateurs de comparaison + bool`.

### 3b — `topk` / `bottomk`
**Sémantique** : `topk(k, vec)` = les `k` samples de plus grande valeur,
**labels d'origine CONSERVÉS** (y compris `__name__`). `param` = `k` (scalaire,
évalué via `eval_at`). Optionnel : `topk(k, vec) by (l)` = top-k **par groupe**.

**Changements** :
- `validate.rs::validate_aggregate` : autoriser `param.is_some()` **uniquement**
  pour `topk`/`bottomk` ; les ajouter à un set autorisé.
- `exec.rs::eval_aggregate` : cas spécial avant le regroupement standard :
  évaluer `param` → `k` (`as usize`), trier les samples desc (topk) / asc
  (bottomk), tronquer à `k` (par groupe si modifier). **Ne pas dropper** les
  labels.

**Tests** : `topk(1, bms_v)` → 1 sample (max, labels conservés) ;
`bottomk(2, bms_v)`.

**Effort** ~½ j. **PR** : `feat(promql): topk/bottomk`.

### 3c — `irate`
**Sémantique** : taux instantané sur les **deux derniers points** de la fenêtre :
`(v_last − v_prev) / ((t_last − t_prev)/1000)`, avec gestion de reset
(`counter_increase(v_prev, v_last)` existant). Diffère de `rate` (toute la
fenêtre).

**Changements** :
- `validate.rs` : ajouter `irate` à `SUPPORTED_RANGE_FUNCS`.
- `reader.rs` : helper `last_two_in_range_with_tx(rtx, sid, from, to) ->
  Result<Option<((i64,f64),(i64,f64))>>` (scan inverse, prend les 2 derniers ;
  garde `from > to ⇒ None` comme les autres helpers).
- `exec.rs::eval_range_call` : router `irate` (tier raw) vers ce helper ; appli :
  `counter_increase(prev.1, last.1) / ((last.0 - prev.0) as f64 / 1000.0)`
  (None si < 2 points ou Δt=0).
- Tier compacté : `irate` mal défini → utiliser les 2 derniers buckets
  (`prev.last → last.last`) **ou** rejeter au-delà du tier raw (documenter le
  choix retenu).

**Tests** : série croissante → irate = pente locale ; reset géré ; 1 seul
point → pas de valeur.

**Effort** ~½ j. **PR** : `feat(promql): irate`.

---

## Phase 4 — Fonctions math & manipulation de labels

**Objectif** : préparer les dashboards plus sophistiqués à venir (légendes
dynamiques, échelles log, normalisation).

### 4a — Math instant
- `validate.rs` : ajouter `sqrt exp ln log2 log10 sgn clamp` à
  `SUPPORTED_INSTANT_FUNCS`.
- `exec.rs` : helper `unary_math` (propagation NaN naturelle ; `sgn` renvoie
  -1/0/1/NaN à la Prometheus) + `clamp_val` (NaN si `min > max`). Branché dans
  `apply_instant_fn` (vecteur) et `apply_instant_scalar` (scalaire).

### 4b — Labels
- `validate.rs` : `SUPPORTED_LABEL_FUNCS = [label_replace, label_join]` ;
  `validate_call` valide le 1er arg (vecteur) et **autorise les `StringLiteral`**
  pour les arguments suivants (seul endroit où une string est acceptée). NB :
  `promql-parser` type-check déjà la signature → un mauvais type donne une
  `ParseError`.
- `exec.rs` : routage dédié dans `eval_call` (les args string ne sont pas
  évaluables). `label_replace` utilise la crate `regex` (ancrage `^(?:…)$`,
  expansion `$1`/`${name}` via `Captures::expand`) ; valeur vide ⇒ label retiré.
  `label_join` concatène les labels source avec le séparateur.
- Dépendance : `regex = "1"` (déjà présente transitivement via promql-parser).

**Tests** : `sqrt/clamp/sgn`, `label_replace` (match, non-match, label retiré),
`label_join`. **PR** : `feat(promql): math functions + label_replace/label_join`.

---

## Récapitulatif

| Phase | Contenu | Effort | Risque | Priorité | État |
|-------|---------|--------|--------|----------|------|
| 1 | Rejet explicite by/without + vector matching | 1–2 h | très faible | **haute** | ✅ fait |
| 2 | Groupement by/without | ½ j | faible | **haute** | ✅ fait |
| 3a | Comparaisons + bool | ½–1 j | moyen | moyenne | ✅ fait |
| 3b | topk/bottomk | ½ j | faible | moyenne | ✅ fait |
| 3c | irate | ½ j | faible | basse | ✅ fait |
| 4a | Math instant (sqrt/exp/ln/log/sgn/clamp) | ¼ j | faible | moyenne | ✅ fait |
| 4b | label_replace / label_join | ½ j | faible | **haute** | ✅ fait |

**Ordre recommandé** : 1 → 2 → (3a/3b/3c à la demande) → 4 (avant dashboards sophistiqués).

### Définition de « terminé » par PR
- [ ] `cargo test -p metrics-store` vert (unitaires + golden + coverage 16 dashboards).
- [ ] `cargo build -p daly-bms-server` OK (API publique intacte).
- [ ] `cargo clippy -p metrics-store` sans nouvelle alerte.
- [ ] Matrice de compat mise à jour (ce doc + doc utilisateur).
- [ ] Aucune régression sur les dashboards (golden).

### Notes de prudence (cruciales en conversation neuve)
- **Deux types `Labels`** : `exec::Labels = BTreeMap<String,String>` (labels d'un
  sample) vs `promql_parser::label::Labels { labels: Vec<String> }` (noms du
  modifier). Accès aux noms : `modifier.labels().labels`.
- Agrégations (sauf topk/bottomk) et comparaisons **droppent `__name__`** ;
  topk/bottomk **conservent** tous les labels.
- `BTreeMap` comme clé de groupe ⇒ sortie déterministe (tests stables).
- Ne pas casser l'optim P2 (instant selector `last_point_in_range`,
  increase/rate `raw_counter_increase`/`buckets_counter_increase` — déjà en place).
- Le test `provisioned_grafana_dashboards_coverage` est le garde-fou anti-régression.
- Réutiliser `counter_increase` (gestion reset) pour `irate`.
