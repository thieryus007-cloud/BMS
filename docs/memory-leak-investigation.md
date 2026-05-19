# Fuite mémoire daly-bms-server — Contexte pour suite d'investigation

> **Document à charger en premier dans toute nouvelle session sur ce sujet.**
> État du repo : commit `37fdc57` sur `main` (cleanup mergé).
> Branche actuelle de travail (si nouvelle session) : créer depuis `main`.

---

## 1. Contexte

`daly-bms-server` (Pi5, systemd, port 8080) présente une **croissance RSS
linéaire passive** : sans activité externe (pas de curl, pas de dashboard
ouvert), le RSS grimpe régulièrement.

| Phase | RSS baseline | Rate (passif) | Δ /jour |
|-------|--------------|---------------|---------|
| Avant tout fix (référence issue.md) | ~95 → 108 MB | +13 MB/h | +312 MB |
| Après broadcast guards (commits 6ba4171+921fd72) | ~23 MB | ~7 MB/h | +168 MB |

**RssAnon grossit en parallèle de RSS** → c'est de la mémoire heap Rust
retenue (live allocations), **pas de la fragmentation**. jemalloc est déjà
tuned (`dirty_decay_ms:1000,muzzy_decay_ms:0,background_thread:true`).

⚠️ **Point critique découvert tardivement** : toutes les mesures
post-broadcast-guards ont été prises avec `[metrics_store].enabled = false`
dans `/etc/daly-bms/config.toml` — donc **sans la base redb active**. Les
mesures à reprendre **avec la base active** (1.2 GB redb sur NVMe) peuvent
donner un comportement très différent. C'est l'objet de la prochaine session.

---

## 2. Architecture (rappel)

```
Pi5 (192.168.1.141, user: pi5compute)
  mosquitto-broker        (systemd, :1883)
  daly-bms-server         (systemd, :8080) ← cible des mesures
    ├── RS485 /dev/ttyUSB0 → 2 BMS + 3 ET112 + 1 PRALRAN
    ├── MQTT sub/pub → 127.0.0.1:1883
    ├── REST + WebSocket
    └── metrics-store (redb à /mnt/nvme/daly-bms/metrics.redb, 1.2 GB)
  energy-manager          (systemd, :8081)
NanoPi (192.168.1.120)
  dbus-mqtt-venus         (runit) — MQTT ↔ D-Bus Victron
```

Stack daly-bms-server : Rust 1.88, tokio multi-thread, axum 0.7, redb 4.1,
rumqttc, askama. Allocator = `tikv-jemallocator = "0.6"` (sans feature
profiling — testé, ne se propage pas via target-conditional dep).

Env systemd actifs (cf. `contrib/daly-bms.service`) :
```
MALLOC_ARENA_MAX=2
_RJEM_MALLOC_CONF=dirty_decay_ms:1000,muzzy_decay_ms:0,background_thread:true
RUST_LOG=info
```

---

## 3. Fixes déjà appliqués et conservés

### Commit `6ba4171` + `921fd72` — Broadcast guards (effet réel mesuré)

Les `tokio::sync::broadcast` ring buffers (cap 128 pour `ws_tx`, cap 512 pour
`console_bus`, cap 64 pour `bus.live`) retenaient en permanence des
`Arc<ConsoleEvent>` / `Arc<Vec<BmsSnapshot>>` / `LiveEvent` même quand aucun
WebSocket n'était connecté — chaque slot du ring tenait un payload JSON
volumineux jusqu'à être écrasé par le send suivant.

Guard ajouté avant chaque `emit()` :
```rust
if self.console_bus.receiver_count() > 0 {
    self.console_bus.emit(ConsoleEvent::...);  // JSON alloué seulement si abonné
}
```

Fichiers patchés (17 sites au total) :
- `crates/daly-bms-server/src/state.rs` : `on_snapshot`, `on_et112_snapshot`,
  `on_irradiance_snapshot`, `on_tasmota_snapshot`, `on_venus_smartshunt` (×2),
  `on_ats_snapshot`, `on_shelly_snapshot` + guard sur `ws_tx.send`
- `crates/daly-bms-server/src/console.rs` : ajout `ConsoleBus::receiver_count()`
- `crates/daly-bms-server/src/bridges/mqtt.rs` : 5 emit (mqtt_in, mqtt_out, etc.)
- `crates/energy-manager/src/bus.rs` : early return dans `emit_live`

⚠️ **Limite identifiée** : le guard sur `ws_tx.send()` dans `state.rs:on_snapshot`
ne skip jamais en pratique parce que `bridges/alerts.rs:run_alert_engine`
subscribe en permanence à `ws_tx` via `state.subscribe_ws()`. Le guard reste
utile pour éviter l'appel à `latest_snapshots().await` (lock + clone Vec).

---

## 4. Suspects écartés par audit code statique

| Suspect | Verdict | Preuve |
|---------|---------|--------|
| `metrics-store::writer::series_cache` LruCache | ❌ inactive | Aucun `Sample::new()` dans daly-bms-server → writer parqué sur rx (à reverifier DB active). Borné à 50 000 entrées (`SERIES_CACHE_CAPACITY`) → pas de croissance illimitée même actif |
| `BmsRingBuffer` / `Et112RingBuffer` / `TasmotaRingBuffer` | ❌ bornés | `VecDeque` avec capacité config, `pop_front()` quand plein |
| `LogBuffer` (tracing) | ❌ borné | Cap 200, `pop_front()` dans `main.rs:84` |
| `Rs485DeviceStats` | ❌ borné | `BTreeMap<u8, ...>` ≤ nb_devices (~6) |
| Tasmota/Shelly MQTT loops | ❌ bornés | `HashMap<id, ...>` keyés par config |
| BMS `poll_loop::fw_cache` | ❌ borné | `HashMap<u8, ...>` ≤ nb_devices |
| `monitor_agent` (toutes les 30s) | ❌ remplacé | `state.on_monitor_snapshot()` overwrite |
| `AlertEngine::states` HashMap | ❌ borné | Keyé `(u8, &'static str)` → cardinalité fixe |
| Connection pool reqwest | ❌ inactif | `Client::new()` seulement dans `send_telegram` (rare) |
| Spawn orphelins tokio | ❌ | Toutes les `tokio::spawn` ont des futures qui complètent |

---

## 5. Suspects à investiguer **avec metrics_store.enabled = true**

C'est la prochaine étape critique. Les soupçons ci-dessous étaient inactifs
pendant nos mesures puisque DB désactivée.

1. **`metrics-store::writer::series_cache: LruCache<(String, String), u32>`** :
   borné à **50 000 entrées** (constante `SERIES_CACHE_CAPACITY` dans
   `crates/metrics-store/src/writer.rs`). L'éviction LRU est automatique
   donc **pas de croissance illimitée**. Hypothèse révisée : à surveiller
   uniquement si la cardinalité atteint le plafond (logs `evict` ou taux
   de miss élevé) → là on saurait que c'est saturé et qu'il faut soit
   l'agrandir soit chercher pourquoi tant de séries uniques apparaissent.

2. **`metrics-store::tiering::spawn_maintenance`** : compaction périodique
   (`maintenance_interval_hours = 6h` par défaut). Pendant la compaction,
   `HashMap<(u32, i64), AggBucketBuilder>` peut être très large avant le
   `finalize()`. Si l'allocation peak n'est pas rendue → pic de RSS toutes
   les 6h.

3. **redb cache pages** : `cache_mb = 64` par défaut. La cache memory-mapped
   peut grimper progressivement à mesure que les pages chaudes changent.
   À mesurer via `/proc/<pid>/status` → `RssFile` (les mmap apparaissent
   dans RssFile, pas RssAnon — donc si la fuite est RssFile c'est ça).

4. **Writer thread channel** (`mpsc::channel::<Sample>` de profondeur
   `queue_depth = 10000`) : si les writes redb sont plus lents que la
   production de samples, le canal grossit jusqu'à 10000 Samples.

5. **`Sample` allocation rate** : qui pousse des Samples au writer ?
   À vérifier — `grep -rn "Sample::new" crates/` a montré 0 hits, mais
   peut-être via une couche d'abstraction. Si la DB est active mais
   personne ne pushe, le writer dort → pas de leak.

---

## 6. Tentatives diagnostiques qui ont échoué (à NE PAS refaire)

| Outil | Pourquoi ça a échoué |
|-------|----------------------|
| **dhat-rs** | Build avec `--features dhat-heap` + `debuginfo=2` + dhat instrumentation = OOM au link sur Pi5 8 GB (build interrompu après 25 min) |
| **jemalloc profiling** | Feature `tikv-jemallocator/profiling` propagée via `[target.'cfg(...)'.dependencies]` ne se propage pas correctement à la build C de jemalloc. Vérifié par `nm` : 0 symbole `_rjem_prof_*` dans le binaire après build avec `--features jeprof`. Le `_RJEM_MALLOC_CONF=prof:true` est silencieusement ignoré, aucun fichier `.heap` jamais produit |
| **heaptrack via systemd wrapper** | L'unit a `Type=notify` + `WatchdogSec=60`. Quand `heaptrack` wrap `ExecStart`, le main PID devient heaptrack (qui ne sd_notify pas) → systemd timeout au démarrage et kill loop. Aurait nécessité d'override `Type=simple` + `WatchdogSec=0` dans le drop-in, jamais validé end-to-end |

---

## 7. Diagnostic recommandé pour la prochaine session

### Étape 1 — Activer la DB et mesurer 1h

```bash
# Sur Pi5
sudo sed -i '/^\[metrics_store\]/,/^\[/{s/^enabled = false$/enabled = true/}' /etc/daly-bms/config.toml
sudo systemctl restart daly-bms

# Tracker RSS toutes les 5 min, séparant RssAnon (heap) et RssFile (mmap)
PID=$(pgrep -f 'daly-bms-server$' | head -1)
while sleep 300; do
  echo "$(date -Iseconds) $(awk '/^VmRSS|^RssAnon|^RssFile/ {printf "%s=%sk ", $1, $2}' /proc/$PID/status)"
done | tee /tmp/rss-db-active.log
```

Laisser 1-2h. Identifier si la croissance vient de **RssAnon** (heap Rust =
allocation retenue) ou **RssFile** (mmap redb = cache pages, normal jusqu'à
`cache_mb`).

### Étape 2 — Stats jemalloc via API live (pas de rebuild)

Le binaire actuel a déjà jemalloc linké. On peut lire les stats via gdb
attach (lecture seule, pas d'allocator switch) :

```bash
PID=$(pgrep -f 'daly-bms-server$' | head -1)
# Première mesure T+0
sudo gdb -p $PID -batch \
  -ex 'set confirm off' \
  -ex 'call (void)_rjem_malloc_stats_print(0,0,"Jmdablxe")' \
  -ex 'detach' 2>&1 | tee /tmp/jestats-t0.json

# Attendre 30 min, refaire :
sudo gdb -p $PID -batch \
  -ex 'set confirm off' \
  -ex 'call (void)_rjem_malloc_stats_print(0,0,"Jmdablxe")' \
  -ex 'detach' 2>&1 | tee /tmp/jestats-t30.json

# Diff
diff <(jq -S . /tmp/jestats-t0.json) <(jq -S . /tmp/jestats-t30.json)
```

Le delta `stats.allocated` entre les deux = ce que le code Rust détient
effectivement en plus. Décomposé par size class → pointe vers le type
d'allocation qui fuit (petits objets = HashMap entries, larges =
Vec/String/payloads).

### Étape 3 — Si confirmation que c'est metrics-store

⚠️ **Attention** : `maintenance_interval_hours = 0` **ne désactive PAS** la
compaction — le code fait `interval_hours.max(1)` dans
`tiering::spawn_maintenance` (`crates/metrics-store/src/tiering.rs:265`).
Une valeur de 0 force à 1h, pas à "off". Pour vraiment désactiver la
compaction tiered, **ne pas appeler `spawn_maintenance` du tout** : il faut
patcher `main.rs` pour skip le `spawn_maintenance` quand un flag config est
positionné, ou simplement le commenter sur une branche de test.

Si on veut juste **diminuer la fréquence** pour observer (ex: pic toutes
les 24h au lieu de toutes les 6h) :
```toml
[metrics_store]
enabled = true
maintenance_interval_hours = 24  # 1 passe de compaction par jour
```

Pour le **vrai test isolant `tiering` comme suspect**, patch temporaire sur
branche de test dans `crates/daly-bms-server/src/main.rs` :
```rust
// if config.metrics_store.maintenance_interval_hours > 0 {
//     let _ = store.spawn_maintenance(policy, config.metrics_store.maintenance_interval_hours);
// }
```
Rebuild + redéploiement → 1h de mesure. Si la fuite disparaît → c'est
`tiering`. Sinon → c'est `writer::run` ou la cache redb mmap.

---

## 8. Process de déploiement standard (rappel)

```bash
# Sur Pi5
cd ~/Daly-BMS-Rust
make sync
bash scripts/deploy-pi5.sh
```

Le script `deploy-pi5.sh` :
1. `make sync`
2. `make build-arm` + `make build-energy-arm`
3. Copie binaires + restart services
4. Auto-répare `[metrics_store].enabled = true` si nécessaire
5. Tests API via `test-api.sh`

Variantes : `--no-build`, `--no-validate`.

---

## 9. Fichiers à lire pour aller plus loin

- `docs/issue.md` — analyse initiale (taux +13 Mo/h avant fixes, avant DB
  désactivée) — note : les hypothèses listées datent d'avant que DB soit off
- `crates/metrics-store/src/writer.rs` — boucle writer, `series_cache` LruCache (cap 50k)
- `crates/metrics-store/src/tiering.rs` — `spawn_maintenance`, compactions
- `crates/daly-bms-server/src/state.rs:386-498` — `dispatched_query_*`,
  `redb_query_*_inner` — chemins de lecture redb depuis l'API
- `crates/daly-bms-server/src/bridges/alerts.rs:529` — `run_alert_engine`
  subscribe en permanence à `ws_tx` (raison pour laquelle le guard
  `ws_tx.receiver_count()` ne skip jamais)

---

## 10. Règle pour la prochaine session

**Ne pas ajouter d'outillage de diagnostic dans le repo sans avoir validé
end-to-end qu'il fonctionne sur Pi5 ARM.** Les outils suivants ont déjà
été essayés et ne marchent pas tels quels :
- dhat-rs (OOM)
- jemalloc profiling via cargo feature (ne propage pas)
- heaptrack via systemd wrapper (Type=notify conflict)

Approches qui devraient marcher mais n'ont pas été terminées :
- **gdb + `_rjem_malloc_stats_print`** sur binaire prod existant (cf. §7,
  pas de rebuild, juste un attach lecture-seule)
- **Bisection par désactivation** : patch `main.rs` pour skip
  `store.spawn_maintenance()`, vider `tasmota.devices = []`,
  vider `alerts.db_path = ""` etc. un par un (rappel : pas de paramètre
  numérique qui désactive complètement `maintenance_interval_hours` —
  cf. §7.3)
- **heaptrack `-p <PID>` attach direct** (pas via systemd wrapper) sur le
  binaire compilé avec `--profile release-debug` (debug=1 suffit pour
  backtraces line-info)

---

## 11. Procédure : isoler `/dashboard/history` comme suspect

Le dashboard custom `/dashboard/history` (cf. `crates/daly-bms-server/src/dashboard/mod.rs:823`)
sert aussi de **datasource type Prometheus** pour Grafana via les endpoints
`/api/v1/query` et `/api/v1/query_range`. Si Grafana scrape en background,
ou si un onglet navigateur reste ouvert quelque part (PC, mobile, tablette),
il y a une **activité HTTP continue** invisible mais qui peut alimenter la
fuite. À vérifier **AVANT** d'imputer la fuite à un "mode passif".

### ce qui a été verifié: la navigation dans la page historique et le changement de l interval provoque une hausse significative de la RAM et cela reste persistant

### 11.1 — Vérifier si quelqu'un tape sur l'API pendant la mesure

```bash
# 1 min d'observation — devrait être 100% silencieux si "mode passif" réel
sudo journalctl -u daly-bms -f --since now &
PID_TAIL=$!
sleep 60
kill $PID_TAIL 2>/dev/null
```

Toute ligne `GET /api/v1/query*`, `GET /api/v1/chart/history`,
`GET /api/v1/history/energy`, `GET /dashboard/history` ou `/ws/bms/stream`
pendant ces 60 s = **scraper actif** → la mesure de fuite n'est pas en mode
passif.

### 11.2 — Identifier le scraper

```bash
# Connexions TCP entrantes sur :8080 (qui se connecte ?)
PID=$(pgrep -f 'daly-bms-server$' | head -1)
sudo ss -tnp -o state established | grep "pid=$PID" | awk '{print $4, $5}'

# Si Grafana tourne sur le même Pi5 :
systemctl is-active grafana-server 2>/dev/null
```

Suspects courants :
- `grafana-server.service` qui scrape `/api/v1/query_range` toutes les
  ~15-30s (intervalle de refresh des panels Grafana)
- Onglet Firefox/Chrome resté ouvert sur `192.168.1.141:8080/dashboard/*`
  avec auto-refresh activé (vérifier mobile + PC + tablette)
- Script de monitoring externe (curl en cron, healthcheck Home Assistant, etc.)

### 11.3 — Bloquer le trafic externe le temps de la mesure

Le moyen le moins invasif : firewall temporaire qui bloque toute connexion
entrante :8080 sauf localhost (les checks systemd internes restent OK) :

```bash
# Bloquer
sudo iptables -I INPUT -p tcp --dport 8080 ! -s 127.0.0.1 -j DROP

# Vérifier
sudo iptables -L INPUT -n | grep 8080

# Remesurer 1h avec rss-tracker.sh

# Retirer la règle après la mesure
sudo iptables -D INPUT -p tcp --dport 8080 ! -s 127.0.0.1 -j DROP
```

Alternative ciblée : stopper Grafana uniquement :
```bash
sudo systemctl stop grafana-server
# ... mesure ...
sudo systemctl start grafana-server
```

Si la fuite **disparaît ou diminue significativement** sous firewall →
c'est bien le trafic HTTP qui alimente la fuite. Si elle persiste à
l'identique → c'est un process interne périodique, pas le dashboard.

### 11.4 — Désactivation au niveau code (si trafic externe insuffisant comme test)

Si le trafic externe est bloqué (11.3) mais la fuite persiste, on peut
désactiver purement les routes `/dashboard/history` et endpoints PromQL au
niveau code pour exclure tout chemin d'exécution lié à l'historique. Patch
minimal sur **branche de test uniquement** (à revert avant prod) :

```rust
// crates/daly-bms-server/src/dashboard/mod.rs:823
// .route("/dashboard/history",           get(dashboard_history))

// crates/daly-bms-server/src/api/mod.rs (lignes à identifier via grep) :
// .route("/api/v1/query",         get(promql::query))
// .route("/api/v1/query_range",   get(promql::query_range))
// .route("/api/v1/chart/history", get(chart::get_chart_history))
// .route("/api/v1/history/energy", get(history::get_energy_history))
```

Puis `make build-arm` + redéploiement. Si la fuite résiduelle disparaît
**alors qu'aucun client externe ne sollicite ces routes** (firewall actif
de 11.3 conservé), la cause est dans le code de la dashboard/history
elle-même (probable : allocation de Vec<Sample>, Strings, JSON encoder
réutilisé sans reset entre requêtes).

**Important** : ne PAS committer ces commentaires sur main.

### 11.5 — Vérifier les WebSocket persistants

Les WS (`/ws/bms/stream`, `/ws/venus/stream`, `/ws/console`) peuvent rester
ouverts indéfiniment côté serveur si un client est mal déconnecté (firewall
qui drop sans RST TCP). Chaque WS connexion = un `tokio::spawn` qui détient
un `broadcast::Receiver` → rétention dans le ring buffer.

```bash
# Nombre de TCP ESTABLISHED côté daly-bms-server (devrait baisser à ~1-2
# après firewall actif si tous les clients sont dehors)
PID=$(pgrep -f 'daly-bms-server$' | head -1)
sudo ss -tnp -o state established | grep -c "pid=$PID"
```

Si ce nombre grimpe au cours du temps même sans nouveau client → fuite de
connexions WS (peu probable mais à exclure).

---

## 12. Audit du chemin PromQL (session 2026-05-19)

### 12.1 — Cause confirmée du spike `/dashboard/history`

Le smoking gun est dans **`crates/metrics-store/src/promql/exec.rs`**.
Chaîne d'appel pour une requête `/api/v1/query_range` :

```
query_range  →  state.dispatched_query_range  →  spawn_blocking
   →  Evaluator::eval_range  (boucle while t <= end_ms, step_ms)
       →  eval_at(expr, t)                             ← N appels
           →  eval_vector_selector  OR  eval_range_call
               →  Evaluator::match_series (exec.rs:349-377)
                   →  Reader::list_series (reader.rs:92-102)
                       →  désérialise TOUTE la table TABLE_SERIES_META
                       →  serde_json::from_str(labels_json) pour chaque série
```

**Amplification mesurable** :

Le JS de `templates/history.html:753` calcule un pas pour viser **~200 points**
quel que soit le range :
```js
const step = Math.max(1, Math.round(Math.min(86400, Math.max(5, span / 200))));
```

Donc `eval_range` exécute **~200 itérations**. Pour chaque itération, chaque
`VectorSelector` ou `MatrixSelector` dans l'expression provoque :
- 1 ouverture de transaction redb (`begin_read`)
- 1 scan complet de `TABLE_SERIES_META`
- N `serde_json::from_str(&meta.labels_json)` (1 par série en base)
- M `Labels::clone()` (1 par série matchée)

Sur `/api/v1/history/energy` (cf. `api/history.rs:61-73`), **9 requêtes
PromQL parallèles** via `tokio::join!`. Pour une période "year" avec
window=30d, c'est 9 × 200 = **1 800 scans complets du catalogue** par
chargement de la page (+1 800 si l'utilisateur change le sélecteur de
période). Avec ~500 séries en base (BMS×2 + ET112×3 + Tasmota + Venus +
Shelly + irradiance), c'est **~900 000 `serde_json::from_str` par
navigation** dont chacun alloue un `BTreeMap<String, String>`.

Ces allocations sont **toutes éphémères** (Rust les drop à la fin de
chaque itération), mais elles font exploser la pression sur jemalloc.
Combiné avec l'observation utilisateur (« la RAM reste persistante »),
cela pointe vers de la **rétention de pages jemalloc dans les arenas
des threads `spawn_blocking`** :

- `tokio::task::spawn_blocking` réutilise un pool de threads (default 512
  max). Les arenas jemalloc sont **par-thread**.
- Les ~1 800 scans sont distribués sur N threads blocking → chacun
  conserve des dirty pages.
- `dirty_decay_ms:1000` libère sous 1s **mais seulement si le thread est
  idle** — pendant un burst de 9 queries parallèles, le decay ne se
  déclenche que **après** la fin du burst. Le pic reste matérialisé.

### 12.2 — Détails du verrou

- `exec.rs:349-377` (`match_series`) : pas de cache. Recalcule entièrement
  pour chaque step même quand `metric` + `matchers` sont identiques (ce qui
  est TOUJOURS le cas pour un même VectorSelector dans une eval_range).
- `exec.rs:100` (`by_labels: BTreeMap<Labels, Vec<(i64, f64)>>`) : accumule
  tous les points pour tous les steps. Pour 9 queries × 200 points × M
  séries, le pic mémoire scale avec le produit. Local à la fonction donc
  drop à la fin, mais contribue au pic.
- `reader.rs:92-102` (`list_series`) : ouvre une nouvelle transaction
  redb et désérialise tous les `SeriesMeta` à chaque appel. Aucune
  mémoïzation au niveau Reader. `bincode::deserialize` + clone de
  `metric: String` + `labels_json: String` × N séries.

### 12.3 — Confirmation : le writer est dormant

`grep -rn "Sample::new" crates/` → seulement des hits dans les tests
(`metrics-store/src/lib.rs`). **Aucun code de production ne pousse de
samples**. Le writer thread (`writer::run`) bloque sur `blocking_recv()`
indéfiniment. Donc :

- §5.1 (`series_cache` LruCache du writer) : **éliminé comme suspect**
  pour ce comportement, il est vide en prod.
- §5.4 (canal `mpsc::channel::<Sample>`) : **éliminé**, jamais alimenté.
- §5.5 (rate de Sample) : confirmé 0/s.

La base `metrics.redb` de 1.2 GB est donc **uniquement lue, jamais
écrite** par daly-bms-server en l'état actuel. Les seules écritures
viennent peut-être de `metrics-cli` (à vérifier) ou de runs antérieurs.
→ Question pour la prochaine session : qui a écrit ces 1.2 GB et est-ce
qu'on veut activer un writer en prod ?

### 12.4 — Suspect mineur : `MALLOC_ARENA_MAX=2` ignoré par jemalloc

Dans `contrib/daly-bms.service` :
```
MALLOC_ARENA_MAX=2
_RJEM_MALLOC_CONF=dirty_decay_ms:1000,muzzy_decay_ms:0,background_thread:true
```

`MALLOC_ARENA_MAX` est une variable **glibc-malloc-only**. jemalloc
l'ignore. L'équivalent jemalloc est `narenas:N` dans `_RJEM_MALLOC_CONF`.
Par défaut, jemalloc crée `4 × ncpus` arenas — soit 16 sur le Pi5
quadricore. Chaque arena peut retenir ses propres dirty pages. Ajouter
`narenas:2` dans `_RJEM_MALLOC_CONF` limiterait la fragmentation
inter-arenas (au prix de plus de contention de lock — acceptable pour le
profil ~4 fsync/s du writer).

### 12.5 — Fix recommandé (non encore appliqué)

**Priorité 1 — Cache `match_series` au niveau Evaluator** :

`match_series` ne dépend que de `(metric, matchers)` et du contenu de
`TABLE_SERIES_META`. Dans une `eval_range` unique, ces valeurs sont
constantes pour chaque VectorSelector. Patch minimal dans `exec.rs` :

```rust
pub struct Evaluator<'r> {
    reader: &'r Reader,
    pub lookback_ms: i64,
    // Cache scopé à l'Evaluator (drop avec l'Evaluator donc fin de query).
    // Clef = (metric, hash des matchers sérialisés). Valeur = résultat
    // partagé via Rc pour éviter le clone.
    series_cache: RefCell<HashMap<(String, u64), Rc<Vec<(u32, Labels)>>>>,
    // Catalogue de séries chargé 1 fois par Evaluator.
    series_catalog: OnceCell<Vec<(u32, SeriesMeta)>>,
}
```

Effet attendu :
- 9 queries × 200 steps × 1 VectorSelector = 1 800 → **9 appels** à
  `list_series` (réduction × 200).
- Total `serde_json::from_str` par burst : 900 000 → **4 500** (réduction
  × 200).

**Priorité 2 — Pré-charger le catalogue une fois par Evaluator** :
même logique, niveau `Reader::list_series` mémoïsé au niveau Evaluator.

**Priorité 3 (optionnelle) — Cache cross-request au niveau MetricsStore** :
`TABLE_SERIES_META` change rarement (uniquement à l'ajout d'une nouvelle
série, donc nouveau device ou nouvelle label combo). Un cache invalidé
par bump de génération côté writer permettrait de servir 100% des
queries sans toucher redb. Mais comme le writer est dormant en prod,
le catalogue ne change littéralement jamais aujourd'hui → cache éternel
trivialement valide. À implémenter quand le writer sera réactivé.

### 12.6 — Pour valider le fix sans rebuild

Avant de patcher, refaire la mesure avec firewall actif (§11.3) **pendant
qu'un curl unique tape `/api/v1/query_range`** avec un range "year"
toutes les 30s. Le delta RSS observé doit être proportionnel au nombre
de queries lancées. Une fois le patch en place, refaire le même test :
le delta doit chuter de l'ordre de × 200 sur le pic d'allocation et la
RSS résiduelle doit redescendre plus vite (moins de pression
inter-arenas).
