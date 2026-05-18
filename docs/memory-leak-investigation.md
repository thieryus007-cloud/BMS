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
| `metrics-store::writer::series_cache` HashMap | ❌ inactive | Aucun `Sample::new()` dans daly-bms-server → writer parqué sur rx (à reverifier DB active) |
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

1. **`metrics-store::writer::series_cache: HashMap<(String, String), u32>`** :
   non-évicté. Si la cardinalité des séries augmente (labels variables ?), la
   HashMap grossit linéairement. À mesurer.

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

Désactiver temporairement les modules un par un et mesurer :
```toml
[metrics_store]
enabled = true
maintenance_interval_hours = 0  # désactive la compaction tiered
```
Puis remesurer 1h. Si la fuite disparaît → coupable identifié dans
`tiering::spawn_maintenance` ou `writer::run`.

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
- `crates/metrics-store/src/writer.rs` — boucle writer, series_cache HashMap
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
- **Bisection par désactivation** : `maintenance_interval_hours = 0`,
  `alerts.db_path = ""`, `tasmota.devices = []` etc. un par un
- **heaptrack `-p <PID>` attach direct** (pas via systemd wrapper) sur le
  binaire compilé avec `--profile release-debug` (debug=1 suffit pour
  backtraces line-info)
