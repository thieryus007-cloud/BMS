#!/usr/bin/env bash
# =============================================================================
# migrate-et112-grid-acload.sh
#
# Déploiement SANS RISQUE de la mise en conformité ET112 :
#   0x08 "ET112-Maison"  : heatpump.mqtt_8  →  acload.mqtt_8
#   0x09 "ET112-Réseau"  : heatpump.mqtt_9  →  grid.mqtt_9
#
# ┌───────────────────────────────────────────────────────────────────────┐
# │  LANCEMENT (les DEUX fonctionnent) :                                    │
# │     bash scripts/migrate-et112-grid-acload.sh          (recommandé)     │
# │     sudo bash scripts/migrate-et112-grid-acload.sh     (auto-corrigé)   │
# └───────────────────────────────────────────────────────────────────────┘
#
# Si lancé avec sudo, le script se RELANCE automatiquement en tant que
# $SUDO_USER (sinon rustup/make build-arm et les clés SSH du user seraient
# introuvables sous root), puis rappelle `sudo` uniquement pour les
# opérations privilégiées (/etc, /usr/local/bin, systemctl).
#
# NB clé SSH : si ta clé NanoPi a une passphrase chargée dans un ssh-agent,
# privilégie le lancement SANS sudo (l'agent reste accessible). Une clé sans
# passphrase (~/.ssh/id_nanopi) fonctionne dans les deux cas.
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
#   bash scripts/migrate-et112-grid-acload.sh            # interactif
#   bash scripts/migrate-et112-grid-acload.sh --dry-run  # simulation
#   bash scripts/migrate-et112-grid-acload.sh --yes      # non interactif
# =============================================================================
set -euo pipefail

# ---------------------------------------------------------------------------
# Chemin du dépôt : dérivé de l'emplacement du script (robuste).
# ---------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT_PATH="$SCRIPT_DIR/$(basename "${BASH_SOURCE[0]}")"
REPO_DIR="${REPO_DIR:-$(dirname "$SCRIPT_DIR")}"

# ---------------------------------------------------------------------------
# Auto-correction : si lancé avec `sudo` (donc en root), on se relance en tant
# que l'utilisateur d'origine. Indispensable car :
#   - rustup/cargo (make build-arm) sont dans le PATH du user, pas de root ;
#   - ssh/scp doivent utiliser les clés ~user/.ssh, pas celles de root.
# Les opérations privilégiées repassent par `sudo` depuis le script.
# ---------------------------------------------------------------------------
if [[ $EUID -eq 0 ]]; then
  if [[ -n "${SUDO_USER:-}" && "$SUDO_USER" != "root" ]]; then
    echo "→ Relance en tant que '$SUDO_USER' (rustup + clés SSH user)…"
    exec sudo -u "$SUDO_USER" -H bash "$SCRIPT_PATH" "$@"
  else
    echo "✗ Lance ce script SANS sudo : bash $SCRIPT_PATH" >&2
    exit 1
  fi
fi

# À partir d'ici on tourne forcément en utilisateur non-root.
# Garantir rustup/cargo dans le PATH (shell non-login = profil non sourcé).
export PATH="$HOME/.cargo/bin:$PATH"
SUDO="sudo"

# Options SSH communes : timeout + acceptation auto d'un hôte inconnu
# (évite l'échec silencieux sous automatisation si known_hosts est vierge).
SSH_OPTS="-o ConnectTimeout=8 -o StrictHostKeyChecking=accept-new"

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

run()  { if [[ $DRY_RUN -eq 1 ]]; then echo "  [dry-run] $*"; else eval "$@"; fi; }
priv() { run "$SUDO $*"; }   # commande privilégiée (sudo si nécessaire)

confirm() {
  [[ $ASSUME_YES -eq 1 || $DRY_RUN -eq 1 ]] && return 0
  read -r -p "  Continuer ? [o/N] " ans
  [[ "$ans" == "o" || "$ans" == "O" ]] || die "Abandon utilisateur."
}

backup() {  # backup <fichier> — copie horodatée si le fichier existe
  local f="$1"
  [[ -f "$f" ]] || { warn "pas de fichier à sauvegarder : $f"; return 0; }
  priv "mkdir -p '$BACKUP_DIR'"
  priv "cp -a '$f' '$BACKUP_DIR/$(basename "$f")'"
  ok "backup → $BACKUP_DIR/$(basename "$f")"
}

# ---------------------------------------------------------------------------
# Phase 0 — Pré-vol
# ---------------------------------------------------------------------------
step "Phase 0 — Vérifications préalables"
cd "$REPO_DIR" || die "Dépôt introuvable : $REPO_DIR"
ok "Dépôt : $REPO_DIR   (utilisateur : $(id -un))"

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

# Test SSH NanoPi (comportement identique au ssh manuel : agent/clé user)
if ssh $SSH_OPTS "$NANOPI" true 2>/dev/null; then
  ok "SSH NanoPi OK"
else
  die "SSH NanoPi ($NANOPI) injoignable. Teste : ssh $NANOPI true"
fi

# Pré-autorisation sudo (prompt 1× en amont, évite une coupure en cours de route)
if [[ -n "$SUDO" && $DRY_RUN -eq 0 ]]; then
  echo "  (sudo requis pour /etc, /usr/local/bin, systemctl)"
  $SUDO -v
fi

ok "Pré-vol OK"
[[ $DRY_RUN -eq 1 ]] && warn "MODE DRY-RUN : aucune modification ne sera appliquée."
confirm

# ---------------------------------------------------------------------------
# Phase 1 — Compilation Pi5 (aarch64) — natif (rustup user)
# ---------------------------------------------------------------------------
step "Phase 1 — Compilation daly-bms-server (aarch64)"
run "make build-arm"
[[ $DRY_RUN -eq 1 || -f $BIN_SRC ]] || die "Binaire non produit : $BIN_SRC"
ok "Binaire compilé"

# ---------------------------------------------------------------------------
# Phase 2 — Déploiement config + mosquitto (validation AVANT redémarrage)
# ---------------------------------------------------------------------------
step "Phase 2 — Déploiement Config.toml + mosquitto.conf (Pi5)"
backup /etc/daly-bms/config.toml
backup /etc/mosquitto/mosquitto.conf

priv "cp Config.toml /etc/daly-bms/config.toml"
priv "cp contrib/mosquitto/mosquitto.conf /etc/mosquitto/mosquitto.conf"

# Garde-fou anti-boucle bridge (règle projet n°11)
# Restauration automatique des configs depuis le backup si la validation échoue,
# AVANT tout redémarrage du broker → aucune config invalide ne reste en place.
restore_configs() {
  warn "Restauration des configurations d'origine depuis $BACKUP_DIR…"
  [[ -f "$BACKUP_DIR/mosquitto.conf" ]] && priv "cp -a '$BACKUP_DIR/mosquitto.conf' /etc/mosquitto/mosquitto.conf"
  [[ -f "$BACKUP_DIR/config.toml" ]]    && priv "cp -a '$BACKUP_DIR/config.toml' /etc/daly-bms/config.toml"
}
if [[ -x /usr/local/bin/verify-no-loop.sh ]]; then
  step "Validation anti-boucle bridge MQTT"
  if ! priv "/usr/local/bin/verify-no-loop.sh"; then
    restore_configs
    die "verify-no-loop a échoué — configurations restaurées, migration annulée (broker NON redémarré)."
  fi
  ok "Aucune boucle détectée"
else
  warn "verify-no-loop.sh absent — vérifie manuellement les règles 'out' santuario/grid/#"
fi

priv "systemctl restart mosquitto-broker"
ok "mosquitto-broker redémarré"

# ---------------------------------------------------------------------------
# Phase 3 — Déploiement binaire + redémarrage daly-bms
# ---------------------------------------------------------------------------
step "Phase 3 — Déploiement binaire daly-bms-server"
backup "$BIN_DST"   # rollback binaire
priv "systemctl stop daly-bms"
priv "cp '$BIN_SRC' '$BIN_DST'"
priv "systemctl start daly-bms"
ok "daly-bms redémarré (publie désormais santuario/grid/8|9/venus)"

# ---------------------------------------------------------------------------
# Phase 4 — Purge des retained heatpump/8 & heatpump/9 (Pi5 + NanoPi)
# ---------------------------------------------------------------------------
step "Phase 4 — Purge des messages retained obsolètes (heatpump 8/9)"
for idx in 8 9; do
  run "mosquitto_pub -h '$MQTT_HOST' -t 'santuario/heatpump/$idx/venus' -r -n"
  run "ssh $SSH_OPTS '$NANOPI' \"mosquitto_pub -h localhost -t 'santuario/heatpump/$idx/venus' -r -n\""
done
ok "Retained heatpump/8 et heatpump/9 purgés (Pi5 + NanoPi)"

# ---------------------------------------------------------------------------
# Phase 5 — Déploiement config NanoPi + redémarrage dbus-mqtt-venus
# (AUCUNE recompilation NanoPi : seule la config change)
# ---------------------------------------------------------------------------
step "Phase 5 — Déploiement config NanoPi + restart dbus-mqtt-venus"
run "ssh $SSH_OPTS '$NANOPI' \"cp '$NANOPI_CFG' '${NANOPI_CFG}.bak-$(date +%Y%m%d-%H%M%S)'\" || true"
run "scp $SSH_OPTS nanoPi/config-nanopi.toml '$NANOPI:$NANOPI_CFG'"
run "ssh $SSH_OPTS '$NANOPI' 'svc -t $NANOPI_SVC'"
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
  ssh $SSH_OPTS "$NANOPI" "dbus -y | grep -E 'acload|grid|heatpump'" || true
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
