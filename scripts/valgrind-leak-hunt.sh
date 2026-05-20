#!/usr/bin/env bash
# valgrind-leak-hunt.sh — Capture un profil valgrind sur daly-bms-server.
#
# Usage (NE PAS lancer en sudo — le script appelle sudo seulement pour
# les commandes systemctl qui en ont besoin) :
#   bash scripts/valgrind-leak-hunt.sh                    # mode isolé, 300s
#   bash scripts/valgrind-leak-hunt.sh 600                # mode isolé, 600s
#   bash scripts/valgrind-leak-hunt.sh 600 --full         # MQTT + redb actifs
#   bash scripts/valgrind-leak-hunt.sh 600 --full --keep  # idem + garde patch
#
# Modes :
#   isolé (défaut) : tout désactivé (mqtt, redb, alerts). Binaire idle sur
#                    Axum HTTP. Capture les leaks pure-runtime tokio/hyper.
#   --full         : mqtt + redb + alerts ACTIFS (DB redirigées vers /tmp
#                    pour ne pas conflit avec prod). RS485 et monitor restent
#                    désactivés (RS485 = /dev/ttyUSB0 owned par dalybms +
#                    locked par service prod ; monitor = spam polkit).
#                    Ce mode capture les leaks du publisher MQTT, du writer
#                    redb et de l'AlertEngine en charge réaliste.

set -uo pipefail

# Refuse d'être lancé en root (cargo n'est pas dans le PATH de root).
if [[ "$EUID" -eq 0 ]]; then
    echo "ERREUR : ne pas lancer en sudo/root — cargo ne sera pas trouvé."
    echo "Lancer simplement : bash scripts/valgrind-leak-hunt.sh [DUREE] [--full] [--keep]"
    exit 1
fi

DURATION="${1:-300}"
FULL_MODE=false
KEEP_PATCH=false
for arg in "${@:2}"; do
    case "$arg" in
        --full) FULL_MODE=true ;;
        --keep) KEEP_PATCH=true ;;
    esac
done

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_DIR"

MAIN_RS="crates/daly-bms-server/src/main.rs"
MAIN_RS_BAK="/tmp/main.rs.before-valgrind"
CONFIG_TMP="/tmp/daly-valgrind.toml"
VALGRIND_LOG="/tmp/valgrind.log"
VALGRIND_SUMMARY="/tmp/valgrind-summary.txt"
# DB temporaires en mode --full (évite conflit avec prod owned dalybms)
TMP_REDB="/tmp/valgrind-metrics.redb"
TMP_ALERTS_DB="/tmp/valgrind-alerts.db"
TMP_DASHBOARDS_DB="/tmp/valgrind-dashboards.db"

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

# ── 2. Config dérivée du template Config.toml du repo + awk modifs ──────────
# Approche définitive : on part du Config.toml du repo qui est le template
# de référence (parser-compatible). On modifie quelques valeurs avec awk
# (plus prévisible que sed pour les modifs section-aware).
if $FULL_MODE; then
    step "Préparation config MODE --full (MQTT + redb + alerts actifs, DB en /tmp)…"
else
    step "Préparation config MODE ISOLÉ (tout désactivé, binaire idle)…"
fi
sudo rm -f "$CONFIG_TMP"

TEMPLATE_CONFIG="$PROJECT_DIR/Config.toml"
[[ -f "$TEMPLATE_CONFIG" ]] || { error "Template absent : $TEMPLATE_CONFIG"; exit 1; }

# Modifs awk selon le mode.
if $FULL_MODE; then
    # Mode --full : conserve mqtt/redb/alerts actifs mais redirige les DB
    # vers /tmp pour éviter conflit avec les fichiers de prod (owned par
    # dalybms) que pi5compute ne peut pas ouvrir.
    awk -v TMP_REDB="$TMP_REDB" -v TMP_ALERTS="$TMP_ALERTS_DB" '
        /^\[/                              { section = $0 }
        section == "[logging]"     && /^log_dir/ { print "log_dir = \"\""; next }
        section == "[metrics_store]" && /^db_path/ { print "db_path = \"" TMP_REDB "\""; next }
        section == "[alerts]"      && /^db_path/ { print "db_path = \"" TMP_ALERTS "\""; next }
        { print }
    ' "$TEMPLATE_CONFIG" > "$CONFIG_TMP"
    # Nettoyage des DB temporaires précédentes pour partir d'un état frais
    rm -f "$TMP_REDB" "$TMP_ALERTS_DB" "$TMP_DASHBOARDS_DB"
else
    # Mode isolé : tout off.
    awk '
        /^\[/                    { section = $0 }
        section == "[logging]"   && /^log_dir/          { print "log_dir = \"\""; next }
        section == "[metrics_store]" && /^enabled/      { print "enabled = false"; next }
        section == "[alerts]"    && /^db_path/          { print "db_path = \"\""; next }
        section == "[mqtt]"      && /^enabled/          { print "enabled = false"; next }
        { print }
    ' "$TEMPLATE_CONFIG" > "$CONFIG_TMP"
fi

# Sécurité : si [logging] n'avait pas log_dir, l'ajouter (pour neutraliser
# le default /var/log/daly-bms).
# Helper : extraire le contenu d'une section TOML (exclut le header pour
# que la range awk ne se referme pas immédiatement sur `^\[`).
section_body() {
    awk -v s="$1" '
        $0 == s { in_section=1; next }
        /^\[/   { in_section=0 }
        in_section { print }
    ' "$CONFIG_TMP"
}

if ! section_body "[logging]" | grep -q '^log_dir'; then
    sed -i '/^\[logging\]/a log_dir = ""' "$CONFIG_TMP"
fi

# Sanity check : valeurs critiques selon le mode
LOG_DIR_LINE=$(section_body "[logging]" | grep '^log_dir' | head -1)
FAIL=0

if $FULL_MODE; then
    # Mode --full : on vérifie que les paths DB sont bien redirigés vers /tmp.
    METRICS_DB=$(section_body "[metrics_store]" | grep '^db_path' | head -1)
    ALERTS_DB=$(section_body  "[alerts]"        | grep '^db_path' | head -1)
    [[ "$LOG_DIR_LINE" == 'log_dir = ""' ]]               || { error "log_dir incorrect : '$LOG_DIR_LINE'"; FAIL=1; }
    [[ "$METRICS_DB"   == *"$TMP_REDB"* ]]                || { error "metrics_store.db_path incorrect : '$METRICS_DB'"; FAIL=1; }
    [[ "$ALERTS_DB"    == *"$TMP_ALERTS_DB"* ]]           || { error "alerts.db_path incorrect : '$ALERTS_DB'"; FAIL=1; }
    MQTT_LINE=$(section_body "[mqtt]" | grep '^enabled' | head -1)
    [[ "$MQTT_LINE"    == 'enabled = true' ]]             || { error "mqtt.enabled incorrect (attendu true en --full) : '$MQTT_LINE'"; FAIL=1; }
    info "Config dédiée MODE --full : $CONFIG_TMP"
    info "  mqtt.enabled         = true (publisher + subscriber actifs)"
    info "  metrics_store.db_path = $TMP_REDB"
    info "  alerts.db_path        = $TMP_ALERTS_DB"
else
    METRICS_LINE=$(section_body "[metrics_store]" | grep '^enabled' | head -1)
    ALERTS_LINE=$(section_body  "[alerts]"        | grep '^db_path' | head -1)
    MQTT_LINE=$(section_body    "[mqtt]"          | grep '^enabled' | head -1)
    [[ "$LOG_DIR_LINE" == 'log_dir = ""' ]]      || { error "log_dir incorrect : '$LOG_DIR_LINE'"; FAIL=1; }
    [[ "$METRICS_LINE" == 'enabled = false' ]]   || { error "metrics_store.enabled incorrect : '$METRICS_LINE'"; FAIL=1; }
    [[ "$ALERTS_LINE"  == 'db_path = ""' ]]      || { error "alerts.db_path incorrect : '$ALERTS_LINE'"; FAIL=1; }
    [[ "$MQTT_LINE"    == 'enabled = false' ]]   || { error "mqtt.enabled incorrect : '$MQTT_LINE'"; FAIL=1; }
    info "Config dédiée MODE ISOLÉ : $CONFIG_TMP (tout désactivé)"
fi

[[ "$FAIL" -eq 0 ]] || { error "Sanity check config échoué — voir $CONFIG_TMP"; exit 1; }

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

# DALY_DISABLE_MONITOR=1  → désactive monitor_agent + watchdog_agent. Sinon
#                          le watchdog tente toutes les 15s un
#                          `systemctl restart energy-manager` qui échoue
#                          (polkit n'autorise que dalybms, pas l'user courant)
#                          et déclenche un agent polkit interactif qui pollue
#                          la sortie + spam les logs.
# DALY_DISABLE_RS485=1    → désactive le polling RS485 BMS. Évite le bruit
#                          dans les logs et simplifie l'investigation
#                          (les "Timeout BMS 0x02" sont juste du bruit).
DALY_CONFIG="$CONFIG_TMP" \
RUST_LOG=warn \
DALY_DISABLE_MONITOR=1 \
DALY_DISABLE_RS485=1 \
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
