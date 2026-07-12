#!/usr/bin/env bash
#
# setup-otbr-pi5.sh
# ----------------------------------------------------------------------------
# Installation native d'OpenThread Border Router (ot-br-posix) sur le
# Raspberry Pi 5, pour le projet Infrastructure Matter/Thread (Santuario).
# Cf. docs/PLAN-INFRASTRUCTURE-MATTER-THREAD.md (§6, Phase 3).
#
# Cible :  Raspberry Pi 5 — Raspberry Pi OS Bookworm 64 bits — user pi5compute
# Source : ~/Daly-BMS-Rust  (lance via : sudo bash scripts/setup-otbr-pi5.sh)
#
# Fonctionnalités :
#   - Clone et compile ot-br-posix (natif, sans Docker) dans ~/ot-br-posix
#   - Backbone = wlan0 par défaut (ce Pi5 tourne en WiFi, pas eth0 — cf. CLAUDE.md §2)
#   - Compile l'interface web (otbr-web) sur un port dédié (8083 par défaut,
#     pour ne pas entrer en conflit avec daly-bms-server:8080 / energy-manager:8081 / nginx:80)
#   - Laisse otbr-agent et otbr-web désactivés + arrêtés : sans XIAO RCP branché,
#     le chemin radio (/dev/ttyACM0) n'existe pas → risque de boucle crash-restart
#     sur un Pi5 de production (cf. CLAUDE.md §8, incident boucle de redémarrage).
#     Activer manuellement une fois le XIAO#1 flashé + branché (Phase 2 du plan).
#   - Idempotent : peut être relancé (ex. après restauration du Pi5 depuis GitHub)
#     sans casser une install existante.
#
# Options :
#   --web-port=N   : port de l'interface web otbr-web (défaut 8083)
#   --infra-if=IF  : interface réseau backbone (défaut wlan0)
#   --no-web       : ne compile pas l'interface web (otbr-agent seul)
#
# Exemples :
#   sudo bash scripts/setup-otbr-pi5.sh
#   sudo bash scripts/setup-otbr-pi5.sh --web-port=8084 --infra-if=eth0
# ----------------------------------------------------------------------------

set -euo pipefail

# ── Couleurs ──────────────────────────────────────────────────────────────────
GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; BLUE='\033[0;34m'; NC='\033[0m'
info()  { echo -e "${GREEN}[OK]${NC} $*"; }
step()  { echo -e "${BLUE}[>>]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!!]${NC} $*"; }
error() { echo -e "${RED}[!!]${NC} $*" >&2; exit 1; }

# ── Paramètres par défaut ─────────────────────────────────────────────────────
WEB_PORT="8083"
INFRA_IF="wlan0"
BUILD_WEB=true
OTBR_DIR="${HOME}/ot-br-posix"

# ── Parsing args ──────────────────────────────────────────────────────────────
for arg in "$@"; do
    case "$arg" in
        --web-port=*) WEB_PORT="${arg#*=}" ;;
        --infra-if=*) INFRA_IF="${arg#*=}" ;;
        --no-web)     BUILD_WEB=false ;;
        *) error "Option inconnue : $arg" ;;
    esac
done

[[ "$(id -u)" -eq 0 ]] || error "Ce script doit être lancé avec sudo (apt + systemctl)."
REAL_USER="${SUDO_USER:-$(whoami)}"
REAL_HOME="$(getent passwd "$REAL_USER" | cut -d: -f6)"
OTBR_DIR="${REAL_HOME}/ot-br-posix"

step "Interface backbone : ${INFRA_IF}  |  Port web : ${WEB_PORT}  |  Web GUI : ${BUILD_WEB}"

if ip -br link show "$INFRA_IF" &>/dev/null; then
    info "Interface ${INFRA_IF} présente."
else
    warn "Interface ${INFRA_IF} introuvable sur ce système — vérifier avec 'ip -br link show' avant de continuer."
fi

# ── Clone ──────────────────────────────────────────────────────────────────────
if [[ -d "$OTBR_DIR/.git" ]]; then
    info "ot-br-posix déjà cloné dans ${OTBR_DIR} — pas de re-clone."
else
    step "Clonage de ot-br-posix dans ${OTBR_DIR}..."
    sudo -u "$REAL_USER" git clone --recursive --depth=1 https://github.com/openthread/ot-br-posix "$OTBR_DIR"
fi

# ── Bootstrap (dépendances de build) ──────────────────────────────────────────
step "Bootstrap (installation des dépendances de build via apt)..."
( cd "$OTBR_DIR" && ./script/bootstrap )

# ── Compilation + installation ────────────────────────────────────────────────
step "Compilation et installation d'ot-br-posix (peut prendre 15-30 min)..."
if [[ "$BUILD_WEB" == true ]]; then
    ( cd "$OTBR_DIR" && WEB_GUI=1 INFRA_IF_NAME="$INFRA_IF" ./script/setup )
else
    ( cd "$OTBR_DIR" && INFRA_IF_NAME="$INFRA_IF" ./script/setup )
fi

# ── Configuration du port web (évite le conflit avec daly-bms:8080 / energy-manager:8081 / nginx:80) ──
if [[ "$BUILD_WEB" == true ]]; then
    step "Configuration d'otbr-web sur le port ${WEB_PORT}..."
    echo "OTBR_WEB_OPTS=\"-I wpan0 -p ${WEB_PORT}\"" > /etc/default/otbr-web
    info "Écrit /etc/default/otbr-web (OTBR_WEB_OPTS=\"-I wpan0 -p ${WEB_PORT}\")"
fi

# ── Sécurité : désactiver tant que le XIAO RCP n'est pas branché ─────────────
step "Désactivation des services (pas de radio XIAO branchée — à activer manuellement après la Phase 2)..."
systemctl disable otbr-agent &>/dev/null || true
systemctl stop otbr-agent &>/dev/null || true
if [[ "$BUILD_WEB" == true ]]; then
    systemctl disable otbr-web &>/dev/null || true
    systemctl stop otbr-web &>/dev/null || true
fi

info "Installation terminée."
echo ""
systemctl status otbr-agent --no-pager 2>&1 | head -4 || true
if [[ "$BUILD_WEB" == true ]]; then
    systemctl status otbr-web --no-pager 2>&1 | head -4 || true
fi
echo ""
warn "otbr-agent/otbr-web restent désactivés. Une fois le XIAO#1 flashé en RCP et branché :"
warn "  vérifier /dev/ttyACM0 (ou /dev/ttyOTBR si règle udev), puis :"
warn "  sudo systemctl enable --now otbr-agent${BUILD_WEB:+ otbr-web}"
