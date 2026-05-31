# Restauration après le commit de nettoyage

Ce document explique comment revenir en arrière si le commit de nettoyage
(`chore: audit cleanup …`) a supprimé un fichier que vous voulez récupérer.

Trois options, du moins destructif au plus destructif.

---

## Option 1 — Restaurer depuis votre clone de backup (recommandé)

Si vous avez **cloné le dépôt avant le nettoyage** (par exemple dans
`~/Daly-BMS-Rust-backup` ou ailleurs), utilisez le script fourni :

```bash
# Dry-run (ne modifie rien, montre ce qui serait restauré)
bash scripts/restore-from-backup.sh /chemin/vers/clone-backup --dry-run

# Restauration réelle
bash scripts/restore-from-backup.sh /chemin/vers/clone-backup

# Si tout est OK, committer la restauration
git add -A
git commit -m "revert: restore files from backup clone"
git push -u origin $(git branch --show-current)
```

Le script utilise `rsync -a --delete` (en excluant `.git/` et `target/`)
pour aligner le contenu du dépôt courant sur le backup. L'historique git
local est **préservé**.

---

## Option 2 — Restaurer un fichier ou un répertoire précis depuis git

Si seuls quelques fichiers manquent et que vous connaissez le SHA du
commit avant le nettoyage :

```bash
# Liste des commits récents
git log --oneline -20

# Récupérer un fichier précis depuis un ancien commit
git checkout <SHA-avant-cleanup> -- chemin/vers/fichier

# Ou un répertoire entier
git checkout <SHA-avant-cleanup> -- crates/daly-bms-cli/

# Committer
git commit -m "revert: restore <fichier>"
```

---

## Option 3 — Revert complet du commit de nettoyage

Annule tout le commit de nettoyage en créant un nouveau commit "revert" :

```bash
# Trouver le SHA du commit de nettoyage
git log --oneline | grep -i "audit cleanup"

# Revert (crée un nouveau commit qui annule l'ancien)
git revert <SHA-du-commit-de-cleanup>
git push
```

Cette option est **non-destructive** : l'historique est conservé, on
ajoute juste un commit qui annule l'ancien.

---

## Option 4 — Reset destructif (à éviter sauf cas extrême)

Si la branche n'a pas encore été partagée et que vous voulez l'effacer
purement et simplement :

```bash
git reset --hard <SHA-avant-cleanup>
git push --force-with-lease
```

⚠️ Destructif : perd tous les commits faits après le SHA cible. À ne
faire que si vous êtes seul sur la branche.

---

## Liste des éléments supprimés par le commit de nettoyage

Pour mémoire, voici ce que le commit `chore: audit cleanup` a retiré :

- **Crates dev** : `crates/daly-bms-cli/`, `crates/daly-bms-probe/`
- **Mode simulation** : `crates/daly-bms-server/src/simulator.rs` + flags
  `--simulate`/`--sim-bms` dans `main.rs`
- **Templates inutilisés** : `templates/correctif.html`, `overview.html`,
  `schema.html`
- **Stack Docker test** : `Dockerfile`, `docker-compose.yml`,
  `Config.docker.toml`, `DEPLOY.sh`
- **NanoPi legacy** : `nanoPi/config-bms1.ini`, `nanoPi/config-bms2.ini`
  (Python `dbus-mqtt-battery`, remplacé par `dbus-mqtt-venus` Rust)
- **Docs obsolètes** : `Plan.md`, `Nodered-vm-test.json`,
  `phase3-venus-dbus-evaluation.md`, `PI5-FREEZE-INVESTIGATION.md`,
  `Fix-waterHeaterMode.md`, `Rust-logic-graph.md`, `Rust-rule-engine.md`,
  `windmill.md`, `Windows.md`, `Grafana.md`, `pv_dashboard_grafana.json`,
  `README_pv_grafana.md`, `total_solar_power.json`, `Readme-Lynx.md`,
  `JSONData.json`, `Gmail - Daly BMS1.pdf`, `issue.md`, `Irradiance.png`
- **Annexes** : `contrib/nginx.conf`, `contrib/irradiance-rs485/`
  (Python remplacé par Rust), `.env` (legacy InfluxDB commenté)

**Conservés** (cœur production) :
- Crates `rs485-bus`, `daly-bms-core`, `daly-bms-server`,
  `energy-manager`, `dbus-mqtt-venus`
- `Config.toml`, `nanoPi/config-nanopi.toml`
- Mosquitto Docker (`docker-compose.infra.yml` + `docker/mosquitto/`)
- Services systemd (`contrib/*.service`, `install-systemd.sh`)
- PDF de référence (Daly UART, ET112, Modbus, solar-radiation)
- Guides : `energy-manager-guide.md`, `ATS_CHINT_MAINTENANCE.md`,
  `DEPLOY-VENUS-ARMV7.md`, `VENUS-DEVICE-INTEGRATION.md`,
  `redb-queries.md`
