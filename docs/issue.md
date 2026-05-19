OK, croissance passive +13 Mo/heure SANS activité = **vraie fuite mémoire**, pas de la fragmentation. À fonds nouveaux dans une session fraîche. Voici le prompt :

---

# 🐛 Investigation fuite mémoire daly-bms-server (post-Phase 5)

## Contexte projet

Projet **Daly-BMS-Rust** (repo `thieryus007-cloud/Daly-BMS-Rust`, branche `claude/migration-vm-redb-kqUG8`). Serveur Rust qui :
- Polle 2 BMS Daly + 3 compteurs ET112 + 1 capteur PRALRAN via RS485
- Reçoit MQTT Venus/Tasmota/Shelly
- Expose API REST/WebSocket sur :8080
- Sert un dashboard custom HTML (`/dashboard/history`) ET fait office de datasource Prometheus pour Grafana
- Stocke les séries temporelles dans une base **redb** locale (crate maison `metrics-store`)
- Tourne en service systemd sur Raspberry Pi5 (aarch64, 8 Go RAM)

VictoriaMetrics a été retiré récemment (Phase 5 cleanup) — `redb` est désormais la seule TSDB.

## Stack pertinente
- Rust 1.88, tokio multi-thread runtime
- **jemalloc** via `tikv-jemallocator = "0.6"` avec config agressive : `MALLOC_ARENA_MAX=2` + `_RJEM_MALLOC_CONF=dirty_decay_ms:1000,muzzy_decay_ms:0,background_thread:true`
- redb 4.1 (TSDB embarquée, cache configurable via `[metrics_store].cache_mb`)
- axum 0.7 (HTTP), reqwest, rumqttc, askama (templates)

## Problème EXACT à résoudre

Le RSS de `daly-bms-server` **croît passivement, sans aucune activité externe** :

```
À T+0   (boot)              : RSS =  95 Mo  RssAnon =  85 Mo  VmPeak = 423 Mo
Après 5 bursts curl + 30 s  : RSS =  95 Mo  RssAnon =  85 Mo  VmPeak = 423 Mo (jemalloc OK)
Après 1 h SANS activité     : RSS = 108 Mo  RssAnon =  98 Mo  VmPeak = 438 Mo
```

**Rate : ~13 Mo/heure sans toucher au service**. Sur 24 h = +312 Mo. C'est inacceptable pour un Pi5 de prod sur 5 ans.

Le RSS *post-burst* est stable (jemalloc rend bien les pages → c'est PAS de la fragmentation). Mais quelque chose alloue **régulièrement et de façon retenue** en arrière-plan.

## Ce qui a déjà été investigué (à ne pas refaire)

| Hypothèse | Verdict |
|---|---|
| Pool tokio blocking explosif | ❌ Threads = 8, stable |
| Cascade VM writes (avant cleanup) | ✅ corrigé (VM retiré complètement) |
| Fragmentation glibc | ✅ corrigé (jemalloc tuned) |
| Cache redb mmap | ❌ RssFile = 10 Mo, pas le coupable |
| Broadcast channels saturés | ❌ ring buffers fixes (ws_tx=128, console=512) |
| Spawn orphelins tokio | ❌ tokio nettoie automatiquement |
| `cache_mb` trop haut | À VOIR — vérifier la valeur dans `/etc/daly-bms/config.toml` |

Variables d'env actuellement actives (confirmées par `systemctl show daly-bms -p Environment`) :
```
MALLOC_ARENA_MAX=2
_RJEM_MALLOC_CONF=dirty_decay_ms:1000,muzzy_decay_ms:0,background_thread:true
RUST_LOG=info
```

## Suspects sérieux à investiguer

Activités périodiques qui pourraient retenir de la mémoire :

1. **Maintenance metrics-store** (`tiering::spawn_maintenance`) — tourne toutes les `maintenance_interval_hours = 6` heures par défaut. Le `HashMap<(u32, i64), AggBucketBuilder>` peut grossir énormément sur une base 1.2 Go avant d'être libéré. Vérifier si la fonction tient des références après finalize.

2. **Writer thread metrics-store** — `writer::run` boucle infinie qui maintient `series_cache: HashMap<(String, String), u32>` qui grossit avec le temps (jamais évicté).

3. **Ring buffers du AppState** — `BmsRingBuffer`, `Et112RingBuffer`, `TasmotaRingBuffer` (`VecDeque` bornés mais initialisés à 0, et **grossissent** jusqu'à `ring_buffer_size` puis stable). Vérifier qu'ils sont bien bornés et qu'ils libèrent en interne.

4. **LogBuffer** — collecté en RAM pour servir `/api/v1/system/logs`. Si non borné → fuite linéaire.

5. **Console_bus / ws_tx broadcast** — les Arc dans le ring buffer SONT droppés quand écrasés, mais si le buffer n'est jamais drainé (aucun subscriber), il reste à 128/512 Arc volumineux en RAM.

6. **AlertEngine** — daemon SQLite qui pourrait accumuler des connexions ou des hash internes.

7. **MQTT bridges** (`bridges::mqtt`, `bridges::tasmota::mqtt`, `bridges::shelly::mqtt`) — chacun a une reconnect loop qui pourrait fuiter.

8. **tracing-appender / tracing-subscriber** — buffers de logs internes.

## Stratégie de diagnostic suggérée

**Étape 1 — Profiling jemalloc en runtime**

Activer les stats internes jemalloc et les dumper périodiquement :

```bash
# Sur Pi5 (gdb attaché à daly-bms-server)
sudo gdb -p $PID -batch -ex 'call (int)_rjem_malloc_stats_print(0,0,"")'
```

Comparer `stats.allocated`, `stats.active`, `stats.mapped` à 2 instants espacés de 1 h. Le delta de `stats.allocated` = ce que le code Rust détient effectivement.

**Étape 2 — Heap profiling avec dhat**

Compiler une variante de daly-bms-server avec `dhat-rs` activé :
```toml
[features]
dhat-heap = ["dhat"]

[dependencies]
dhat = { version = "0.3", optional = true }
```

```rust
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();
    // ...
}
```

Laisser tourner 30 min, dumper le rapport JSON, analyser avec `dhat-viewer` ou `dh_view.html` pour identifier la backtrace allocante.

**Étape 3 — Check des structures suspectes**

Auditer le code de :
- `crates/metrics-store/src/writer.rs` — la `series_cache: HashMap` est-elle bornée ?
- `crates/metrics-store/src/tiering.rs` — `spawn_maintenance` libère-t-il tout après compaction ?
- `crates/daly-bms-server/src/state.rs` — quels champs grossissent au cours du temps ?
- `crates/daly-bms-server/src/state.rs` — `log_buffer: LogBuffer` — borné ?
- `crates/daly-bms-server/src/console.rs` — broadcast 512 events × taille Arc<ConsoleEvent>

**Étape 4 — Vérification par mesure**

Reproduire la croissance en désactivant systématiquement chaque agent :
1. Désactiver MQTT bridges → mesurer croissance/h
2. Désactiver BMS polling → mesurer
3. Désactiver writer redb → mesurer
4. Trouver le coupable par élimination

## État du repo + déploiement Pi5

```bash
# Pi5 actuel
PID: tournant sur le binaire commit ~Phase 5.1 (md5 différent du `target/release/`)
Config /etc/daly-bms/config.toml :
  [metrics_store] enabled = true, cache_mb = 64, db_path = /mnt/nvme/daly-bms/metrics.redb
  [victoriametrics] enabled = false (section présente mais ignorée par le code)
  default_backend = "vm" (champ ignoré, allow_dead_code)
Base redb : ~1.2 Go (importée de VM le 18 mai)
```

## Commits récents pertinents

- `d5e483d` — refactor: retrait TOTAL VictoriaMetrics
- `de33bbd` — fix: deploy-pi5.sh auto-répare [metrics_store].enabled
- `bd254f1` — fix: MALLOC_ARENA_MAX=2 dans le unit
- `75b6b84` — fix(perf): jemalloc à la place de glibc
- `4d7f0df` — fix(systemd): tune jemalloc dirty_decay_ms

## Objectif de la session

Identifier la source précise de la fuite **+13 Mo/heure passive** et la corriger. Cible : RSS stable à ±5 Mo de la baseline sur 24 h sans activité.

## Mesure de référence pour valider le fix

Après chaque correction :
```bash
PID=$(pgrep -f 'daly-bms-server$' | head -1)
echo "T0:"; sudo cat /proc/$PID/status | grep -E '^VmRSS|^RssAnon'
# Attendre 1h sans rien faire
echo "T+1h:"; sudo cat /proc/$PID/status | grep -E '^VmRSS|^RssAnon'
```

Delta RSS attendu : < 2 Mo/h.

---

Tu peux copier ce prompt dans une nouvelle session. Je suggère aussi de lancer en parallèle, sur Pi5, un tracker continu pour avoir des données fraîches dès le démarrage de la prochaine session :

```bash
PID=$(pgrep -f 'daly-bms-server$' | head -1)
nohup bash -c 'while sleep 600; do
  TS=$(date -Iseconds)
  RSS=$(awk "/^VmRSS/ {print \$2}" /proc/'"$PID"'/status)
  ANON=$(awk "/^RssAnon/ {print \$2}" /proc/'"$PID"'/status)
  echo "[$TS] RSS=${RSS}kB Anon=${ANON}kB"
done' > /tmp/rss-leak-trace.log 2>&1 &
disown
```

→ toutes les 10 min, log la pente. Demain matin tu auras 16-20 points alignés qui confirment ou infirment la fuite linéaire.
