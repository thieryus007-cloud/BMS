# Audit robustesse — juin 2026

> **État d'implémentation (2026-06-09)** : **§3 → §18 IMPLÉMENTÉS** (branche
> `claude/jolly-pascal-0uaha9`, commits `feat(robustesse)`/`test(robustesse)`).
> Restent **§1 et §2** (sécurité), qui demandent une action de l'utilisateur :
> §1 = révoquer/régénérer le token LG ThinQ puis basculer les secrets vers
> `/etc/daly-bms/.env` ; §2 = middleware api_key + CORS (à activer en
> connaissance de cause). Nouveaux outils livrés : `--check-config` (dry-run
> sur les deux binaires), métrique `source_last_update_age_seconds{source=…}`
> (+ `em_source_last_update_age_seconds` côté energy-manager), harnais
> `cargo fuzz` (`fuzz/`), job CI `cargo-deny` bloquant.

> Second audit de robustesse du workspace (le premier, non versionné, est référencé
> dans le code par « audit robustesse §1–§8 » : supervision `spawn_critical`,
> `SharedBus::reopen()`, toolchain épinglée, CI…).
> Celui-ci couvre ce qui reste : sécurité API, panics résiduels, I/O externes,
> persistance, config, tests. **Toutes les recommandations sont conçues pour être
> appliquées sans AUCUNE régression** : comportement nominal identique, gardes
> activables par config, ajouts purement défensifs.
>
> Convention de référence dans le code : `audit 2026-06 §N`.
> Date : 2026-06-09. Périmètre : commit `8fac3dc`.

---

## §0. Synthèse exécutive

| # | Constat | Sévérité | Effort | Fichier |
|---|---------|----------|--------|---------|
| §1 | Secrets LG ThinQ committés en clair dans `Config.toml` | **P0 sécurité** | XS | `Config.toml:578-582` |
| §2 | `api_key` jamais appliquée + mot de passe Daly en dur + CORS `Any` | **P0 sécurité** | S | `api/bms.rs:216`, `api/mod.rs:32` |
| §3 | `eval_range` sans borne de points → pic mémoire (367 Mo observés) | **P0 fiabilité** | S | `metrics-store/src/promql/exec.rs:201` |
| §4 | `HeaderValue::from_str().unwrap()` × 5 → crash-loop sur config invalide | **P0 fiabilité** | XS | `energy-manager/src/http_clients/lg_thinq.rs:83-102` |
| §5 | Pas d'arrêt gracieux SIGTERM → perte du batch metrics **à chaque restart quotidien** | P1 | S | `daly-bms-server/src/main.rs:742` |
| §6 | Clients HTTP `reqwest` **sans timeout** (Open-Meteo, LG ThinQ) | P1 | XS | `open_meteo.rs:63`, `lg_thinq.rs:65` |
| §7 | Unité systemd sans `RequiresMountsFor=/mnt/nvme` → redb créée sur la SD si NVMe absent | P1 | XS | `contrib/daly-bms.service` |
| §8 | `flush_rx()` ne draine qu'un seul `read()` → désync de trame RS485 possible | P1 | S | `rs485-bus/src/lib.rs:179-186` |
| §9 | Tâches longues à sortie normale non supervisées (forwarder switch NanoPi…) | P1 | S | `dbus-mqtt-venus/src/switch_manager.rs:163` |
| §10 | SQLite alertes : pas de WAL/busy_timeout, écritures bloquantes sur le runtime | P2 | S | `daly-bms-server/src/bridges/alerts.rs` |
| §11 | Pas de `MemoryHigh/Max` sur daly-bms.service (fuite résiduelle + pics requêtes) | P2 | XS | `contrib/daly-bms.service` |
| §12 | Validation config inexistante (bornes numériques, clés inconnues silencieuses) | P2 | M | `config.rs` (×2) |
| §13 | WebSocket : pas de ping/pong, clients morts détectés tardivement | P2 | S | `api/bms.rs:607-730` |
| §14 | Port série par chemin instable (`/dev/ttyUSB0`) — `reopen()` peut rouvrir un autre device | P2 | XS (déploiement) | `Config.toml`, udev |
| §15 | Pas de quarantaine si redb corrompue au boot → service sans historique jusqu'à action manuelle | P2 | S | `daly-bms-server/src/main.rs` (ouverture store) |
| §16 | CI sans `cargo-deny`/`cargo-audit` ; pas de dependabot | P2 | XS | `.github/workflows/ci.yml` |
| §17 | Couverture tests très inégale (dbus-mqtt-venus : 1 test / 6,7 k LOC) ; pas de fuzz | P2 | M | — |
| §18 | Observabilité : pas de métrique « âge de la dernière donnée » par source | P2 | S | transverse |

Effort : XS < 1 h, S = 1–4 h, M = 1–2 jours.

---

## §A. Ce qui est déjà robuste (à préserver)

L'essentiel des recommandations du premier audit est en place et fonctionne :

- **Supervision fail-fast** : `spawn_critical` sur toutes les boucles critiques des
  3 binaires + `panic=abort` + `Restart=on-failure` systemd/runit. Avec `panic=abort`,
  une panique dans **n'importe quelle** tâche tue le process → pas de tâche zombie.
- **Réouverture série** : `SharedBus::reopen()` sur `DalyError::Serial`/`Io`, backoff
  borné (2→30 s), partagé par tous les périphériques du bus.
- **Canaux bornés partout** : aucun `unbounded_channel` en chemin chaud ; les
  producteurs RS485 utilisent `try_send` (drop volontaire si consommateur lent,
  documenté `main.rs:654-657`).
- **Production quasi sans `unwrap`** : metrics-store (0 en prod, 169 en tests),
  dbus-mqtt-venus (0), daly-bms-core (1, prouvé infaillible). Les exceptions sont
  listées en §4.
- **redb** : durabilité par défaut = `Immediate` (fsync au commit, redb 4.1) ;
  écriture batchée 250 ms ; rate-limiting des écritures (`redb_writes.rs`).
- **Toolchain épinglée** (1.94.1), `Cargo.lock` committé, CI clippy `-D warnings`
  + cross-builds aarch64/armv7 (piège SIGILL verrouillé).
- **Mémoire** : fuite passive investiguée à fond (tower-http 0.6, axum 0.8, jemalloc
  tuné), workaround `RuntimeMaxSec=86400` assumé et documenté.
- **PromQL** : tests golden (`crates/daly-bms-server/tests/golden_promql.rs`).
- **systemd** : `Type=notify` + `WatchdogSec=60`, watchdog croisé daly-bms →
  energy-manager (sonde TCP 8081 + polkit), `MemoryMax=100M` sur energy-manager.

---

## §B. Findings P0

### §1. Secrets LG ThinQ committés dans `Config.toml` — P0 sécurité

`Config.toml:578-582` contient un `bearer_token` (format `thinqpat_…`), un
`api_key` et un `device_id` LG ThinQ **réels, en clair, dans l'historique git**
poussé sur GitHub. CLAUDE.md (règle 12 et §8 dépannage) prévoit pourtant que ces
secrets vivent dans `/etc/daly-bms/.env`.

Le mécanisme d'override existe déjà et fonctionne (`energy-manager/src/config.rs:495-503` :
`LG_DEVICE_ID`, `LG_BEARER_TOKEN`, `LG_API_KEY` écrasent le TOML ; l'unité a
`EnvironmentFile=-/etc/daly-bms/.env`).

**Remédiation (zéro régression — l'env l'emporte déjà sur le TOML) :**
1. **Révoquer/régénérer le token LG ThinQ** (il restera dans l'historique git même
   après suppression du fichier).
2. Reporter les 3 valeurs dans `/etc/daly-bms/.env` sur le Pi5.
3. Vider les champs dans `Config.toml` (`bearer_token = ""` …) + commentaire
   « → .env ».
4. Optionnel : job CI de détection de secrets (gitleaks) — additif.

### §2. Écritures API : `api_key` morte, mot de passe usine en dur, CORS `Any` — P0 sécurité

Trois constats qui se combinent en une chaîne d'attaque LAN :

- `api.api_key` est définie dans la config (`config.rs:342`) et **affichée** comme
  `auth_enabled` (`api/system.rs:59`) mais **aucun middleware ne la vérifie** —
  grep exhaustif : aucune lecture de header `x-api-key`/`Authorization` dans le
  serveur. CLAUDE.md §7 (« api_key requis si configurée ») est donc inexact.
- La seule protection des POST d'écriture est `DALY_WRITE_PASSWORD = "12345678"`
  (`api/bms.rs:216`) — le mot de passe usine Daly, **public dans ce repo**, comparé
  en temps non constant.
- `CorsLayer` est en `Any` (`api/mod.rs:32`) : une page web malveillante visitée
  depuis une machine du LAN peut émettre des POST cross-origin vers `:8080`
  (couper les MOS, modifier le SOC…), le mot de passe étant connu.

**Remédiation (zéro régression — `api_key = ""` en prod aujourd'hui) :**
1. Middleware `x-api-key` appliqué **uniquement si `api_key` non vide** (comparaison
   constante via `subtle::ConstantTimeEq`), d'abord sur les routes d'écriture
   (POST). Config prod actuelle vide → aucun changement de comportement tant que
   l'utilisateur n'active pas la clé. Mettre `auth_enabled` en cohérence.
2. Documenter dans CLAUDE.md la procédure d'activation (générer la clé, l'ajouter
   au datasource Grafana si on protège aussi les GET un jour — pour l'instant ne
   protéger que les POST pour ne pas toucher Grafana/dashboard).
3. CORS restreint derrière la clé de config existante `cors_allow_all`
   (`Config.toml:62`, aujourd'hui non lue/`true`) : si `false`, n'autoriser que
   same-origin. Défaut = comportement actuel.
4. Garder le mot de passe Daly comme second facteur applicatif (compat UI), mais
   le rendre surchargeable par config (`write_password`, défaut = valeur actuelle).

### §3. `query_range` sans borne de points + pas de cap mémoire — P0 fiabilité

`Evaluator::eval_range` (`metrics-store/src/promql/exec.rs:201`) valide
`step > 0` et `end ≥ start` mais **pas le nombre de pas** `(end-start)/step`.
Conséquence mesurée en prod (commentaire `contrib/daly-bms.service`) : un
dashboard 30 jours → ~270 séries × 43 k itérations → **VmPeak 367 Mo** sur Pi5.
Une requête accidentelle (step minuscule) ou hostile peut déclencher l'OOM killer
**du Pi entier** (daly-bms n'a pas de `MemoryMax`, voir §11).

**Remédiation (zéro régression pour Grafana) :**
1. Garde dans `eval_range` (et `query_range` API) : si
   `(end-start)/step + 1 > max_points`, retourner l'erreur PromQL standard
   (Prometheus refuse > 11 000 points avec
   `exceeded maximum resolution of 11,000 points`). **Grafana calcule toujours
   `step` pour ~1 point/pixel (≤ ~2 000 points)** → aucun dashboard légitime
   n'est affecté ; seules les requêtes aberrantes sont refusées proprement au
   lieu de dégrader tout le Pi.
2. `max_points` configurable (`[metrics_store] query_max_points`, défaut 11 000 —
   sémantique Prometheus).
3. Après (1) seulement : `MemoryHigh=256M` (souple) sur l'unité — voir §11.

### §4. Crash-loop sur config LG invalide — P0 fiabilité

`lg_thinq.rs:83-102` : 5 × `HeaderValue::from_str(...).unwrap()` sur
`bearer_token`, `api_key`, `country`, `client_id` (valeurs venant de
Config.toml/.env). Un caractère non-ASCII ou de contrôle (copier-coller avec
retour chariot dans `.env`, accent dans `client_id`…) → panique à **chaque poll**
du chauffe-eau → avec `panic=abort`, crash-loop du service entier (la logique
solaire/DEYE/ATS tombe avec).

**Remédiation (zéro régression — configs valides inchangées) :**
- Valider les 4 champs **au démarrage** (tentative `HeaderValue::from_str`,
  message d'erreur nommant le champ fautif), puis conserver des `HeaderValue`
  pré-construits dans le client (bonus : plus d'allocation par requête).
- À défaut, `match` sur `from_str` à l'usage : header omis + `warn!` (l'API LG
  répondra 401, déjà géré).

---

## §C. Findings P1

### §5. Pas d'arrêt gracieux SIGTERM — perte de données quotidienne

`daly-bms-server/src/main.rs:742` : `axum::serve(listener, router).await?` sans
`with_graceful_shutdown` ni handler SIGTERM. Or le service redémarre **tous les
jours** (`RuntimeMaxSec=86400`, workaround fuite) : à chaque restart, le batch
metrics en cours (fenêtre 250 ms + backlog du canal writer) est perdu, les
réponses HTTP en vol sont coupées, la connexion MQTT n'est pas fermée proprement
(le broker attend le keep-alive pour purger la session).

**Remédiation (zéro régression) :**
1. `tokio::signal::unix::signal(SignalKind::terminate())` →
   `axum::serve(...).with_graceful_shutdown(...)`.
2. À la sortie : drainer/fluser le writer metrics-store (commit final), publier
   un `disconnect` MQTT. energy-manager : idem sur son serveur :8081.
3. `sd_notify(STOPPING=1)` pour que systemd connaisse l'état.
   Effet : le restart quotidien devient sans perte. Aucun changement nominal.

### §6. Clients HTTP sans timeout

`open_meteo.rs:63` et `lg_thinq.rs:65` : `reqwest::Client::new()` — **aucun
timeout total ni de connexion** (reqwest n'en a pas par défaut). Une connexion
TCP gelée (box, DNS, API LG en panne réseau silencieuse) bloque le poller
**indéfiniment** : météo/chauffe-eau figés sans erreur ni redémarrage (la tâche
est vivante, juste suspendue sur l'await).

**Remédiation (zéro régression) :**
`Client::builder().connect_timeout(5 s).timeout(15 s)` pour les deux. Les appels
qui réussissent aujourd'hui en < 15 s sont inchangés ; les blocages deviennent
des erreurs loggées + retry au tick suivant (chemin d'erreur déjà existant).

### §7. NVMe non garanti au démarrage

`contrib/daly-bms.service` n'a ni `RequiresMountsFor=/mnt/nvme/daly-bms` ni
`ConditionPathIsMountPoint`. Si le montage NVMe échoue/retarde au boot, redb crée
`metrics.redb` **dans le répertoire `/mnt/nvme` de la rootfs (SD)** : remplissage
de la SD, puis base « fantôme » masquée au montage suivant (split-brain
d'historique).

**Remédiation (zéro régression sur boot nominal) :**
`RequiresMountsFor=/mnt/nvme/daly-bms` dans `[Unit]` (ordonne après le mount
et échoue proprement si absent — systemd retentera). Documenter dans CLAUDE.md §8.

### §8. Drain RX incomplet → désynchronisation de trame RS485

`rs485-bus/src/lib.rs:179-186` : `flush_rx()` fait **un seul** `read()` (≤ 256 o)
sous timeout. Scénario : un BMS répond après le timeout de 500 ms → ses 13 octets
traînent dans le buffer ; au cycle suivant, un seul `read()` peut ne drainer
qu'une partie (arrivée en plusieurs chunks UART) → la réponse suivante est lue
décalée. Le checksum + adresse + Data ID rejettent la trame (pas de corruption de
données), mais le **décalage peut persister plusieurs transactions** (erreurs CRC
en rafale observables dans `monitor/rs485-health`).

**Remédiation (zéro régression — buffer vide = comportement identique) :**
- Boucler `read()` jusqu'à buffer vide ou expiration du budget `FLUSH_TIMEOUT_MS`.
- Option défense en profondeur : à l'échec de parse, scanner jusqu'au prochain
  `0xA5` (start flag Daly) avant retry — resynchronisation active.

### §9. Tâches longues à **sortie normale** non supervisées

Avec `panic=abort`, seule une boucle qui peut **retourner normalement** meurt en
silence (une panique tue le process). Tâches concernées :

| Tâche | Sortie normale possible | Conséquence silencieuse |
|---|---|---|
| `dbus-mqtt-venus/switch_manager.rs:163` `run_command_forwarder` | `while let Some = cmd_rx.recv()` → fin si canal fermé (`:287-295`) | Switches **en lecture seule** (ATS/Tongou inertes) |
| `energy-manager/main.rs:136` persist watcher | `break` sur `RecvError::Closed` | Baselines plus restaurées |
| `energy-manager/monitoring.rs:30,35` | retours possibles | Perte métriques tokio/système |

**Remédiation (zéro régression — en nominal ces canaux ne ferment jamais) :**
passer ces spawns en `spawn_critical` (règle de travail 16 : ce sont bien des
boucles de service, pas des one-shot). NB : la tâche de poll Shelly
(`shelly/mqtt.rs:95`) est une boucle infinie sans sortie normale → la laisser
telle quelle est acceptable (uniformiser si souhaité, sans urgence).

---

## §D. Findings P2 (défense en profondeur)

### §10. SQLite alertes : WAL + busy_timeout + spawn_blocking
`bridges/alerts.rs` : connexion unique sous `Mutex`, `INSERT` exécutés dans la
tâche async de l'AlertEngine (bloque un worker tokio pendant l'I/O disque) ; pas
de `PRAGMA journal_mode=WAL` ni `busy_timeout`. Faible débit aujourd'hui → impact
minime, mais un orage d'alertes + disque lent peut geler le runtime.
**Fix additif** : `PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;` à
l'ouverture + envelopper les écritures dans `spawn_blocking` (les lectures API le
sont déjà : `api/alerts.rs:60,81,100`).

### §11. Cap mémoire sur daly-bms.service
energy-manager a `MemoryMax=100M`, daly-bms **rien** — alors que c'est lui qui a
la fuite résiduelle et les pics de requêtes. **Ordre impératif** : d'abord la
borne de points §3 (sinon un dashboard 30 j légitime → 367 Mo → OOM-kill =
régression), ensuite `MemoryHigh=256M` (reclaim souple, pas de kill) puis
éventuellement `MemoryMax=512M` (filet dur, marge ×1,4 sur le pic historique).

### §12. Validation de configuration
Aucune borne vérifiée (`poll_interval_ms = 0` → boucle chaude ; intervalles
négatifs via `as` ; topics vides). Erreurs TOML correctes (`missing field`) mais
sans aide (quel fichier, quelle section attendue).
**Fix additif** : méthode `validate()` appelée après parse (bornes minimales,
messages nommant champ + fichier) + flag `--check-config` (dry-run pour CI/déploiement).
**⚠ À NE PAS FAIRE** : `#[serde(deny_unknown_fields)]` sur les structs racines —
`Config.toml` est **partagé** entre daly-bms-server et energy-manager : chacun
ignore légitimement les sections de l'autre → ce serait une régression de
démarrage immédiate. Pour détecter les typos sans casser : `serde_ignored` en
log `warn!` (purement informatif).

### §13. WebSockets : keepalive et Lagged explicites
`api/bms.rs:607-730` : pas de ping/pong → un client mort n'est détecté qu'au
premier `send` en échec (TCP buffers pleins : minutes) ; il retient récepteur
broadcast + tâche. `rx.recv()` en `select!` à motif `Ok(...)` : un `Lagged`
désactive la branche pour ce tour (comportement final acceptable mais implicite).
**Fix additif** : ping périodique (30 s) + timeout d'inactivité ; gérer
explicitement `Lagged(n)` (log) / `Closed` (break). Aucun changement pour les
clients sains.

### §14. Chemin série stable
`reopen()` rouvre `self.port_path` en dur ; après ré-énumération `/dev/ttyUSB0`
peut désigner un **autre** adaptateur (ou rien). **Fix sans code** : utiliser
`/dev/serial/by-id/usb-…` dans `Config.toml` (symlink udev stable par
VID:PID+serial). Documenter dans CLAUDE.md + `integration-materiel.md`.

### §15. Quarantaine redb au boot
Si `MetricsStore::open` échoue (corruption après coupure brutale), le serveur
continue **sans historique** (warn) jusqu'à intervention manuelle.
**Fix (à valider — politique de données)** : renommer le fichier en
`metrics.redb.corrupt.<ts>` et recréer une base vide (service à nouveau
fonctionnel, l'ancien fichier reste pour autopsie/récupération). Ne jamais
supprimer automatiquement.

### §16. Chaîne d'approvisionnement
`Cargo.lock` committé ✔, toolchain épinglée ✔, mais : pas de `cargo-deny`
(advisories RUSTSEC, licences, doublons) ni dependabot. **Fix additif** : job CI
`cargo-deny check advisories` (non bloquant au début), `deny.toml` minimal,
dependabot mensuel limité à `Cargo.toml` (PRs à merger manuellement — la CI
cross-build protège).

### §17. Tests ciblés sur les frontières d'entrée
État : core 7 tests (protocole), server 29/13 k LOC, **dbus-mqtt-venus 1/6,7 k**,
energy-manager 47, metrics-store 56 + golden PromQL. Les hotspots de fixes
3 mois : promql (6), grafana (4), deye (4) — corrélés aux zones peu testées.
**Priorités additionnelles (additif pur) :**
1. Corpus de trames Daly réelles malformées (tronquées, checksum faux, adresse
   croisée) → `ResponseFrame::parse` ;
2. Golden tests payload MQTT → D-Bus pour dbus-mqtt-venus (mapping
   battery/grid/pvinverter : 1 JSON d'entrée → items D-Bus attendus) ;
3. `cargo-fuzz` sur `ResponseFrame::parse` + parseur PromQL (job CI nightly,
   hors chemin bloquant) ;
4. Tests des décisions Rust (`rules.rs`) charge/deye sur snapshots historiques
   (rejouer un scénario 51,5 Hz).

### §18. Observabilité de fraîcheur
Les morts silencieuses restantes sont toutes de la forme « la donnée ne se
rafraîchit plus ». **Fix additif** : métrique générique
`source_last_update_age_seconds{source=...}` (RS485 par device, MQTT in, Open-Meteo,
LG, Venus) écrite dans metrics-store + règle d'alerte « âge > N×intervalle ».
Transforme tous les findings « silencieux » ci-dessus en alertes actionnables.

---

## §E. Contre-indications (régressions identifiées à NE PAS introduire)

1. **`deny_unknown_fields`** sur la config partagée (§12) — casse le démarrage.
2. **`MemoryMax` avant la borne §3** — OOM-kill sur dashboard 30 j légitime.
3. **`spawn_critical` sur tâches transitoires** (callbacks snapshot, one-shots
   eau chaude) — règle 16 : leur fin normale tuerait le process.
4. **`Durability` redb** : inutile d'y toucher — défaut 4.1 = `Immediate` (fsync
   au commit). L'expliciter en commentaire si souhaité, rien de plus.
5. **Refactor des `lock().unwrap()`** std restants : avec `panic=abort`d
   l'empoisonnement est inobservable — les toucher n'apporte rien et risque des
   erreurs de portée de verrou.
6. **Reformatage massif / rustfmt bloquant** : déjà exclu par le premier audit
   (alignement manuel volontaire).
7. **Protéger les GET par api_key dans un premier temps** : casserait le
   datasource Grafana et le dashboard SSR — ne protéger que les POST.

## Findings d'agents écartés après contre-vérification (transparence)

- « Mutex bus tenu pendant le backoff 2 s » : faux — le guard est relâché au
  retour de `send_command` ; le sleep de `poll_loop` est hors verrou.
- « redb en `Durability::Eventual` par défaut » : faux (4.1 = `Immediate`).
- « unwraps `dashboards/grafana.rs:125-154` en prod » : tous en `#[test]`.
- « Tâche poll Shelly = mort silencieuse HIGH » : boucle infinie sans sortie
  normale + `panic=abort` → ne peut pas mourir en silence ; déclassé (§9 NB).

---

## §F. Plan d'exécution proposé (ordre = dépendances)

| Phase | Contenu | Critère de non-régression |
|-------|---------|---------------------------|
| **1 — Sécurité (XS)** | §1 rotation token + .env ; §4 validation headers LG au boot | configs valides → comportement identique ; `cargo test` vert |
| **2 — Gardes serveur (S)** | §3 borne points PromQL ; §6 timeouts reqwest ; §2 middleware api_key (inactif si clé vide) + CORS opt-in | golden PromQL verts ; dashboards Grafana inchangés (step auto ≤ 2 k pts) ; `api_key=""` → zéro différence observable |
| **3 — Cycle de vie (S)** | §5 graceful shutdown + flush writer ; §7 RequiresMountsFor ; §11 MemoryHigh (après §3) | restart quotidien sans gap dans Grafana ; boot nominal identique |
| **4 — Bus & tâches (S)** | §8 drain bouclé + resync 0xA5 ; §9 spawn_critical ciblés | compteurs `rs485-health` stables ou meilleurs ; soak test 48 h |
| **5 — Fond (M)** | §10 WAL ; §12 validate()+serde_ignored ; §13 WS keepalive ; §14 by-id ; §15 quarantaine ; §16 CI deny ; §17 tests/fuzz ; §18 fraîcheur | chaque item indépendant, livrable séparément |

Chaque phase : branche dédiée, `cargo clippy -D warnings` + tests + cross-builds
CI verts, déploiement Pi5 avec observation `journalctl` + dashboards 24 h avant
la phase suivante.
