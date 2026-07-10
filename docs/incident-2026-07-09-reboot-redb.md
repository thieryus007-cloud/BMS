# Incident 2026-07-09 — Boucle de crash au reboot (ouverture redb)

> **Statut** : résolu. Correctifs déployés et vérifiés en production.
> **Impact** : site web + API + polling RS485 (BMS/ET112/ATS) indisponibles
> plusieurs heures après un reboot, alors que le Pi5 répondait en SSH.
> **Perte de données** : ~3 h de métriques (fenêtre reboot → intervention) +
> la nuit d'exploitation sur base vide (récupérable, cf. §6).

---

## 1. Résumé (TL;DR)

Après un **reboot non gracieux**, `daly-bms-server` restait bloqué en
`activating (start)` : il ne finissait jamais d'ouvrir la base metrics-store
(redb). La base ayant enflé à **6,1 Go** (`raw_retention_days = 60`), redb
lançait au démarrage une **récupération intégrale** dont le coût est
proportionnel à la taille du fichier. Cette récupération **dépassait
`TimeoutStartSec=300`** → systemd tuait puis relançait le service, **en boucle
infinie toutes les ~5 minutes**. Le port 8080 n'était jamais ouvert **et** le
RS485 ne démarrait jamais (le code se bloque à l'ouverture redb, bien **avant**
`/dev/ttyUSB0`).

Correctifs (3 niveaux, tous déployés) :

1. **`TimeoutStartSec=infinity`** — systemd attend la fin de la récupération,
   quelle que soit la taille du fichier. Fin de la boucle de crash.
2. **Ouverture redb en tâche de fond** — le site HTTP (`READY=1`) et le RS485
   démarrent **sans attendre** redb ; la persistance reprend dès la fin de la
   récupération.
3. **Sauvegarde quotidienne** (`daly-bms-backup.timer`) — filet contre une
   corruption *avérée* (quarantaine automatique → base recréée vide).

Reste, pour supprimer *totalement* le trou de métriques pendant une
récupération : **un UPS + arrêt propre** (cf. §7).

---

## 2. Symptômes

- Pi5 **joignable en SSH** (l'OS est up) mais **site web KO** (`curl
  localhost:8080/-/healthy` → « Could not connect »).
- **RS485 muet** (aucune donnée BMS/ET112/ATS ne se rafraîchit).
- `systemctl status daly-bms` → `Active: activating (start)`, jamais `active`.
- **Le Main PID change toutes les ~5 min** (949 → 1168 → 1215…) = boucle de
  redémarrage.
- Les logs s'arrêtent net après `AlertEngine initialisé` — le log suivant
  attendu (`metrics-store ouvert`) n'apparaît jamais.

> ⚠️ À distinguer du cas « **Pi5 injoignable après reboot** » (SSH KO) qui, lui,
> est un problème **WiFi** (profil NetworkManager perdu) — cf.
> `docs/diagnostic-depannage.md §10`. Ici SSH marche : ce n'est PAS le WiFi.

---

## 3. Diagnostic (démarche)

1. `systemctl status daly-bms` → `activating (start)`, logs stoppés après
   `AlertEngine initialisé`. Dans `main.rs`, l'étape juste après est
   **l'ouverture redb** → suspicion de blocage à l'ouverture.
2. Confirmation par l'état du process :
   ```
   ps -o pid,stat,wchan,etime -p <MAINPID>   → STAT "Dsl" (D = I/O disque ininterruptible)
   cat /proc/<MAINPID>/io                     → read_bytes ≈ 13,5 Go en ~2 min (≈ 2× la taille du fichier)
   ```
   → redb **lit tout le fichier** : c'est une **récupération** en cours.
3. Taille de la base : `ls -lh /mnt/nvme/daly-bms/` → **6,1 Go**.
4. Unité systemd : `TimeoutStartSec=300`, `Type=notify` (donc `READY=1` n'est
   envoyé qu'**après** l'ouverture redb). Le PID qui change toutes les ~5 min =
   systemd atteint le timeout de 300 s, tue, relance → **boucle**.

**Signature à retenir** : `activating (start)` en boucle + STAT `Dsl` +
`read_bytes` >> taille du fichier = **récupération redb qui dépasse le timeout
de démarrage**.

---

## 4. Cause racine

- **Déclencheur** : reboot **non gracieux** (coupure brutale) → redb doit
  **réparer** la base au prochain `open()` (coût ∝ taille du fichier).
- **Amplificateur** : `raw_retention_days = 60` → fichier de plusieurs Go
  (6,1 Go mesuré ; l'ajout récent de la télémétrie Toshiba a contribué). La
  maintenance périodique fait du **tiering logique** mais **jamais de compaction
  physique** en ligne → le fichier ne rétrécit pas tout seul.
- **Défaut de conception** : l'hypothèse « `TimeoutStartSec=300` suffit » était
  vraie pour une ouverture **propre** (rapide, même à 6 Go) mais **fausse pour le
  chemin de récupération** après arrêt non gracieux. Le commentaire de l'unité
  documentait d'ailleurs le risque… en pariant que 300 s suffiraient.
- **Conséquence en cascade** : `Type=notify` + ouverture redb **en avant-plan**
  → tout le démarrage (READY, bind 8080, RS485) était **derrière** l'ouverture
  redb.

Ce **n'est pas** une régression des commits récents (bridges HomeKit/Matter/
Toshiba) : ils ne touchent ni `main.rs` ni le chemin d'ouverture redb.

---

## 5. Correctifs

| # | Correctif | Fichier(s) | Commit |
|---|-----------|-----------|--------|
| 1 | `TimeoutStartSec` 300 → **infinity** | `contrib/daly-bms.service` (+ `scripts/jemalloc-leak-profile.sh`, commentaires `Config.toml`) | `cfe7695` |
| 2 | Ouverture redb **en tâche de fond** (slot renseigné après récupération) | `crates/daly-bms-server/src/{main,state}.rs` (+ sites d'accès migrés) | `c07f5c2` |
| 3 | **Sauvegarde quotidienne** crash-consistent + rotation | `scripts/backup-redb.sh`, `contrib/daly-bms-backup.{service,timer}`, `scripts/deploy-pi5.sh` | `c07f5c2`, `fda5dd6` |

### 5.1 `TimeoutStartSec=infinity`
Aucune valeur **finie** ne couvre un fichier de taille arbitraire (choix
d'exploitation assumé : le fichier *peut* faire des dizaines de Go). `infinity`
fait attendre systemd aussi longtemps que la récupération **progresse** ; il ne
tue jamais un démarrage en cours. Le `WatchdogSec` ne s'arme qu'**après**
`READY=1`, il ne joue donc aucun rôle pendant cette phase.

### 5.2 Ouverture redb en tâche de fond
`AppState` démarre avec un backend `None` (slot
`Arc<std::sync::RwLock<Option<Arc<MetricsStore>>>>`). Une tâche de fond ouvre
redb (sur le pool `spawn_blocking`) puis appelle `state.set_metrics_store()`.
Pendant l'intervalle : écritures de métriques **ignorées**, lectures Grafana =
« backend not ready ». Le site + RS485 sont disponibles **immédiatement**.
Vérif en prod : logs `metrics-store : ouverture en tâche de fond…` puis
`metrics-store ouvert … backend disponible elapsed_s=…`.

### 5.3 Sauvegarde quotidienne
redb est transactionnel (COW + fsync par commit) → une **copie à chaud** capture
un instantané **crash-consistent** (au pire, la restauration déclenche une brève
récupération, désormais tolérée par `infinity`). Le script `backup-redb.sh` :
copie horodatée + rotation (`KEEP`) + garde-fou espace + **verrou flock sur le
répertoire** (anti-concurrence) + purge des `.partial` orphelins. Timer
quotidien à 04:15 (`Persistent=true` → rattrapage si le Pi5 était éteint).

---

## 6. Remise en route & récupération de l'historique

**Remise en route immédiate le jour J** (sans attendre la récupération, avant
le déploiement des correctifs) : mettre la base de côté et repartir vide —
```bash
sudo systemctl stop daly-bms
sudo mv /mnt/nvme/daly-bms/metrics.redb /mnt/nvme/daly-bms/metrics.redb.bloat-<TS>
sudo systemctl start daly-bms   # ouvre une base vide → site + RS485 OK en secondes
```

**Récupération des 60 jours** (le fichier `.bloat` contient l'historique) :
compaction hors-ligne (force la récupération une bonne fois, sans timeout) puis
bascule —
```bash
sudo cp /etc/daly-bms/config.toml /tmp/recover-bloat.toml
sudo sed -i 's#/mnt/nvme/daly-bms/metrics.redb"#/mnt/nvme/daly-bms/metrics.redb.bloat-<TS>"#' /tmp/recover-bloat.toml
sudo -u dalybms env DALY_CONFIG=/tmp/recover-bloat.toml nohup \
     /usr/local/bin/daly-bms-server --compact-db > /tmp/recover-bloat.log 2>&1 &
tail -f /tmp/recover-bloat.log     # attendre "Fichier : X Mo → Y Mo"
# puis bascule (coupure ~5 s) : mv de la base compactée vers metrics.redb
```

> **La nuit d'exploitation sur base vide** (entre la remise en route et la
> bascule de l'historique) reste dans le fichier `metrics.redb.emptystart-*`
> mis de côté. Il n'existe **pas** d'outil de fusion redb→redb (`import_vm` ne
> lit que le format VictoriaMetrics) → cette nuit apparaît comme un **trou** sur
> les dashboards Grafana. Récupérable seulement si un merge est développé.

---

## 7. Ce qui n'est PAS couvert — prévention réelle du trou

Les correctifs 1–3 suppriment l'**indisponibilité** (boucle de crash) et
protègent de la **corruption**, mais **ne suppriment pas le trou de métriques**
pendant la récupération : tant qu'il y a un **arrêt non gracieux**, il y a une
récupération, donc un intervalle sans persistance.

**Seul un onduleur (UPS) + arrêt propre automatique** élimine la cause : plus
d'arrêt brutal → plus de récupération → plus de trou. Piste recommandée :
UPS HAT (PiSugar / Geekworm X1200 / Waveshare) avec un daemon qui déclenche
`systemctl poweroff` sur batterie basse, **ou** alimentation du Pi5 depuis une
source tamponnée du parc batterie. Hygiène complémentaire : ne **jamais** monter
`/mnt/nvme` avec `nobarrier`/`data=writeback` (ce sont les barrières fsync qui
rendent la durabilité réelle) ; NVMe avec *power-loss protection* en bonus.

---

## 8. Leçons

1. **Tester le chemin de récupération**, pas seulement l'ouverture propre : un
   timeout de démarrage doit couvrir le pire cas (grosse base + arrêt brutal).
2. **Découpler l'I/O lourde du démarrage** : un service `Type=notify` ne doit
   pas cacher son `READY=1` derrière une opération de durée non bornée.
3. **La taille du fichier ne doit pas être un facteur de panne** : `infinity` +
   ouverture en fond rendent la robustesse indépendante de la taille.
4. **Sauvegarder** : la quarantaine anti-corruption recrée une base *vide* —
   sans copie, l'historique est perdu.
5. Une **coupure de courant ne détruit pas** l'historique redb (transactionnel) :
   la perte réelle vient d'une intervention manuelle (mise de côté du fichier)
   ou d'une corruption avérée, pas de la coupure elle-même.

---

## 9. Références

- Unité systemd : `contrib/daly-bms.service` (§`TimeoutStartSec`).
- Ouverture en fond : `crates/daly-bms-server/src/main.rs`
  (bloc « OUVERTURE EN TÂCHE DE FOND »), `state.rs`
  (`metrics_store()` / `set_metrics_store()`).
- Sauvegarde : `scripts/backup-redb.sh`, `contrib/daly-bms-backup.{service,timer}`.
- Dépannage : `docs/diagnostic-depannage.md` (WiFi §10), `CLAUDE.md §8`
  (tableau « Problèmes courants »).
- Architecture redb / tiering : `docs/metriques-redb-architecture.md`.
