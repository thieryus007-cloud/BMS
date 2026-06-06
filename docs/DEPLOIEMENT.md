# Déploiement — Pi5 + NanoPi

> Point d'entrée **unique** pour déployer le projet. Procédures basées sur les
> scripts déjà présents (`scripts/deploy-pi5.sh`, `nanoPi/install-venus.sh`,
> cibles `Makefile`).
> Détails NanoPi/armv7 → `docs/DEPLOY-VENUS-ARMV7.md`. Maintenance → `PROCEDURES.md`.

## Cibles & binaires

| Hôte | Arch | Binaire(s) | Superviseur | Chemin |
|------|------|-----------|-------------|--------|
| **Pi5** (`pi5compute@192.168.1.141`) | aarch64 | `daly-bms-server`, `energy-manager` | systemd | `/usr/local/bin/` |
| **NanoPi** (`root@192.168.1.120`) | armv7 | `dbus-mqtt-venus` | runit (`/service/`) | `/data/daly-bms/` |

> ⚠️ Le service lit `/etc/daly-bms/config.toml` (Pi5) et `/data/daly-bms/config.toml`
> (NanoPi), **pas** le `Config.toml` du dépôt. Les templates Askama sont compilés
> dans le binaire → tout changement HTML impose un rebuild + redéploiement.

---

## 0. Préalables

1. **Branche** : `make sync` fait `git reset --hard origin/$(branche-courante)`.
   - Déploiement standard : merger dans `main` puis déployer depuis `main`.
   - Déploiement d'une branche de travail : `git checkout <branche>` sur l'hôte
     **avant** `make sync`.
2. **Toolchain** : `rust-toolchain.toml` épingle `1.94.1`. Au 1er build, `rustup`
   télécharge la toolchain + cibles ARM (une fois, réseau requis).
3. **Cross-deps armv7** (build NanoPi, sur l'hôte de build) :
   `sudo apt install gcc-arm-linux-gnueabihf`.
4. **SSH NanoPi** (une fois) : `ssh-copy-id root@192.168.1.120`.

---

## 1. Pi5 — `daly-bms-server` + `energy-manager` (aarch64)

### Option A — Script tout-en-un (recommandé), **sur le Pi5**
```bash
cd ~/Daly-BMS-Rust
bash scripts/deploy-pi5.sh
```
Ce que fait `scripts/deploy-pi5.sh` :
1. `make sync` — récupère le code.
2. `make build-arm` + `make build-energy-arm` (aarch64, sous l'utilisateur appelant, pas root).
3. Config.toml : copiée vers `/etc/daly-bms/config.toml` **seulement si absente** (jamais d'écrasement en prod).
4. Auto-réparations : `[serial].port` → `/dev/ttyUSB0`, `[metrics_store].enabled=true`, droits du `db_path` (backup horodaté).
5. NVMe monté ? + retrait éventuel de l'ancien VictoriaMetrics.
6. Units systemd (`daly-bms.service`, `energy-manager.service`) mis à jour si modifiés + `daemon-reload`.
7. Bridge Mosquitto : copie + `verify-no-loop.sh` (restauration auto si boucle) + restart.
8. Déploiement binaires : `stop → cp /usr/local/bin → start` (les 2 services).
9. Grafana : datasource + import dashboards (`scripts/fix-grafana.sh`).
10. Validation : `scripts/test-api.sh` + comptage des séries redb.

Flags :
```bash
bash scripts/deploy-pi5.sh --no-build      # binaires déjà construits
bash scripts/deploy-pi5.sh --no-validate   # saute test-api.sh
```
> `rustup: not found` sous `sudo` → builder sans sudo puis :
> `make build-arm && make build-energy-arm && sudo bash scripts/deploy-pi5.sh --no-build`.

### Option B — Manuel, **sur le Pi5**
```bash
cd ~/Daly-BMS-Rust && make sync

# daly-bms-server
make build-arm
sudo systemctl stop daly-bms
sudo cp target/aarch64-unknown-linux-gnu/release/daly-bms-server /usr/local/bin/
sudo systemctl start daly-bms

# energy-manager
make build-energy-arm
sudo systemctl stop energy-manager
sudo cp target/aarch64-unknown-linux-gnu/release/energy-manager /usr/local/bin/
sudo systemctl start energy-manager
```

### Option C — Depuis un poste de dev (cross-build + scp)
```bash
make deploy          # build-arm + scp + install + restart daly-bms
make install-energy  # build-energy-arm + scp + install + restart energy-manager
```
Variables : `PI_HOST ?= pi5compute@192.168.1.141`, `PI_BIN_PATH ?= /usr/local/bin/daly-bms-server`.

---

## 2. NanoPi — `dbus-mqtt-venus` (armv7, Venus OS / runit)

Piloté **depuis le Pi5** (ou poste dev) :
```bash
make build-venus-v7 && make install-venus-v7      # GX_IP ?= 192.168.1.120
```
- `build-venus-v7` : compile `dbus-mqtt-venus` en armv7 (**jamais** `target-cpu=native` → évite le SIGILL).
- `install-venus-v7` → `ARCH=armv7 ./nanoPi/install-venus.sh 192.168.1.120`, qui :
  1. vérifie le binaire armv7 ;
  2. `svc -d` (arrêt runit) avant copie ;
  3. retire `daly-bms-server` du NanoPi s'il traîne (ne doit jamais y tourner) ;
  4. `scp` binaire → `/data/daly-bms/` + `chmod +x` ;
  5. copie `config.toml` **seulement si absent** ;
  6. installe le run script, active le symlink `/service/dbus-mqtt-venus`, `svc -u` ;
  7. persistance boot via `/data/rc.local` ;
  8. `svstat` + commandes de vérif D-Bus.

> Détails / dépannage armv7 : `docs/DEPLOY-VENUS-ARMV7.md`.

---

## 3. Appliquer une configuration (séparé du code)

```bash
# Pi5
sudo cp Config.toml /etc/daly-bms/config.toml
sudo systemctl restart daly-bms        # et/ou energy-manager

# NanoPi
scp nanoPi/config-nanopi.toml root@192.168.1.120:/data/daly-bms/config.toml
ssh root@192.168.1.120 "svc -t /service/dbus-mqtt-venus"
```

---

## 4. Vérifications post-déploiement

```bash
# Pi5
systemctl status daly-bms energy-manager --no-pager
curl -s http://localhost:8080/-/healthy
journalctl -u daly-bms -f
journalctl -u energy-manager -f

# NanoPi
ssh root@192.168.1.120 "svstat /service/dbus-mqtt-venus"
ssh root@192.168.1.120 "dbus -y | grep victronenergy"
```

### Comportements de robustesse à valider
- **Réouverture série** : débrancher/rebrancher l'USB-RS485 → logs
  `Erreur port série/IO … backoff …` puis `Port série réouvert avec succès`,
  **sans** redémarrage manuel (le bus partagé fait repartir ET112/ATS/PRALRAN aussi).
- **Supervision fail-fast** : si une boucle de service meurt, le process sort
  (log `tâche critique … terminée`) → systemd `Restart=on-failure` / runit relance
  (~5 s). Plus de « service up mais sous-système mort ».

---

## 5. Récapitulatif

| Étape | Commande |
|------|----------|
| Pi5 — tout-en-un | `bash scripts/deploy-pi5.sh` |
| Pi5 — code Rust seul | `make build-arm` → stop → `cp` binaire → start |
| Pi5 — energy seul | `make build-energy-arm` → stop → `cp` → start |
| Pi5 — config seule | `sudo cp Config.toml /etc/daly-bms/config.toml && sudo systemctl restart daly-bms` |
| NanoPi — code | `make build-venus-v7 && make install-venus-v7` |
| NanoPi — config | `scp nanoPi/config-nanopi.toml root@192.168.1.120:/data/daly-bms/config.toml && ssh root@192.168.1.120 "svc -t /service/dbus-mqtt-venus"` |

> Dépannage : voir `CLAUDE.md` §8 (PROBLÈMES COURANTS) et `PROCEDURES.md`.
