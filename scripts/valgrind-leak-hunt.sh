#!/usr/bin/env bash
# valgrind-leak-hunt.sh — Capture un profil valgrind sur daly-bms-server.
#
# Usage (NE PAS lancer en sudo — le script appelle sudo seulement pour
# les commandes systemctl qui en ont besoin) :
#   bash scripts/valgrind-leak-hunt.sh              # durée 300s (5 min)
#   bash scripts/valgrind-leak-hunt.sh 600          # durée 10 min
#   bash scripts/valgrind-leak-hunt.sh 300 --keep   # garde le patch jemalloc
#
# Ce que fait le script :
#   1. Stoppe le service systemd daly-bms (libère le port 8080)
#   2. Crée une config temporaire sans file logger (/tmp/daly-valgrind.toml)
#   3. Patche temporairement main.rs pour désactiver jemalloc (sinon valgrind
#      génère des milliers de faux positifs "reachable" internes jemalloc)
#   4. Rebuild en debug (cargo dans le PATH de l'utilisateur courant)
#   5. Lance valgrind avec timeout configurable
#   6. Analyse le rapport, extrait les "definitely lost" / "indirectly lost"
#   7. CLEANUP GARANTI via trap EXIT :
#      - Revert le patch jemalloc
#      - Rebuild release (pour redéploiement futur)
#      - Restart le service systemd
#
# Sorties :
#   /tmp/valgrind.log         — log complet valgrind
#   /tmp/valgrind-summary.txt — extraction des leaks intéressants
#
set -uo pipefail

# Refuse d'être lancé en root (cargo n'est pas dans le PATH de root).
if [[ "$EUID" -eq 0 ]]; then
    echo "ERREUR : ne pas lancer en sudo/root — cargo ne sera pas trouvé."
    echo "Lancer simplement : bash scripts/valgrind-leak-hunt.sh [DUREE]"
    echo "Le script appelle sudo en interne pour les commandes systemctl."
    exit 1
fi

DURATION="${1:-300}"
KEEP_PATCH=false
[[ "${2:-}" == "--keep" ]] && KEEP_PATCH=true

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_DIR"

MAIN_RS="crates/daly-bms-server/src/main.rs"
MAIN_RS_BAK="/tmp/main.rs.before-valgrind"
CONFIG_TMP="/tmp/daly-valgrind.toml"
VALGRIND_LOG="/tmp/valgrind.log"
VALGRIND_SUMMARY="/tmp/valgrind-summary.txt"

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; CYAN='\033[0;36m'; NC='\033[0m'
info()  { echo -e "${GREEN}[OK]${NC} $*"; }
step()  { echo -e "${CYAN}[>>]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!!]${NC} $*"; }
error() { echo -e "${RED}[XX]${NC} $*" >&2; }

# ── Cleanup garanti à la sortie ──────────────────────────────────────────────
cleanup() {
    echo
    step "Cleanup — restore état initial…"

    # Tuer valgrind si encore en cours
    sudo pkill -f "valgrind.*daly-bms-server" 2>/dev/null || true

    # Revert patch jemalloc
    if [[ -f "$MAIN_RS_BAK" ]] && ! $KEEP_PATCH; then
        cp "$MAIN_RS_BAK" "$MAIN_RS"
        rm -f "$MAIN_RS_BAK"
        info "Patch jemalloc reverté"

        # Rebuild release pour que le binaire prod reste à jour
        step "Rebuild release (cargo build --release)…"
        if cargo build --release -p daly-bms-server 2>&1 | tail -3; then
            info "Release rebuild OK"
        else
            warn "Rebuild release a échoué — à refaire manuellement avant déploiement"
        fi
    elif $KEEP_PATCH; then
        warn "Patch jemalloc CONSERVÉ (--keep). À reverter avec :"
        warn "  cp $MAIN_RS_BAK $MAIN_RS && rm $MAIN_RS_BAK"
    fi

    # Restart service
    if sudo systemctl start daly-bms 2>/dev/null; then
        sleep 3
        if systemctl is-active --quiet daly-bms; then
            info "Service daly-bms restarted"
        else
            error "Service daly-bms ne démarre pas — vérifier : sudo systemctl status daly-bms"
        fi
    fi
}
trap cleanup EXIT INT TERM

# ── Vérifications pré-requises ───────────────────────────────────────────────
command -v valgrind >/dev/null || { error "valgrind non installé : sudo apt install valgrind"; exit 1; }
[[ -f "$MAIN_RS" ]] || { error "Pas dans le repo : main.rs introuvable"; exit 1; }
[[ -f /etc/daly-bms/config.toml ]] || { error "Config /etc/daly-bms/config.toml introuvable"; exit 1; }

# ── 1. Stop service ──────────────────────────────────────────────────────────
step "Stop service daly-bms…"
sudo systemctl stop daly-bms || true
sleep 2

# ── 2. Config sans file logger ───────────────────────────────────────────────
step "Préparation config temporaire (log_dir vide)…"
# Supprime un éventuel fichier précédent qui appartiendrait à root et
# bloquerait l'écriture par l'utilisateur courant.
sudo rm -f "$CONFIG_TMP"
# /etc/daly-bms/config.toml est readable par tous, on peut le lire en non-root.
sed 's|^log_dir.*|log_dir = ""|' /etc/daly-bms/config.toml > "$CONFIG_TMP"
if ! grep -q '^log_dir' "$CONFIG_TMP"; then
    sed -i '/^\[logging\]/a log_dir = ""' "$CONFIG_TMP"
fi
info "Config temporaire : $CONFIG_TMP"

# ── 3. Patch main.rs pour désactiver jemalloc ───────────────────────────────
if grep -q '^#\[global_allocator\]' "$MAIN_RS"; then
    step "Patch main.rs : désactivation temporaire de jemalloc…"
    cp "$MAIN_RS" "$MAIN_RS_BAK"
    # Commente les 3 lignes du global_allocator
    sed -i 's|^\(#\[cfg(not(target_env = "msvc"))\]\)$|// VALGRIND PATCH: \1|; s|^\(#\[global_allocator\]\)$|// VALGRIND PATCH: \1|; s|^\(static GLOBAL: tikv_jemallocator::Jemalloc.*\)$|// VALGRIND PATCH: \1|' "$MAIN_RS"
    info "jemalloc désactivé pour ce test"
else
    info "jemalloc déjà absent — pas de patch nécessaire"
fi

# ── 4. Build debug ───────────────────────────────────────────────────────────
step "Build debug avec system malloc (peut prendre 1-3 min)…"
if ! cargo build -p daly-bms-server 2>&1 | tail -3; then
    error "Build échoué"
    exit 1
fi
info "Build OK : ./target/debug/daly-bms-server"

# ── 5. Lance valgrind avec timeout ───────────────────────────────────────────
step "Lancement valgrind (durée: ${DURATION}s = $((DURATION / 60)) min)…"
warn "Le service va tourner ~5-10× plus lentement sous valgrind."
warn "Surveille avec : tail -f $VALGRIND_LOG"

DALY_CONFIG="$CONFIG_TMP" RUST_LOG=warn \
    timeout --foreground -s INT "$DURATION" \
    valgrind \
        --leak-check=full \
        --show-leak-kinds=definite,indirect,possible \
        --num-callers=30 \
        --track-origins=no \
        --log-file="$VALGRIND_LOG" \
        --error-limit=no \
        ./target/debug/daly-bms-server || true

info "Valgrind terminé après ${DURATION}s"

# ── 6. Analyse du rapport ────────────────────────────────────────────────────
step "Analyse de $VALGRIND_LOG…"
[[ -f "$VALGRIND_LOG" ]] || { error "Pas de log valgrind généré"; exit 1; }

{
    echo "=== LEAK SUMMARY ==="
    grep -A 15 "LEAK SUMMARY" "$VALGRIND_LOG" | head -25
    echo
    echo "=== DEFINITELY LOST (top backtraces) ==="
    awk '/definitely lost in loss record/,/^==/{ print }' "$VALGRIND_LOG" | head -200
    echo
    echo "=== INDIRECTLY LOST (top backtraces) ==="
    awk '/indirectly lost in loss record/,/^==/{ print }' "$VALGRIND_LOG" | head -100
    echo
    echo "=== POSSIBLY LOST (top backtraces) ==="
    awk '/possibly lost in loss record/,/^==/{ print }' "$VALGRIND_LOG" | head -100
} > "$VALGRIND_SUMMARY"

info "Résumé écrit dans : $VALGRIND_SUMMARY"
echo
step "Affichage du résumé :"
cat "$VALGRIND_SUMMARY"

echo
echo -e "${GREEN}════════════════════════════════════════════════${NC}"
echo -e "${GREEN}  Investigation terminée${NC}"
echo -e "${GREEN}════════════════════════════════════════════════${NC}"
echo "  Log complet  : $VALGRIND_LOG ($(du -h "$VALGRIND_LOG" | cut -f1))"
echo "  Résumé       : $VALGRIND_SUMMARY"
echo
echo "Pour analyser les backtraces les plus gros leaks :"
echo "  less $VALGRIND_LOG"
echo "  grep -B 1 -A 30 'definitely lost' $VALGRIND_LOG | less"
echo
echo "Le cleanup va se déclencher (revert patch + restart service)…"
