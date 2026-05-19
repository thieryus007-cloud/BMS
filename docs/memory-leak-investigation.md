# Memory leak `/dashboard/history` — investigation 2026-05-18/19

> **STATUT FINAL** : pas de vraie fuite mémoire. Plateau d'allocation jemalloc
> normal pour un workload concurrent en burst. Voir §6 — Conclusion.

---

## 1. Symptôme initial

`daly-bms-server` (Pi5, systemd, port 8080) montrait :
- Baseline ~30 MB RSS au démarrage frais
- Après navigation `/dashboard/history` avec sélection d'intervalle : +23 à +47 MB
- La mémoire ne redescendait pas (au-delà de 1-2 minutes) → soupçon de fuite
- Mesure passive sans action : +5-6 MB/h (faible mais visible)

## 2. Hypothèses initiales et fixes appliqués

### 2.1 — PR #481 : cache `match_series` + catalogue dans Evaluator

`crates/metrics-store/src/promql/exec.rs` : chaque step d'`eval_range`
rechargeait `TABLE_SERIES_META` entière et redéserialisait `labels_json`.
Sur `/dashboard/history` (≈200 steps × 9 queries parallèles), cela
générait ~900 000 allocs `BTreeMap<String,String>` éphémères par
chargement.

Fix : `series_catalog: RefCell<Option<Arc<Vec<SeriesMeta>>>>` chargé 1×
et `match_cache: RefCell<HashMap<...>>`. Réduction des allocs × 200 par
query.

### 2.2 — PR #482 : `Arc<Labels>`, rtx partagée, cache par ptr AST

Suite review Gemini :
- `InstantSample.labels: Arc<Labels>` (évite ~20 000 clones de BTreeMap)
- `read_txn: OnceCell<ReadTransaction>` réutilisée sur tout `eval_range`
  (évite ~20 000 ouvertures rtx + open_table)
- Cache `match_series` keyé par `vs as *const VectorSelector as usize`
  (évite le calcul de fingerprint à chaque step)

### 2.3 — PR #483 : libération `match_cache` avant `try_unwrap`

`drop(scalar_key) + match_cache.clear()` avant la conversion finale
`Arc<Labels> → Labels` pour que `Arc::try_unwrap` réussisse au lieu de
retomber sur `clone()`.

### 2.4 — `narenas:2` dans `_RJEM_MALLOC_CONF` (ROLLBACK)

Tentative pour limiter la fragmentation inter-arenas jemalloc. **Résultat
opposé : a aggravé le problème** parce que la concurrence des bursts se
concentre sur 2 arenas au lieu de se répartir, chaque arena retient plus
de pages dirty. Retiré.

## 3. Mesures terrain — décryptage du plateau

### Test 1 — Fresh restart, 1 nav 30j

| Mesure | RSS | Anon | Δ |
|--------|-----|------|---|
| Baseline (post-restart) | 30.7 MB | 21.2 MB | — |
| Après nav 30j | 63.6 MB | 53.8 MB | +32 MB |
| Après 60 s idle | 63.1 MB | 53.3 MB | +31.5 MB (non libéré sur 60 s) |

### Test 2 — Stress sequential 500 requêtes /api/v1/history/energy

| Mesure | RSS | Anon | Δ |
|--------|-----|------|---|
| T0 | 54.9 MB | 44.5 MB | — |
| Après 500 req | 59.9 MB | 49.5 MB | +5.0 MB |
| Après 90 s idle | 55.4 MB | 44.9 MB | **+0.5 MB seulement** |

→ jemalloc libère bien après idle suffisamment long.

### Test 3 — Plateau confirmé après ~10 navs

| Mesure | RSS | Anon | Δ |
|--------|-----|------|---|
| Avant 10 navs (baseline déjà chargé) | 56.7 MB | 46.2 MB | — |
| Après 10 navs rapides | 59.3 MB | 48.9 MB | **+2.7 MB (≈270 kB/nav)** |

→ chaque nav supplémentaire au-delà du plateau coûte ~100× moins que
les premières.

### Test 4 — Endpoint trivial /-/healthy

100 GET /healthy = +0.8 MB (≈8 kB/req). Confirme que **Axum,
TraceLayer, hyper et les middleware ne fuient pas**.

## 4. Compréhension du pattern

Le pattern observé est **un plateau d'allocator**, pas une fuite linéaire.

1. **1ère nav** : la rafale concurrente du JS (≈10 fetches simultanés
   vers `/api/v1/dashboards/{catalog,layout,panel/.../data}` +
   `/api/v1/query_range`) force jemalloc à étendre ses arenas pour
   absorber le burst. Coût : +23 à +47 MB.

2. **2-3èmes navs** : la concurrence peak peut être encore plus large que
   la 1ère, donc nouvelles extensions d'arenas. +20-30 MB.

3. **Plateau atteint** vers 55-110 MB selon l'historique de bursts.

4. **Navs au-delà du plateau** : réutilisent les arenas existantes →
   +1-2 MB par nav, libéré après quelques minutes d'idle.

5. **Idle long** : jemalloc rend une partie des pages au kernel
   (`dirty_decay_ms:1000` actif) mais conserve une portion comme
   working-set pour absorber les bursts futurs.

## 5. Pourquoi `narenas:2` aggravait

Avec 16 arenas (4 × ncpus = défaut sur Pi5) :
- 9 queries parallèles se répartissent sur ~9 arenas distinctes
- Chaque arena retient peu de pages dirty
- Total dirty pages = somme petite

Avec `narenas:2` :
- 9 queries parallèles se concentrent sur 2 arenas
- Chaque arena gonfle pour absorber la pression
- Avec `dirty_decay_ms:1000` actif, les pages dirty s'accumulent dans
  ces 2 arenas avant le purge
- Total dirty pages = somme plus grande

Le tuning jemalloc à conserver :
```
_RJEM_MALLOC_CONF=dirty_decay_ms:1000,muzzy_decay_ms:0,background_thread:true
```
**SANS** `narenas:2`. Le défaut 4 × ncpus est correct pour notre profil.

## 6. Conclusion

Il n'y a **pas de fuite mémoire au sens strict**. Le service plateau
entre 55 et 110 MB selon la charge concurrente la plus large rencontrée
historiquement. C'est un coût d'allocator normal, accepté pour un service
système Rust avec workload concurrent.

Les optimisations PR #481/#482/#483 restent **utiles** :
- Réduisent les allocations transitoires lors d'`eval_range`
- Diminuent la pression sur jemalloc pendant les bursts
- Si retirées, le plateau serait probablement plus haut (~150-200 MB)

**Pas d'action corrective requise**. Le comportement est conforme aux
attentes pour un service avec :
- 9 requêtes PromQL parallèles via `tokio::join!` sur `/api/v1/history/energy`
- Burst de 10-20 fetches concurrents au chargement de `/dashboard/history`
- Allocator jemalloc multi-arena par défaut

## 7. Workarounds optionnels si baseline trop élevé en prod

| Approche | Effet attendu | Coût |
|----------|---------------|------|
| `RuntimeMaxSec=86400` dans la service unit | Restart quotidien automatique, plateau réinitialisé | Mini interruption ~5 s/jour |
| Sérialiser les 9 queries de `/history/energy` (retirer `tokio::join!`) | Réduit la concurrence peak → plateau plus bas | Latence × 9 sur l'endpoint |
| Limiter le nombre de panels custom sur `/dashboard/history` | Moins de fetches concurrents | UX |
| Garder `narenas:2` désactivé | Plateau plus bas qu'avec narenas:2 | (déjà fait) |

## 8. Outils diagnostiques utilisés

- `awk '/^VmRSS|^RssAnon/' /proc/$PID/status` — mesure RSS / Anon
- `curl` burst sequential pour isoler les endpoints
- Comparaison avant / après idle long (90 s à 5 min) pour distinguer
  rétention jemalloc vs fuite vraie
- Toggle `[metrics_store].enabled = false` dans `/etc/daly-bms/config.toml`
  pour bisecter le path PromQL

Outils essayés sans succès :
- `heaptrack -p $PID` via GDB attach : crash du service à l'injection
  (signaux GDB → auto-restart systemd)
- jemalloc profiling via cargo feature : `--features jeprof` ne propage
  pas correctement au build C, `_RJEM_MALLOC_CONF=prof:true` est ignoré

## 9. Apprentissages

1. **Mesurer RssAnon (heap) ET RssFile (mmap) séparément** — la
   confusion entre les deux fausse l'analyse.
2. **Attendre 5 min après un burst** avant de conclure à une fuite —
   `dirty_decay_ms:1000` libère sur thread idle, pas instantanément.
3. **Comparer le N-ème nav au 1er** — une vraie fuite est linéaire en
   N. Un plateau d'allocator se manifeste comme `f(N) → constant`.
4. **`MALLOC_ARENA_MAX=N` est glibc-only** — ne pas le confondre avec
   `narenas:N` dans `_RJEM_MALLOC_CONF`. Pour jemalloc, le défaut
   (4 × ncpus) est généralement bon.
5. **Une optimisation Rust qui réduit les allocations transitoires
   n'apparaît pas forcément comme une baisse de baseline RSS** — elle
   apparaît comme une baisse du plateau atteint sous charge.
