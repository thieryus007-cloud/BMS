#!/usr/bin/env bash
# Profiling heap dhat — capture la fuite mémoire de daly-bms-server.
#
# Process en 2 étapes :
#   bash scripts/dhat-profile.sh start    # build dhat + déploie + démarre
#   ... attendre 30-60 min sans toucher au serveur ...
#   bash scripts/dhat-profile.sh stop     # arrête, récupère JSON, restaure prod
#
# Le JSON final atterrit dans ./dhat-heap.json (CWD = repo).
# Analyse : ouvrir https://nnethercote.github.io/dh_view/ et glisser le fichier.
# Trier par "t-end bytes" → la backtrace en tête = la fuite.

set -euo pipefail

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; CYAN='\033[0;36m'; NC='\033[0m'
info()  { echo -e "${GREEN}[OK]${NC} $*"; }
step()  { echo -e "${CYAN}[>>]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!!]${NC} $*"; }
error() { echo -e "${RED}[XX]${NC} $*" >&2; exit 1; }

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BIN_DHAT="$REPO_DIR/target/aarch64-unknown-linux-gnu/release-debug/daly-bms-server"
BIN_PROD="$REPO_DIR/target/aarch64-unknown-linux-gnu/release/daly-bms-server"
DHAT_JSON_SERVICE="/var/lib/daly-bms/dhat-heap.json"
DHAT_JSON_OUT="$REPO_DIR/dhat-heap.json"
INSTALL_BIN="/usr/local/bin/daly-bms-server"
DROPIN_DIR="/etc/systemd/system/daly-bms.service.d"
DROPIN_FILE="$DROPIN_DIR/dhat-timeout.conf"

cd "$REPO_DIR"

case "${1:-}" in

# =============================================================================
# START — build dhat, déploie, démarre, vérifie
# =============================================================================
start)
    step "1/6 Build de la variante dhat-heap (peut prendre 5-15 min)…"
    make build-arm-dhat
    [[ -x "$BIN_DHAT" ]] || error "Binaire dhat absent : $BIN_DHAT"
    info "Binaire dhat compilé"

    step "2/6 Override systemd TimeoutStopSec=120 (pour laisser dhat écrire le JSON)…"
    sudo mkdir -p "$DROPIN_DIR"
    sudo tee "$DROPIN_FILE" >/dev/null <<EOF
[Service]
TimeoutStopSec=120
EOF
    sudo systemctl daemon-reload
    info "Override appliqué (sera retiré au stop)"

    step "3/6 Arrêt service daly-bms…"
    sudo systemctl stop daly-bms

    step "4/6 Déploiement binaire dhat + nettoyage ancien JSON…"
    sudo cp "$BIN_DHAT" "$INSTALL_BIN"
    sudo rm -f "$DHAT_JSON_SERVICE"
    info "Binaire dhat en place : $INSTALL_BIN"

    step "5/6 Démarrage service…"
    sudo systemctl start daly-bms
    sleep 5
    if ! systemctl is-active --quiet daly-bms; then
        sudo journalctl -u daly-bms -n 30 --no-pager
        error "Service daly-bms n'a pas démarré — voir logs ci-dessus"
    fi

    step "6/6 Vérification que dhat est actif…"
    if sudo journalctl -u daly-bms -n 50 --no-pager | grep -q '\[dhat\]'; then
        sudo journalctl -u daly-bms -n 50 --no-pager | grep '\[dhat\]'
        info "dhat profiling ACTIF"
    else
        warn "Pas de log [dhat] détecté. Vérifier : sudo journalctl -u daly-bms -f"
    fi

    echo ""
    echo -e "${GREEN}═══════════════════════════════════════════════════════════════${NC}"
    echo -e "${GREEN}  Profiling DÉMARRÉ — laisser tourner 30-60 min en mode passif${NC}"
    echo -e "${GREEN}═══════════════════════════════════════════════════════════════${NC}"
    echo ""
    echo "  Surveiller le RSS pour confirmer que la fuite se reproduit :"
    echo "    PID=\$(pgrep -f 'daly-bms-server\$' | head -1)"
    echo "    watch -n 60 \"sudo cat /proc/\$PID/status | grep -E '^VmRSS|^RssAnon'\""
    echo ""
    echo "  Quand RSS a monté de ~5-10 MB, lancer :"
    echo "    bash scripts/dhat-profile.sh stop"
    ;;

# =============================================================================
# STOP — arrête proprement, récupère JSON, restaure binaire prod
# =============================================================================
stop)
    step "1/5 Arrêt PROPRE du service (peut prendre 30-60s — dhat sérialise le JSON)…"
    sudo systemctl stop daly-bms
    info "Service arrêté"

    step "2/5 Récupération de dhat-heap.json…"
    if ! sudo test -f "$DHAT_JSON_SERVICE"; then
        error "Fichier $DHAT_JSON_SERVICE absent — le service a peut-être été tué (kill -9 ?) ou dhat n'était pas activé"
    fi
    sudo cp "$DHAT_JSON_SERVICE" "$DHAT_JSON_OUT"
    sudo chown "$USER:$USER" "$DHAT_JSON_OUT"
    SIZE=$(du -h "$DHAT_JSON_OUT" | cut -f1)
    info "Fichier récupéré : $DHAT_JSON_OUT ($SIZE)"

    step "3/5 Build du binaire prod (sans dhat)…"
    if [[ ! -x "$BIN_PROD" ]]; then
        make build-arm
    else
        info "Binaire prod déjà présent (sauté make build-arm)"
    fi

    step "4/5 Restauration binaire prod + redémarrage…"
    sudo cp "$BIN_PROD" "$INSTALL_BIN"
    sudo rm -f "$DROPIN_FILE"
    sudo systemctl daemon-reload
    sudo systemctl start daly-bms
    sleep 3
    if systemctl is-active --quiet daly-bms; then
        info "Service daly-bms redémarré en mode prod"
    else
        warn "Service ne démarre pas — voir : sudo journalctl -u daly-bms -n 30"
    fi

    step "5/5 Profil disponible"
    echo ""
    echo -e "${GREEN}═══════════════════════════════════════════════════════════════${NC}"
    echo -e "${GREEN}  Profil capturé : $DHAT_JSON_OUT${NC}"
    echo -e "${GREEN}═══════════════════════════════════════════════════════════════${NC}"
    echo ""
    echo "  Analyse :"
    echo "    1. Sur ton poste de dev :"
    echo "         scp pi5compute@192.168.1.141:$DHAT_JSON_OUT ."
    echo "    2. Ouvrir https://nnethercote.github.io/dh_view/"
    echo "    3. Glisser-déposer dhat-heap.json"
    echo "    4. Trier par 't-end bytes' (mémoire LIVE retenue à la fin)"
    echo "    5. La backtrace en tête = la fuite"
    ;;

# =============================================================================
*)
    cat <<EOF
Usage:
  bash scripts/dhat-profile.sh start    # build dhat + déploie + démarre
  bash scripts/dhat-profile.sh stop     # arrête, récupère JSON, restaure prod

Process complet :
  1. bash scripts/dhat-profile.sh start
  2. Attendre 30-60 min en mode passif (vérifier que RSS monte)
  3. bash scripts/dhat-profile.sh stop
  4. Récupérer ./dhat-heap.json sur le poste de dev (scp)
  5. Ouvrir https://nnethercote.github.io/dh_view/ et glisser le fichier
EOF
    exit 1
    ;;
esac
