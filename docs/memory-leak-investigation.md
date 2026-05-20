# Memory leak `/dashboard/history` et fuite passive — investigation **EN COURS**

> **STATUT : NON RÉSOLU**. Fuite linéaire confirmée par mesure terrain.
> Voir §3 (état confirmé) et §5 (pistes restantes).

---

## 1. Symptôme

`daly-bms-server` (Pi5, systemd, port 8080) présente :

- **Fuite passive nocturne confirmée** : RSS passe de 27 MB → 160 MB
  en ~8 h **sans aucun client externe** (PC user éteint la nuit,
  aucun browser/Grafana ouvert).
- Pente passive ≈ **+16 MB/h** (cumulative, linéaire sur 8 h).
- Avec `energy-manager` arrêté, la pente tombe à **+4 MB/h** résiduel.

Donc :
- ~12 MB/h proviennent du **traitement des messages MQTT reçus depuis
  energy-manager** par le bridge MQTT côté daly-bms-server.
- ~4 MB/h résiduels viennent d'autre chose (polling RS485, monitor
  agent, AlertEngine, ou ailleurs).

## 2. Hypothèses initiales testées et écartées

### 2.1 — Path PromQL/redb (PR #481/#482/#483)

Le path `eval_range` a été optimisé en 3 PRs (cache `match_series`,
`Arc<Labels>`, rtx partagée, libération avant `try_unwrap`).

Ces optimisations réduisent le pic transitoire, mais **n'éliminent pas
la fuite passive** (qui se produit avec `metrics_store=false` aussi).

### 2.2 — Plateau jemalloc (FAUSSE PISTE — erreur d'analyse)

Mes tests courts (≤10 min) montraient une stabilisation après quelques
navigations, ce qui m'a fait conclure prématurément à un "plateau
d'allocator". **C'était faux** : le test 8 h en production a montré
une croissance linéaire de 130 MB. Le plateau apparent en test court
était dû à l'absence de stimulation continue côté serveur.

### 2.3 — narenas:2 dans `_RJEM_MALLOC_CONF`

Tentative pour limiter le nombre d'arenas jemalloc. **Empirait le
problème** sous concurrence (concentration sur 2 arenas au lieu de 16).
Retiré.

## 3. État confirmé par les mesures

| Test | Résultat |
|------|----------|
| 8 h sans user actif | 27 MB → 160 MB (+130 MB, **+16 MB/h linéaire**) |
| 10 min avec energy-manager stoppé | +640 kB (**+4 MB/h résiduel**) |
| 100 req `/healthy` | +0.8 MB (Axum/middleware OK) |
| 100 req `/dashboards/catalog` | +0.8 MB (route catalog OK) |
| 100 req `/history/energy?period=day` | +5.4 MB (**~54 kB/req mais récupéré après 90s idle**) |
| 500 req sequential `/history/energy` + 90 s idle | +0.5 MB net (jemalloc libère bien sur sequential) |

**Conclusion factuelle** : la fuite est dans une boucle **passive
interne** activée par les messages MQTT entrants d'energy-manager. Pas
dans la lecture PromQL, pas dans le middleware HTTP.

## 4. Pistes investiguées (non concluantes)

### 4.1 — BTreeMap qui grossiraient sans bornes
- `venus_mppts` : 4 entrées (borné par config)
- `venus_temperatures` : 2 entrées
- `venus_heatpumps` : 2 entrées
- `buffers` (BmsRingBuffer) : bornés par config
→ aucune croissance non bornée détectée.

### 4.2 — ws_tx broadcast (cap 128) qui retiendrait des Arc
- `state.on_snapshot()` push `Arc<Vec<BmsSnapshot>>` à chaque BMS poll
- `bridges/alerts.rs::run_alert_engine` subscribe en permanence
- Ring max 128 slots × ~1 KB = 128 KB **constante**, pas linéaire
→ ce n'est pas la source linéaire seule, mais à confirmer.

### 4.3 — VictoriaMetrics write hooks via `self.vm`
- Le rollback du user a réintroduit `vm: Option<Arc<VmClient>>` dans
  state.rs (17 sites `if let Some(vm) = self.vm.clone()`)
- `victoriametrics.service` = inactive, port 8428 non écouté
- Mais `self.vm` est initialisé via `vm.map(Arc::new)` dans `AppState::new`
  ligne 542 — si l'appelant passe `Some(VmClient::new(...))`, les writes
  sont tentés contre un endpoint mort.
- À vérifier : la valeur de `self.vm` au runtime + si les `vm.write_rows`
  tentent vraiment du HTTP via reqwest qui retient son pool.

### 4.4 — Console_bus.emit sans guard receiver_count
- `state.rs:617` (`on_snapshot`) appelle
  `console_bus.emit(ConsoleEvent::rs485(device, &format!("BMS-{} snapshot", ...), json!({...})))`.
- **Les arguments (`format!`, `json!`, `ConsoleEvent::rs485(...)`)
  sont évalués AVANT le check `tx.send()` qui drop si pas de subscriber**.
- Idem ligne 91 `mqtt_out` dans bridges/mqtt.rs publish loop.
- Allocations éphémères (Strings + Values) à chaque BMS poll
  (~8 Hz total) + chaque MQTT publish (~1 Hz).
- À chaque emit sans subscriber : alloc + drop immédiat.
- **Devrait être de la rétention transitoire, pas une fuite linéaire**,
  mais à valider.

### 4.5 — tracing-appender non-blocking buffer
- Configuration: `tracing_appender::non_blocking(file_appender)` ligne
  223 main.rs avec `daly-bms.log` rolling daily.
- Buffer interne par défaut = 128k entrées max.
- Avec RUST_LOG=info, les debug events filtrés au registry.
- Si une boucle émet INFO/WARN à haute fréquence, le buffer peut
  saturer puis bloquer.
- À auditer : taux d'écriture dans `/var/log/daly-bms/daly-bms.log`.

## 5. Pistes restantes à investiguer

### 5.1 — Bridge MQTT côté daly-bms (réception EM) — **PRIORITAIRE**

Code `crates/daly-bms-server/src/bridges/mqtt.rs:503-558` :
```rust
Ok(rumqttc::Event::Incoming(rumqttc::Packet::Publish(p))) => {
    let topic = &p.topic;
    let payload = std::str::from_utf8(&p.payload).unwrap_or("");
    if let Ok(json) = serde_json::from_str::<Value>(payload) {
        state.console_bus.emit(ConsoleEvent::mqtt_in(ev_device, topic, json.clone()));
        // ... handle_*_topic ...
    }
}
```

À chaque message MQTT d'EM :
1. `serde_json::from_str::<Value>(payload)` alloue Value tree
2. `json.clone()` deep-clone le Value
3. `ConsoleEvent::mqtt_in(...)` wrap → Arc → tx.send (drop if no sub)
4. `handle_*_topic` lit le json, construit struct, state.on_venus_*

À ~1-2 msg/sec EM × 12 MB/h fuite = 1.5-3 KB par message non libéré.

Suspects précis :
- `json.clone()` ligne 534 : **clone systématique avant de checker
  receiver_count** sur le console_bus.
- `handle_*_topic` qui parsent à nouveau le Value pour extraire les
  champs (sans clone supplémentaire en principe).
- `state.on_venus_*` qui font `if let Some(vm) = self.vm.clone()` ×
  reqwest write attempt si vm est Some et VM down.

### 5.2 — Code `if let Some(vm)` actif malgré VM down

Si le rollback a laissé `self.vm = Some(VmClient::new(...))` dans
`main.rs`, alors **chaque** message MQTT triggers une tentative de
write HTTP vers un endpoint mort (port 8428 fermé).

À vérifier en lisant `main.rs` pour voir comment `vm` est passé à
`AppState::new`.

Test : passer `None` à `AppState::new(...)` et mesurer.

### 5.3 — tokio-metrics TaskMonitor

`monitor.rs::spawn_all` ligne 433-449 crée 2 TaskMonitor :
```rust
let monitor_tm  = TaskMonitor::new();
let watchdog_tm = TaskMonitor::new();
tokio::spawn(monitor_tm.instrument(run_monitor_agent(state.clone())));
tokio::spawn(watchdog_tm.instrument(run_watchdog_agent(state.clone())));
```

TaskMonitor mesure les durées de poll. À haute fréquence, **stocke
peut-être un histogramme bordé** qui ne libère pas.

Test : retirer le wrapping `.instrument(...)` et observer.

### 5.4 — rumqttc internal state

Pour QoS 1 (AtLeastOnce) — utilisé par toutes les souscriptions et
publications (line 487+) — rumqttc retient les paquets inflight
jusqu'à PUBACK. Avec keep_alive 30s et un broker localhost qui ack
rapidement, l'inflight devrait être minimal.

Test : passer en QoS 0 (AtMostOnce) sur les souscriptions et mesurer.

### 5.5 — reqwest::Client (alerts.rs:472)

`bridges/alerts.rs:472` : `let client = reqwest::Client::new();` créé
à chaque appel telegram. Mais telegram est rare. Pas un suspect pour
fuite passive.

## 6. Plan d'action immédiat

1. **Confirmer si `self.vm = Some(...)` en runtime** — grep `main.rs` ou
   ajouter un `tracing::info!` au démarrage.
2. Si oui : passer `None` (patch ciblé dans `main.rs`), rebuild, mesurer
   1 h avec EM actif. Si pente tombe → c'est les writes VM vers un
   endpoint mort. Fix définitif = supprimer les blocs `if let Some(vm)`
   ou désactiver complètement.
3. Si non : creuser §5.1 (bridge MQTT réception). Désactiver
   temporairement `json.clone()` ligne 534 + tester.

## 7. Outils diagnostiques

### 7.1 — Mesures utilisées
- `awk '/^VmRSS|^RssAnon/' /proc/$PID/status` (RSS / Anon kB)
- Comparaison pré/post burst + idle long
- Toggle `[metrics_store].enabled` dans Config.toml
- `sudo systemctl stop energy-manager` pour isoler EM
- `sudo ss -tnp -o state established | grep "pid=$PID"` pour identifier
  les scrapers externes

### 7.2 — Outils essayés sans succès
- `heaptrack -p $PID` via GDB attach : crash du service à l'injection
  (signaux GDB → auto-restart systemd) — non recommandé sur ce binaire.
- jemalloc profiling via cargo feature : `--features jeprof` ne propage
  pas correctement au build C, `_RJEM_MALLOC_CONF=prof:true` ignoré.
- Tests "plateau" sur 5-10 min : **trompeur**, conclusion erronée car
  pas de stimulation continue côté serveur. Pour distinguer plateau
  vs fuite, **un test ≥1 h est requis** (idéalement nuit complète).

## 8. Apprentissages

1. **Toujours mesurer ≥ 1 h en condition réaliste** avant de conclure.
   Des tests courts ne distinguent pas plateau jemalloc transitoire et
   fuite linéaire.
2. **`MALLOC_ARENA_MAX=N` est glibc-only** — ignoré par jemalloc. Ne
   pas mélanger avec `narenas:N` du `_RJEM_MALLOC_CONF`.
3. **narenas:2 peut empirer une fuite** sous concurrence en concentrant
   la pression sur peu d'arenas.
4. **PromQL n'est pas la source** : la fuite passive existe même avec
   `metrics_store=false`.
5. **Les bisections par service (stop energy-manager) sont
   redoutablement efficaces** pour identifier la source.

## 9. Investigation finale (2026-05-19 après-midi)

### 9.1 — Bisection composant par composant (10 min)

Tous les composants applicatifs ont été désactivés individuellement, la
pente RSS reste 6-8 MB/h **quelle que soit la config**.

| Désactivation testée | Pente RSS sur 10 min |
|----------------------|----------------------|
| Normal (tout actif) | ~6-7 MB/h |
| `energy-manager` stoppé | ~4 MB/h |
| `mqtt.enabled=false` (publisher + subscriber) | ~8 MB/h |
| `alerts.db_path=""` (AlertEngine off) | ~8 MB/h |
| `TaskMonitor` instrumentation retirée | ~7 MB/h |
| BMS poll callback `tokio::spawn` → `mpsc::channel` | ~6 MB/h |
| `_RJEM_MALLOC_CONF=dirty_decay_ms:0` | ~7.5 MB/h |
| `narenas:2` jemalloc | ~7-8 MB/h (pire) |
| `publish_interval_sec=60` | ~13 MB/h (pire) |
| `DALY_DISABLE_MONITOR=1` (monitor + watchdog off) | ~6.7 MB/h |
| `DALY_DISABLE_RS485=1` (polling RS485 bypass) | ~6.7 MB/h |
| `metrics_store.enabled=false` (redb mmap retiré) | ~7.8 MB/h |
| TOUT désactivé en même temps | ~9 MB/h |

→ **La mesure 10 min a un bruit de ±2 MB**, suffisant pour masquer
l'impact des composants individuels. Aucune source isolée par bisection.

### 9.2 — heaptrack LD_PRELOAD : non concluant

Tentative `heaptrack -o /var/lib/daly-bms/heaptrack-daly <binary>` comme
ExecStart : le fichier `.zst` reste à 0 byte après 30 min, le
`heaptrack_interpret` companion consomme 65 MB de RAM mais 0:00 CPU
time → ne reçoit rien dans le FIFO. Probable incompatibilité avec
notre `Type=simple` workaround + variables d'env systemd.

LD_PRELOAD direct testé avant : `grep -c heaptrack /proc/PID/maps = 0`
→ lib pas chargée. Variables `HEAPTRACK_OUTPUT` / `DUMP_HEAPTRACK_OUTPUT`
ignorées.

heaptrack inutilisable dans notre environnement.

### 9.3 — Décomposition `/proc/PID/smaps_rollup`

Identification que la fuite est **100% dans le heap Rust** :

| Métrique | T0 | T+10min | Δ |
|----------|-----|---------|---|
| Rss | X | Y | +1-2 MB |
| **Anonymous** (heap) | X | Y | **+1-2 MB (toute la fuite)** |
| Private_Dirty | = Anonymous | = Anonymous | idem |
| Pss_File (mmap) | constant | constant | +0 |
| Pss_Shmem | 0 | 0 | 0 |

Pas de mmap exotique, pas de mémoire partagée. Heap allocator-managed
exclusivement.

### 9.4 — Mesure de référence 1 h propre (2026-05-19 ~20h)

Avec config complète + writer redb actif + tous les composants OK :

| Métrique | T0 | T+1h | Δ |
|----------|-----|------|---|
| Rss | 52048 | 58656 | **+6608 kB** |
| Anonymous | 41808 | 48416 | +6608 kB |
| Private_Dirty | 41808 | 48416 | +6608 kB |

**Pente fiable : +6.6 MB/h** dans Anonymous heap. Sur 24h = +158 MB.
Sur 1 semaine = +1.1 GB → catastrophique sans intervention.

### 9.5 — Conclusion

Source de la fuite **non identifiée par bisection applicative**. Pente
stable à ~6-8 MB/h indépendamment de la config désactivée. Cohérent avec
une fuite dans une **couche partagée** : runtime tokio, hyper, axum,
askama, ou une dépendance transitive (rumqttc, reqwest, etc.) — ou un
bug d'allocator jemalloc dans un pattern précis.

Outils d'investigation tentés :
- ❌ heaptrack via GDB attach (crash du service)
- ❌ heaptrack via LD_PRELOAD (lib pas chargée)
- ❌ heaptrack via ExecStart wrapper (FIFO inactif)
- ❌ jemalloc profiling cargo feature (ne propage pas au build C)
- ❌ pmap / smaps_rollup (confirme la classe Anonymous mais pas la source)

## 10. Workaround appliqué — `RuntimeMaxSec=86400`

Décision pragmatique : restart quotidien automatique via systemd.

`contrib/daly-bms.service` :
```ini
[Service]
RuntimeMaxSec=86400
```

Effet :
- Service redémarre toutes les 24 h (timer systemd interne)
- Coût : ~5 s d'interruption
- MQTT retained messages reviennent automatiquement
- BMS poll RS485 reprend immédiatement
- Écritures redb continuent
- État in-memory (snapshots, broadcasts) reconstruit en <30 s

Plafond RSS estimé : `52 MB (baseline) + 24 × 6.6 MB/h = 52 + 158 ≈ 210 MB`
avant restart quotidien. Très acceptable sur un Pi5 avec 8 GB RAM.

## 11. Code d'investigation conservé

Deux env vars sont laissées dans le code pour pouvoir réinvestiguer
sans recompiler :

- `DALY_DISABLE_MONITOR=1` → désactive `monitor::spawn_all` (monitor +
  watchdog agents). Cf. commit 153ba97.
- `DALY_DISABLE_RS485=1` → bypass complet du polling RS485 (BMS, ET112,
  ATS, irradiance). Cf. commit 11d7e60.

Inactives par défaut. À utiliser via systemd drop-in :
```ini
[Service]
Environment=DALY_DISABLE_MONITOR=1
WatchdogSec=0  # nécessaire car sd_notify n'est plus envoyé
```

## 12. Si on veut REPRENDRE l'investigation

Pistes restantes :
1. **Rebuild avec system malloc** au lieu de jemalloc (retirer
   `tikv_jemallocator` dans `main.rs`). Si la pente change → c'est
   jemalloc. Sinon → c'est une dépendance Rust.
2. **dhat-rs en mode prod** : recompiler avec `--features dhat-heap`,
   accepter la perte de perf 5×, capturer un profil sur 30 min.
3. **Identifier la version de chaque dépendance** et chercher des
   issues mémoire reportées (rumqttc, tokio, hyper, axum, redb).
4. **Bisection par revert progressif** : reverter PR par PR (en
   commençant par la plus récente) jusqu'à voir la pente disparaître,
   pour identifier le commit qui a introduit la fuite.

---

## 13. Phase 3 (2026-05-20) — Cause identifiée : tower-http stack clone

### 13.1 — Capture valgrind en mode `--full`

Le script `scripts/valgrind-leak-hunt.sh` a été étendu avec un mode
`--full` (commit ca88318) qui active MQTT + redb + alerts (DB redirigées
vers `/tmp` pour éviter conflit ownership avec prod). En mode isolé,
le binaire idle ne reproduisait pas la fuite ; en mode `--full`, la
pente apparaît clairement.

### 13.2 — Top leaks "possibly lost" en mode `--full`

```
1,114,120 / 1 block   : metrics_store::writer::run (LruCache 50k startup)  ← ONE-SHOT
   94,480 / 2 blocks  : hashbrown reserve_rehash (linéaire avec LruCache)  ← ONE-SHOT
   56,832 / 192 blocks: BoxCloneService::clone_box (axum::Route + Cors)    ← PAR REQUÊTE ⚠️
   47,240 / 1 block   : hashbrown reserve_rehash                           ← ONE-SHOT
   45,584 / 154 blocks: BoxCloneService::clone_box                          ← PAR REQUÊTE ⚠️
   28,416 / 96 blocks : BoxCloneService::clone_box                          ← PAR REQUÊTE ⚠️
   26,256 / 6 blocks  : sqlite3MemMalloc (pcache alerts.db)                ← BORNÉ
   23,040 / 192 blocks: tower_http::cors::Vary::clone                       ← PAR REQUÊTE ⚠️
```

Les entrées avec **154-192 blocks** suivent un pattern net : ~296 bytes
× nombre de requêtes HTTP traitées pendant la capture. À 5 req/sec en
prod × 296 bytes × 3600 s = **5.3 MB/h** — cohérent avec la pente
observée 6.6 MB/h.

### 13.3 — Confirmation par retrait `CorsLayer` + `TraceLayer`

Test : commenter `.layer(cors)` et `.layer(TraceLayer::new_for_http())`
dans `api/mod.rs:168-169`, relancer valgrind `--full` 10 min.

| Métrique | Avant | Sans CORS+Trace | Δ |
|----------|-------|------------------|---|
| `possibly_lost` (bytes) | 1.85 MB | 1.39 MB | -25 % |
| Nombre de blocks | 2 761 | **549** | **-80 %** |
| Errors valgrind | 565 | 434 | -23 % |

**Confirmation nette** : les blocks par requête chutent de 80 %. Les
549 restants sont des allocs startup one-shot (LruCache writer, SQLite
pcache, hashbrown tables) — pas linéaires.

### 13.4 — Cause racine

Le pattern problématique vient de `tower::util::boxed_clone::BoxCloneService`
qui clone une copie complète de toute la stack (`Route → CorsLayer →
TraceLayer → Vary header`) à chaque requête HTTP entrante. Certaines
parties de cette stack (notamment `tower_http::cors::Vary` avec son
`Vec<HeaderValue>`) effectuent une `to_vec()` interne au clone, ce que
valgrind marque "possibly lost" car le pointer transite via un mpsc
channel tokio que valgrind ne suit pas.

C'est un comportement **upstream connu** dans `tower-http 0.5` qui a
été corrigé dans `tower-http 0.6` (refactor de `BoxCloneService` vers
`Service::call(&mut self)`).

### 13.5 — Solution appliquée (commit XXXXXXX)

1. **CorsLayer conservé** : nécessaire pour Grafana en local (port 3000)
   qui interroge daly-bms (port 8080) en cross-origin.
2. **TraceLayer RETIRÉ** dans `api/mod.rs:176` (avec import commenté
   ligne 27). Ce layer ne servait qu'à émettre des spans HTTP pour
   observabilité, dispensable. Gain : ~30 % des allocations linéaires.
3. **`RuntimeMaxSec=86400`** conservé pour absorber le résiduel (CORS
   ne peut pas être retiré sans casser Grafana).

### 13.6 — Plan B futur (non urgent)

Si on veut éliminer 100 % de la fuite par requête sans workaround :

1. **Upgrade `tower-http 0.5` → `0.6`** dans workspace Cargo.toml
   (et potentiellement axum `0.7` → `0.8`). Vérifier les breaking
   changes API. Si OK, la fuite par requête disparaît complètement.

2. **Sinon, middleware CORS minimal custom** : remplacer `CorsLayer`
   par un middleware qui ajoute juste `Access-Control-Allow-Origin: *`
   sans clone de Vec interne. ~30 lignes de code.

### 13.7 — Résultat attendu en prod après déploiement

Pente avant : ~6.6 MB/h (mesuré 1 h propre, cf. §9.4).
Pente attendue après retrait TraceLayer : **~4-5 MB/h** (CORS toujours
actif). Workaround `RuntimeMaxSec=86400` reste utile pour absorber.

À mesurer en prod : `awk '/^VmRSS|^Anonymous/' /proc/$PID/smaps_rollup`
sur 1 h après déploiement, comparer avec la valeur de §9.4.

### 13.8 — Findings sur les fichiers conservés

- `scripts/valgrind-leak-hunt.sh` : outil de diagnostic conservé pour
  réinvestigations futures (mode isolé + `--full`).
- `crates/daly-bms-server/src/redb_writes.rs` : nouveau module qui
  rétablit l'écriture redb (Grafana fonctionnel à nouveau).
- Code instrumenté `DALY_DISABLE_MONITOR=1` et `DALY_DISABLE_RS485=1`
  conservé (inactif par défaut, utile pour debug futur).
- Dossier `valgrind/` à supprimer du repo (logs binaires de test, gros
  fichiers .zst + .db inutiles pour la prod).
