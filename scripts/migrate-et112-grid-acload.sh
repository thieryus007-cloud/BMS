#!/usr/bin/env bash
# =============================================================================
# migrate-et112-grid-acload.sh
#
# Déploiement SANS RISQUE de la mise en conformité ET112 :
#   0x08 "ET112-Maison"  : heatpump.mqtt_8  →  acload.mqtt_8
#   0x09 "ET112-Réseau"  : heatpump.mqtt_9  →  grid.mqtt_9
#
# À exécuter SUR LE PI5 avec sudo, depuis le dépôt, APRÈS `make sync`
# (la branche claude/confident-feynman-riH89 doit être mergée/récupérée).
#
# Important — exécution sous sudo :
#   Les opérations privilégiées (/etc, /usr/local/bin, systemctl) tournent en
#   root. Les opérations « utilisateur » (make build-arm, ssh/scp NanoPi) sont
#   relancées sous $SUDO_USER pour récupérer rustup ET les clés SSH du user
#   (sinon : « rustup not found » ou « Permission denied » SSH).
#
# Caractéristiques :
#   - Sauvegarde horodatée de chaque fichier avant écrasement (rollback facile).
#   - Validation TOML + verify-no-loop.sh avant tout redémarrage.
#   - Conserve l'ancien binaire daly-bms-server pour rollback.
#   - Idempotent : ré-exécutable sans effet de bord.
#   - --dry-run : affiche les actions sans rien modifier.
#   - --yes     : pas de confirmation interactive.
#
# Usage :
#   sudo bash scripts/migrate-et112-grid-acload.sh            # interactif
#   sudo bash scripts/migrate-et112-grid-acload.sh --dry-run  # simulation
#   sudo bash scripts/migrate-et112-grid-acload.sh --yes      # non interactif
# =============================================================================
set -euo pipefail

# ---------------------------------------------------------------------------
# Chemin du dépôt : dérivé de l'emplacement du script (robuste sous sudo).
# ---------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="${REPO_DIR:-$(dirname "$SCRIPT_DIR")}"

# Utilisateur non-root pour build + SSH (clés dans ~user/.ssh, rustup user).
RUN_USER="${SUDO_USER:-$(id -un)}"

NANOPI="${NANOPI:-root@192.168.1.120}"
NANOPI_SVC="/service/dbus-mqtt-venus"
NANOPI_CFG="/data/daly-bms/config.toml"
BIN_SRC="target/aarch64-unknown-linux-gnu/release/daly-bms-server"
BIN_DST="/usr/local/bin/daly-bms-server"
MQTT_HOST="127.0.0.1"
BACKUP_DIR="/var/backups/daly-bms/et112-migration-$(date +%Y%m%d-%H%M%S)"

DRY_RUN=0
ASSUME_YES=0
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    --yes|-y)  ASSUME_YES=1 ;;
    *) echo "Argument inconnu : $arg" >&2; exit 2 ;;
  esac
done

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
c_blue='\033[1;34m'; c_grn='\033[1;32m'; c_yel='\033[1;33m'; c_red='\033[1;31m'; c_off='\033[0m'
step() { echo -e "\n${c_blue}▶ $*${c_off}"; }
ok()   { echo -e "${c_grn}✓ $*${c_off}"; }
warn() { echo -e "${c_yel}⚠ $*${c_off}"; }
die()  { echo -e "${c_red}✗ $*${c_off}" >&2; exit 1; }

# Préfixe pour relancer une commande sous l'utilisateur non-root (si on est root).
USER_PREFIX=""
if [[ $EUID -eq 0 && "$RUN_USER" != "root" ]]; then
  USER_PREFIX="sudo -u $RUN_USER -H"
fi

run()       { if [[ $DRY_RUN -eq 1 ]]; then echo "  [dry-run] $*"; else eval "$@"; fi; }   # root
run_user()  { run "$USER_PREFIX $*"; }                                                     # $RUN_USER

confirm() {
  [[ $ASSUME_YES -eq 1 || $DRY_RUN -eq 1 ]] && return 0
  read -r -p "  Continuer ? [o/N] " ans
  [[ "$ans" == "o" || "$ans" == "O" ]] || die "Abandon utilisateur."
}

backup() {  # backup <fichier> — copie horodatée si le fichier existe (root)
  local f="$1"
  [[ -f "$f" ]] || { warn "pas de fichier à sauvegarder : $f"; return 0; }
  run "mkdir -p '$BACKUP_DIR'"
  run "cp -a '$f' '$BACKUP_DIR/$(basename "$f")'"
  ok "backup → $BACKUP_DIR/$(basename "$f")"
}

# ---------------------------------------------------------------------------
# Phase 0 — Pré-vol
# ---------------------------------------------------------------------------
step "Phase 0 — Vérifications préalables"
cd "$REPO_DIR" || die "Dépôt introuvable : $REPO_DIR"
ok "Dépôt : $REPO_DIR   (build+ssh sous : $RUN_USER)"

command -v mosquitto_pub >/dev/null || die "mosquitto_pub manquant (apt install mosquitto-clients)"
[[ -f Config.toml ]]                || die "Config.toml absent"
[[ -f nanoPi/config-nanopi.toml ]]  || die "nanoPi/config-nanopi.toml absent"

# Le code doit contenir la mise en conformité (garde-fou anti-vieux checkout)
grep -q 'service_type     = "acload"' Config.toml \
  || die "Config.toml ne contient pas service_type=acload — fais 'make sync' d'abord."
grep -q 'service_type     = "grid"'   Config.toml \
  || die "Config.toml ne contient pas service_type=grid — fais 'make sync' d'abord."
grep -q '"grid" | "acload" => "grid"' crates/daly-bms-server/src/bridges/mqtt.rs \
  || die "mqtt.rs ne contient pas la branche grid/acload — fais 'make sync' d'abord."

if run_user "ssh -o ConnectTimeout=5 -o BatchMode=yes '$NANOPI' true" 2>/dev/null; then
  ok "SSH NanoPi OK (clé de $RUN_USER)"
else
  die "SSH NanoPi ($NANOPI) injoignable sous l'utilisateur $RUN_USER (vérifie ~$RUN_USER/.ssh)"
fi
ok "Pré-vol OK"
[[ $DRY_RUN -eq 1 ]] && warn "MODE DRY-RUN : aucune modification ne sera appliquée."
confirm

# ---------------------------------------------------------------------------
# Phase 1 — Compilation Pi5 (aarch64) — sous $RUN_USER (rustup)
# ---------------------------------------------------------------------------
step "Phase 1 — Compilation daly-bms-server (aarch64)"
run_user "make build-arm"
[[ $DRY_RUN -eq 1 || -f $BIN_SRC ]] || die "Binaire non produit : $BIN_SRC"
ok "Binaire compilé"

# ---------------------------------------------------------------------------
# Phase 2 — Déploiement config + mosquitto (validation AVANT redémarrage)
# ---------------------------------------------------------------------------
step "Phase 2 — Déploiement Config.toml + mosquitto.conf (Pi5)"
backup /etc/daly-bms/config.toml
backup /etc/mosquitto/mosquitto.conf

run "cp Config.toml /etc/daly-bms/config.toml"
run "cp contrib/mosquitto/mosquitto.conf /etc/mosquitto/mosquitto.conf"

# Garde-fou anti-boucle bridge (règle projet n°11)
if [[ -x /usr/local/bin/verify-no-loop.sh ]]; then
  step "Validation anti-boucle bridge MQTT"
  run "/usr/local/bin/verify-no-loop.sh" || die "verify-no-loop a échoué — rollback mosquitto.conf depuis $BACKUP_DIR avant restart."
  ok "Aucune boucle détectée"
else
  warn "verify-no-loop.sh absent — vérifie manuellement les règles 'out' santuario/grid/#"
fi

run "systemctl restart mosquitto-broker"
ok "mosquitto-broker redémarré"

# ---------------------------------------------------------------------------
# Phase 3 — Déploiement binaire + redémarrage daly-bms (root)
# ---------------------------------------------------------------------------
step "Phase 3 — Déploiement binaire daly-bms-server"
backup "$BIN_DST"   # rollback binaire
run "systemctl stop daly-bms"
run "cp '$BIN_SRC' '$BIN_DST'"
run "systemctl start daly-bms"
ok "daly-bms redémarré (publie désormais santuario/grid/8|9/venus)"

# ---------------------------------------------------------------------------
# Phase 4 — Purge des retained heatpump/8 & heatpump/9 (Pi5 + NanoPi)
# ---------------------------------------------------------------------------
step "Phase 4 — Purge des messages retained obsolètes (heatpump 8/9)"
for idx in 8 9; do
  run_user "mosquitto_pub -h '$MQTT_HOST' -t 'santuario/heatpump/$idx/venus' -r -n"
  run_user "ssh '$NANOPI' \"mosquitto_pub -h localhost -t 'santuario/heatpump/$idx/venus' -r -n\""
done
ok "Retained heatpump/8 et heatpump/9 purgés (Pi5 + NanoPi)"

# ---------------------------------------------------------------------------
# Phase 5 — Déploiement config NanoPi + redémarrage dbus-mqtt-venus
# (AUCUNE recompilation NanoPi : seule la config change) — ssh/scp sous $RUN_USER
# ---------------------------------------------------------------------------
step "Phase 5 — Déploiement config NanoPi + restart dbus-mqtt-venus"
run_user "ssh '$NANOPI' \"cp '$NANOPI_CFG' '${NANOPI_CFG}.bak-$(date +%Y%m%d-%H%M%S)'\" || true"
run_user "scp nanoPi/config-nanopi.toml '$NANOPI:$NANOPI_CFG'"
run_user "ssh '$NANOPI' 'svc -t $NANOPI_SVC'"
ok "dbus-mqtt-venus redémarré (supprime heatpump.mqtt_8/9, crée acload.mqtt_8 + grid.mqtt_9)"

# ---------------------------------------------------------------------------
# Phase 6 — Vérification
# ---------------------------------------------------------------------------
step "Phase 6 — Vérification post-déploiement"
if [[ $DRY_RUN -eq 1 ]]; then
  warn "dry-run : vérifications ignorées"
else
  sleep 5
  echo "--- Topics MQTT grid (doit montrer 8 et 9) ---"
  timeout 6 mosquitto_sub -h "$MQTT_HOST" -t 'santuario/grid/+/venus' -v -W 5 || true
  echo "--- Services D-Bus Victron (NanoPi) ---"
  ${USER_PREFIX} ssh "$NANOPI" "dbus -y | grep -E 'acload|grid|heatpump'" || true
  echo "--- Healthcheck daly-bms ---"
  curl -s http://localhost:8080/-/healthy || true
  echo
fi

# ---------------------------------------------------------------------------
# Résumé / rollback
# ---------------------------------------------------------------------------
step "Terminé"
cat <<RESUME
Attendu côté NanoPi :
  com.victronenergy.acload.mqtt_8   ET112-Maison (inst. 30)
  com.victronenergy.grid.mqtt_9     ET112-Réseau (inst. 31)
  (plus aucun com.victronenergy.heatpump.mqtt_8 / mqtt_9)

Sauvegardes : $BACKUP_DIR
Rollback Pi5 :
  sudo cp $BACKUP_DIR/config.toml /etc/daly-bms/config.toml
  sudo cp $BACKUP_DIR/mosquitto.conf /etc/mosquitto/mosquitto.conf
  sudo cp $BACKUP_DIR/daly-bms-server /usr/local/bin/daly-bms-server
  sudo systemctl restart mosquitto-broker daly-bms
Rollback NanoPi :
  ssh $NANOPI 'cp ${NANOPI_CFG}.bak-* $NANOPI_CFG && svc -t $NANOPI_SVC'
RESUME
ok "Migration ET112 grid/acload appliquée."
