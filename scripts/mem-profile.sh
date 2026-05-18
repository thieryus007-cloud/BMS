#!/usr/bin/env bash
# Profiling mémoire daly-bms-server via heaptrack.
#
# Process en 2 commandes :
#   bash scripts/mem-profile.sh start    # build debug + déploie + démarre heaptrack
#   ... attendre 30-60 min en mode passif ...
#   bash scripts/mem-profile.sh stop     # arrête + analyse + restaure binaire prod
#
# Pourquoi heaptrack et pas dhat ou jemalloc-prof :
# - heaptrack s'intègre via LD_PRELOAD au démarrage (pas de modification code)
# - Outil standard Debian (apt install heaptrack heaptrack-gui)
# - Overhead modéré (~2-3x), peut tourner 1h sans saturer la RAM
# - Détection automatique des leaks via `heaptrack_print --print-leaks`
# - dhat-rs : OOM au link sur Pi5 (debuginfo=2 + instrumentation)
# - jemalloc prof : feature `profiling` ne s'active pas via target-conditional dep
#
# Le binaire DOIT contenir des symboles (debuginfo) pour avoir des backtraces
# utiles → on utilise `make build-arm-debug` (profile release-debug) qui a
# `debug=true, strip=none`. Sans symboles, heaptrack montre juste des adresses.

set -euo pipefail

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; CYAN='\033[0;36m'; NC='\033[0m'
info()  { echo -e "${GREEN}[OK]${NC} $*"; }
step()  { echo -e "${CYAN}[>>]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!!]${NC} $*"; }
error() { echo -e "${RED}[XX]${NC} $*" >&2; exit 1; }

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BIN_DEBUG="$REPO_DIR/target/aarch64-unknown-linux-gnu/release-debug/daly-bms-server"
BIN_PROD="$REPO_DIR/target/aarch64-unknown-linux-gnu/release/daly-bms-server"
INSTALL_BIN="/usr/local/bin/daly-bms-server"
DROPIN_DIR="/etc/systemd/system/daly-bms.service.d"
DROPIN_FILE="$DROPIN_DIR/heaptrack.conf"
TRACE_DIR="/var/lib/daly-bms/heaptrack"
TRACE_FILE="$TRACE_DIR/daly-bms.gz"
OUT_LEAKS="$REPO_DIR/heaptrack-leaks.txt"
OUT_TRACE="$REPO_DIR/heaptrack-trace.gz"

cd "$REPO_DIR"

case "${1:-}" in

# =============================================================================
# START — build debug + deploy + start with heaptrack wrapper
# =============================================================================
start)
    step "1/7 Vérification dépendance heaptrack…"
    if ! command -v heaptrack >/dev/null 2>&1; then
        error "heaptrack manquant — installer :  sudo apt install -y heaptrack"
    fi
    HEAPTRACK_BIN="$(command -v heaptrack)"
    info "heaptrack : $HEAPTRACK_BIN ($(heaptrack --version 2>&1 | head -1))"

    step "2/7 Build du binaire avec symboles (make build-arm-debug, ~3-5 min)…"
    make build-arm-debug
    [[ -x "$BIN_DEBUG" ]] || error "Binaire absent : $BIN_DEBUG"

    # FAIL-FAST : vérifier la présence de sections DWARF (debug info pour
    # backtraces heaptrack). readelf est plus fiable que nm car il liste
    # directement les sections ELF sans dépendre du symbol table format.
    if ! readelf -S "$BIN_DEBUG" 2>/dev/null | grep -qE "\.debug_(info|line)"; then
        echo "----- DIAGNOSTIC ELF -----"
        readelf -S "$BIN_DEBUG" 2>/dev/null | head -30
        echo "--------------------------"
        error "Pas de sections .debug_info/.debug_line → backtraces heaptrack inutilisables. Vérifier RUSTFLAGS=-C debuginfo=2 dans make build-arm-debug."
    fi
    DEBUG_SIZE=$(readelf -S "$BIN_DEBUG" 2>/dev/null | awk '/\.debug_info/ {print strtonum("0x"$6)}')
    info "Binaire OK — sections DWARF présentes (.debug_info ≈ $((DEBUG_SIZE / 1024 / 1024)) MB)"

    step "3/7 Préparation du dossier de trace…"
    sudo mkdir -p "$DROPIN_DIR" "$TRACE_DIR"
    sudo chown dalybms:dalybms "$TRACE_DIR" 2>/dev/null || true
    sudo rm -f "$TRACE_DIR"/*.gz 2>/dev/null || true
    info "Dir trace : $TRACE_DIR"

    step "4/7 Configuration systemd drop-in (wrapper heaptrack)…"
    # heaptrack -o <output> <command> : démarre command sous heaptrack
    # qui injecte LD_PRELOAD automatiquement. Le wrapper retourne quand le
    # process enfant termine → systemd voit le wrapper sortir et c'est OK.
    sudo tee "$DROPIN_FILE" >/dev/null <<EOF
[Service]
TimeoutStopSec=60
# Heaptrack capture les allocations et écrit dans /var/lib/daly-bms/heaptrack/
# Le service tourne ~2-3x plus lent — DIAGNOSTIC UNIQUEMENT.
ExecStart=
ExecStart=$HEAPTRACK_BIN -o $TRACE_FILE $INSTALL_BIN
EOF
    sudo systemctl daemon-reload
    info "Drop-in en place : $DROPIN_FILE"

    step "5/7 Arrêt service + déploiement binaire debug…"
    sudo systemctl stop daly-bms
    sudo cp "$BIN_DEBUG" "$INSTALL_BIN"
    info "Binaire deployé"

    step "6/7 Démarrage service sous heaptrack…"
    sudo systemctl start daly-bms
    sleep 5
    if ! systemctl is-active --quiet daly-bms; then
        sudo journalctl -u daly-bms -n 30 --no-pager
        error "Service ne démarre pas — voir logs ci-dessus"
    fi
    info "Service actif"

    # FAIL-FAST : un fichier trace doit apparaître < 15s
    step "7/7 Vérification de l'apparition du fichier de trace (max 15s)…"
    for i in $(seq 1 8); do
        if sudo ls "$TRACE_DIR"/*.gz 2>/dev/null | head -1 >/dev/null; then
            FILE=$(sudo ls -t "$TRACE_DIR"/*.gz 2>/dev/null | head -1)
            SIZE=$(sudo stat -c%s "$FILE")
            info "Trace détectée après ${i}×2s : $(basename "$FILE") ($SIZE bytes)"
            break
        fi
        sleep 2
    done
    if ! sudo ls "$TRACE_DIR"/*.gz 2>/dev/null | head -1 >/dev/null; then
        echo "----- DIAGNOSTIC -----"
        sudo ls -la "$TRACE_DIR" 2>&1 || true
        sudo journalctl -u daly-bms --since "30 seconds ago" --no-pager | tail -20
        echo "----------------------"
        error "Aucun fichier trace dans $TRACE_DIR après 15s"
    fi

    echo ""
    echo -e "${GREEN}═══════════════════════════════════════════════════════════════${NC}"
    echo -e "${GREEN}  Heaptrack VALIDÉ — laisser tourner 30-60 min en mode passif${NC}"
    echo -e "${GREEN}═══════════════════════════════════════════════════════════════${NC}"
    echo ""
    echo "  Surveiller que la fuite se reproduit :"
    echo "    PID=\$(pgrep -f 'daly-bms-server\$' | head -1)"
    echo "    watch -n 60 \"sudo cat /proc/\$PID/status | grep -E '^VmRSS|^RssAnon'\""
    echo ""
    echo "  Quand RSS a monté de 5-10 MB :"
    echo "    bash scripts/mem-profile.sh stop"
    ;;

# =============================================================================
# STOP — stop service (flushes trace), analyze leaks, restore prod
# =============================================================================
stop)
    step "1/5 Arrêt service (heaptrack flushe la trace)…"
    sudo systemctl stop daly-bms
    sleep 2
    info "Service arrêté"

    step "2/5 Récupération de la trace heaptrack…"
    TRACE_GZ=$(sudo ls -t "$TRACE_DIR"/*.gz 2>/dev/null | head -1 || true)
    if [[ -z "$TRACE_GZ" ]]; then
        echo "----- DIAGNOSTIC -----"
        sudo ls -la "$TRACE_DIR" 2>&1 || true
        echo "----------------------"
        warn "Aucune trace heaptrack trouvée — restauration binaire prod"
        sudo rm -f "$DROPIN_FILE"
        sudo systemctl daemon-reload
        [[ -x "$BIN_PROD" ]] && sudo cp "$BIN_PROD" "$INSTALL_BIN"
        sudo systemctl start daly-bms
        error "Pas de trace → relancer 'mem-profile.sh start'"
    fi
    sudo cp "$TRACE_GZ" "$OUT_TRACE"
    sudo chown "$USER:$USER" "$OUT_TRACE"
    SIZE=$(du -h "$OUT_TRACE" | cut -f1)
    info "Trace récupérée : $OUT_TRACE ($SIZE)"

    step "3/5 Analyse de la trace (heaptrack_print)…"
    # heaptrack_print sans flag affiche les sections :
    #   - MOST CALLS TO ALLOCATION FUNCTIONS
    #   - PEAK MEMORY CONSUMERS  ← top des consommateurs RAM avec backtrace
    #   - MOST TEMPORARY ALLOCATIONS
    # Pas de --print-leaks (option inexistante dans heaptrack 1.5).
    heaptrack_print "$OUT_TRACE" > "$OUT_LEAKS"
    info "Rapport : $OUT_LEAKS"

    step "4/5 Restauration binaire prod (sans heaptrack)…"
    if [[ ! -x "$BIN_PROD" ]]; then
        warn "Binaire prod absent — rebuild en cours…"
        make build-arm
    fi
    sudo rm -f "$DROPIN_FILE"
    sudo systemctl daemon-reload
    sudo cp "$BIN_PROD" "$INSTALL_BIN"
    sudo systemctl start daly-bms
    sleep 3
    if systemctl is-active --quiet daly-bms; then
        info "Service prod redémarré normalement"
    else
        warn "Service ne démarre pas — voir : sudo journalctl -u daly-bms -n 30"
    fi

    step "5/5 Top des allocations qui FUITENT"
    echo ""
    echo -e "${GREEN}═══════════════════════════════════════════════════════════════${NC}"
    echo -e "${GREEN}  Heaptrack analyse — Top leaks${NC}"
    echo -e "${GREEN}═══════════════════════════════════════════════════════════════${NC}"
    echo ""
    # Section "PEAK MEMORY CONSUMERS" du rapport heaptrack_print —
    # affiche les sites d'allocation qui retiennent le + de mémoire avec
    # backtrace complète résolue (fichier:ligne).
    awk '/^PEAK MEMORY CONSUMERS/,/^$|^MOST/' "$OUT_LEAKS" | head -60
    echo ""
    echo "  Rapport complet : $OUT_LEAKS"
    echo "  Trace brute     : $OUT_TRACE"
    echo ""
    echo "  Visualisation graphique (GUI) :"
    echo "    sudo apt install heaptrack-gui"
    echo "    heaptrack_gui $OUT_TRACE"
    ;;

# =============================================================================
*)
    cat <<EOF
Usage:
  bash scripts/mem-profile.sh start    # build debug + déploie + démarre heaptrack
  bash scripts/mem-profile.sh stop     # arrête + analyse leaks + restaure prod

Process complet :
  1. sudo apt install -y heaptrack   (une fois)
  2. bash scripts/mem-profile.sh start
  3. Attendre 30-60 min en mode passif (vérifier que RSS monte)
  4. bash scripts/mem-profile.sh stop
  5. Lire ./heaptrack-leaks.txt — top des leaks avec backtraces
EOF
    exit 1
    ;;
esac
