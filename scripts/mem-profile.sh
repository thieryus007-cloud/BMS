#!/usr/bin/env bash
# Profiling mémoire daly-bms-server via jemalloc (--enable-prof).
#
# Process en 2 commandes :
#   bash scripts/mem-profile.sh start    # build léger + déploie + active prof
#   ... attendre 30-60 min en mode passif ...
#   bash scripts/mem-profile.sh stop     # dump + arrête + analyse + restaure
#
# Beaucoup plus léger que dhat :
# - Build identique à `make build-arm` (pas de debuginfo=2 → ~3-5 min sur Pi5)
# - ~5% overhead runtime (vs 10x pour dhat)
# - jemalloc déjà l'allocator de prod, on active juste --enable-prof
#
# Le profil heap (avec backtraces) atterrit dans ./jeprof-heap.txt.
# Le diff entre 2 snapshots = la fuite.

set -euo pipefail

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; CYAN='\033[0;36m'; NC='\033[0m'
info()  { echo -e "${GREEN}[OK]${NC} $*"; }
step()  { echo -e "${CYAN}[>>]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!!]${NC} $*"; }
error() { echo -e "${RED}[XX]${NC} $*" >&2; exit 1; }

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BIN_JEPROF="$REPO_DIR/target/aarch64-unknown-linux-gnu/release/daly-bms-server"
INSTALL_BIN="/usr/local/bin/daly-bms-server"
DROPIN_DIR="/etc/systemd/system/daly-bms.service.d"
DROPIN_FILE="$DROPIN_DIR/jeprof.conf"
PROF_DIR="/var/lib/daly-bms/jeprof"
PROF_PREFIX="$PROF_DIR/heap"
OUT_TEXT="$REPO_DIR/jeprof-heap.txt"
OUT_RAW="$REPO_DIR/jeprof-heap.raw"

cd "$REPO_DIR"

case "${1:-}" in

# =============================================================================
# START — build léger + déploie + active profiling
# =============================================================================
start)
    step "1/6 Vérification dépendance jeprof (apt install libjemalloc-dev)…"
    if ! command -v jeprof >/dev/null 2>&1; then
        warn "jeprof absent — installation requise pour l'analyse"
        echo "  Installe maintenant :  sudo apt install -y libjemalloc-dev"
        echo "  (non bloquant pour la capture, mais nécessaire au stop)"
    else
        info "jeprof disponible : $(jeprof --version 2>&1 | head -1)"
    fi

    step "2/6 Build de la variante jeprof (release normal + flag profiling)…"
    make build-arm-jeprof
    [[ -x "$BIN_JEPROF" ]] || error "Binaire absent : $BIN_JEPROF"
    info "Binaire jeprof compilé"

    step "3/6 Configuration systemd drop-in (env _RJEM_MALLOC_CONF)…"
    sudo mkdir -p "$DROPIN_DIR" "$PROF_DIR"
    sudo chown dalybms:dalybms "$PROF_DIR" 2>/dev/null || true
    sudo rm -f "$PROF_DIR"/heap.* 2>/dev/null || true

    # Config jemalloc :
    #   prof:true             — active le sous-système prof
    #   prof_active:true      — démarre l'enregistrement immédiatement
    #   prof_prefix:...       — chemin des fichiers .heap dumpés
    #   lg_prof_sample:19     — 1 échantillon tous les 512 KB alloués
    #                           (compromis taille/précision)
    #   prof_final:true       — dump automatique au shutdown du process
    sudo tee "$DROPIN_FILE" >/dev/null <<EOF
[Service]
TimeoutStopSec=60
Environment=_RJEM_MALLOC_CONF=prof:true,prof_active:true,prof_prefix:$PROF_PREFIX,lg_prof_sample:19,prof_final:true
EOF
    sudo systemctl daemon-reload
    info "Drop-in en place : $DROPIN_FILE"

    step "4/6 Arrêt service + déploiement binaire jeprof…"
    sudo systemctl stop daly-bms
    sudo cp "$BIN_JEPROF" "$INSTALL_BIN"
    info "Binaire déployé"

    step "5/6 Démarrage service…"
    sudo systemctl start daly-bms
    sleep 4
    if ! systemctl is-active --quiet daly-bms; then
        sudo journalctl -u daly-bms -n 30 --no-pager
        error "Service ne démarre pas — voir logs ci-dessus"
    fi
    info "Service actif"

    step "6/6 Vérification activation du profiling…"
    sleep 3
    if sudo ls "$PROF_DIR" 2>/dev/null | grep -q heap; then
        sudo ls -lh "$PROF_DIR" | head -5
        info "Premier dump heap détecté → profiling actif"
    else
        warn "Pas encore de fichier heap dans $PROF_DIR — attends quelques secondes"
        echo "       sudo ls -lh $PROF_DIR"
    fi

    echo ""
    echo -e "${GREEN}═══════════════════════════════════════════════════════════════${NC}"
    echo -e "${GREEN}  Profiling DÉMARRÉ — laisser tourner 30-60 min en mode passif${NC}"
    echo -e "${GREEN}═══════════════════════════════════════════════════════════════${NC}"
    echo ""
    echo "  Surveiller que la fuite se reproduit avec le binaire jeprof :"
    echo "    PID=\$(pgrep -f 'daly-bms-server\$' | head -1)"
    echo "    watch -n 60 \"sudo cat /proc/\$PID/status | grep -E '^VmRSS|^RssAnon'\""
    echo ""
    echo "  Quand RSS a monté de 5-10 MB, lancer :"
    echo "    bash scripts/mem-profile.sh stop"
    ;;

# =============================================================================
# STOP — dump final, analyse, restauration binaire prod
# =============================================================================
stop)
    step "1/6 Trigger d'un dump final via systemctl stop (prof_final:true)…"
    sudo systemctl stop daly-bms
    info "Service arrêté → dump heap final écrit"

    step "2/6 Listing des fichiers heap capturés…"
    HEAP_FILES=$(sudo find "$PROF_DIR" -name 'heap.*.heap' 2>/dev/null | sort)
    if [[ -z "$HEAP_FILES" ]]; then
        error "Aucun fichier .heap dans $PROF_DIR — profiling pas actif ? Voir : sudo journalctl -u daly-bms -n 50"
    fi
    COUNT=$(echo "$HEAP_FILES" | wc -l)
    info "$COUNT fichiers heap capturés"
    echo "$HEAP_FILES" | xargs sudo ls -lh

    step "3/6 Récupération du heap final + dépendance jeprof…"
    LAST_HEAP=$(echo "$HEAP_FILES" | tail -1)
    FIRST_HEAP=$(echo "$HEAP_FILES" | head -1)
    sudo cp "$LAST_HEAP" "$OUT_RAW"
    sudo chown "$USER:$USER" "$OUT_RAW"
    info "Heap final récupéré : $OUT_RAW"

    if ! command -v jeprof >/dev/null 2>&1; then
        warn "jeprof manquant → analyse impossible en local"
        echo "       sudo apt install -y libjemalloc-dev"
        echo "       puis : jeprof --text --base=$FIRST_HEAP $INSTALL_BIN $LAST_HEAP"
    else
        step "4/6 Génération du rapport texte (diff first → last)…"
        if [[ "$FIRST_HEAP" == "$LAST_HEAP" ]]; then
            warn "Un seul snapshot disponible — diff impossible, sortie absolue"
            sudo jeprof --text --show_bytes --lines "$INSTALL_BIN" "$LAST_HEAP" > "$OUT_TEXT" 2>/dev/null || \
                sudo jeprof --text --show_bytes "$INSTALL_BIN" "$LAST_HEAP" > "$OUT_TEXT"
        else
            # --base = ce qu'on soustrait du final → ne reste QUE la fuite
            sudo jeprof --text --show_bytes --lines --base="$FIRST_HEAP" \
                "$INSTALL_BIN" "$LAST_HEAP" > "$OUT_TEXT" 2>/dev/null || \
            sudo jeprof --text --show_bytes --base="$FIRST_HEAP" \
                "$INSTALL_BIN" "$LAST_HEAP" > "$OUT_TEXT"
        fi
        sudo chown "$USER:$USER" "$OUT_TEXT"
        info "Rapport généré : $OUT_TEXT"
    fi

    step "5/6 Build + restauration du binaire prod (sans jeprof)…"
    make build-arm
    sudo rm -f "$DROPIN_FILE"
    sudo systemctl daemon-reload
    sudo cp "target/aarch64-unknown-linux-gnu/release/daly-bms-server" "$INSTALL_BIN"
    sudo systemctl start daly-bms
    sleep 3
    if systemctl is-active --quiet daly-bms; then
        info "Service prod redémarré normalement"
    else
        warn "Service ne redémarre pas — voir : sudo journalctl -u daly-bms -n 30"
    fi

    step "6/6 Résultats"
    echo ""
    echo -e "${GREEN}═══════════════════════════════════════════════════════════════${NC}"
    echo -e "${GREEN}  Profil capturé${NC}"
    echo -e "${GREEN}═══════════════════════════════════════════════════════════════${NC}"
    echo ""
    if [[ -f "$OUT_TEXT" ]]; then
        echo "  Top 20 allocations qui FUITENT (delta first → last) :"
        echo ""
        head -40 "$OUT_TEXT" | sed 's/^/    /'
        echo ""
        echo "  Rapport complet : $OUT_TEXT"
    fi
    echo "  Heap brut       : $OUT_RAW"
    echo ""
    echo "  Pour visualisation graphique :"
    echo "    jeprof --web --base=$FIRST_HEAP $INSTALL_BIN $LAST_HEAP"
    echo "    jeprof --svg --base=$FIRST_HEAP $INSTALL_BIN $LAST_HEAP > leak.svg"
    ;;

# =============================================================================
*)
    cat <<EOF
Usage:
  bash scripts/mem-profile.sh start    # build léger + déploie + démarre profiling
  bash scripts/mem-profile.sh stop     # dump + analyse + restaure binaire prod

Process complet :
  1. bash scripts/mem-profile.sh start
  2. Attendre 30-60 min en mode passif (vérifier que RSS monte)
  3. bash scripts/mem-profile.sh stop
  4. Le rapport ./jeprof-heap.txt liste les allocations qui fuitent

Prérequis : sudo apt install -y libjemalloc-dev   (pour jeprof)
EOF
    exit 1
    ;;
esac
