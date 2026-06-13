#!/usr/bin/env bash
# =============================================================================
# jemalloc-leak-profile.sh — capture automatisée d'un profil de fuite heap
# pour daly-bms-server (investigation RSS, docs/diagnostic-depannage.md §18).
#
# Le script :
#   1. vérifie que le binaire est compilé avec le profiling jemalloc,
#   2. ACTIVE le profiling (drop-in systemd) et redémarre le service,
#   3. mesure pendant une durée donnée (le service dumpe /tmp/jeprof.*.heap
#      toutes les ~5 min via l'agent monitor),
#   4. produit un rapport jeprof : le DIFF entre le 1er et le dernier profil
#      = exactement ce qui a CRÛ (la fuite), au call-stack près,
#   5. RÉTABLIT la configuration normale (profiling OFF) — y compris si le
#      script est interrompu (Ctrl-C, déconnexion SSH, erreur).
#
# Usage :
#   sudo bash scripts/jemalloc-leak-profile.sh [durée]
#   durée : suffixes coreutils — 30m, 2h, 90s… (défaut : 2h)
#
# Pour une mesure longue sans garder le terminal ouvert :
#   sudo tmux new -s prof 'bash scripts/jemalloc-leak-profile.sh 3h'
# =============================================================================
set -euo pipefail

DURATION="${1:-2h}"
SVC=daly-bms
BIN=/usr/local/bin/daly-bms-server
DROPIN_DIR="/etc/systemd/system/${SVC}.service.d"
DROPIN="${DROPIN_DIR}/zz-jemalloc-prof.conf"
OUTDIR=/tmp/jeprof
HEAP_GLOB="/tmp/jeprof.*.heap"

log() { printf '\033[1;34m[prof]\033[0m %s\n' "$*"; }
err() { printf '\033[1;31m[prof]\033[0m %s\n' "$*" >&2; }

if [ "$(id -u)" -ne 0 ]; then
    err "À lancer avec sudo (modification de l'unité systemd)."
    exit 1
fi

# --- Restauration garantie (succès, Ctrl-C, déconnexion, erreur) ------------
# `ENABLED` n'est mis à 1 qu'une fois le profiling réellement activé : sur un
# échec précoce (binaire sans prof, jeprof absent…) on ne redémarre PAS le
# service inutilement.
RESTORED=0
ENABLED=0
restore() {
    [ "$RESTORED" -eq 1 ] && return
    RESTORED=1
    if [ "$ENABLED" -eq 0 ] && [ ! -f "$DROPIN" ]; then
        return  # rien n'a été modifié → aucun redémarrage
    fi
    log "Restauration : profiling OFF + redémarrage normal de $SVC…"
    rm -f "$DROPIN"
    systemctl daemon-reload
    systemctl restart "$SVC" || true
    if systemctl is-active --quiet "$SVC"; then
        log "$SVC redémarré en configuration normale (profiling désactivé)."
    else
        err "$SVC n'est pas actif après restauration — vérifie : journalctl -u $SVC -n 40"
    fi
}
trap restore EXIT INT TERM HUP

# --- 1. binaire compilé avec profiling ? ------------------------------------
# NB : on utilise `grep -c` (lit tout le flux) et NON `grep -q` : sous
# `set -o pipefail`, `grep -q` ferme le tube au 1er match → `strings` reçoit
# SIGPIPE (141) → la pipeline est vue en échec malgré un match (faux négatif).
prof_strings=$(strings "$BIN" 2>/dev/null | grep -c "prof\.dump" || true)
if [ "${prof_strings:-0}" -eq 0 ]; then
    err "Le binaire $BIN n'a PAS le profiling jemalloc compilé."
    err "Déploie la dernière version d'abord :  make sync && sudo bash scripts/deploy-pi5.sh"
    exit 1
fi

# --- 2. jeprof disponible ? -------------------------------------------------
if ! command -v jeprof >/dev/null 2>&1; then
    log "jeprof absent — installation de libjemalloc-dev…"
    apt-get update -qq && apt-get install -y libjemalloc-dev \
        || { err "Échec installation de jeprof. Installe-le manuellement puis relance."; exit 1; }
fi

# --- 3. table rase des anciens profils --------------------------------------
mkdir -p "$OUTDIR"
# shellcheck disable=SC2086
rm -f $HEAP_GLOB 2>/dev/null || true

# --- 4. activer le profiling ------------------------------------------------
log "Activation du profiling jemalloc (drop-in $DROPIN)…"
mkdir -p "$DROPIN_DIR"
cat > "$DROPIN" <<'EOF'
[Service]
Environment=DALY_JEMALLOC_PROF=1
Environment=_RJEM_MALLOC_CONF=prof:true,prof_active:true,lg_prof_sample:19,dirty_decay_ms:1000,muzzy_decay_ms:0,background_thread:true
EOF
systemctl daemon-reload
systemctl restart "$SVC"
sleep 5
if ! systemctl is-active --quiet "$SVC"; then
    err "$SVC n'a pas redémarré avec le profiling :"
    journalctl -u "$SVC" -n 30 --no-pager >&2 || true
    exit 1
fi
log "Profiling actif. Mesure pendant $DURATION."
log "Le service écrit un profil dans $HEAP_GLOB toutes les ~5 min."

# --- 5. attendre la fenêtre de mesure ---------------------------------------
sleep "$DURATION"

# --- 6. analyse : diff premier → dernier profil -----------------------------
mapfile -t HEAPS < <(ls -1tr $HEAP_GLOB 2>/dev/null || true)
if [ "${#HEAPS[@]}" -lt 2 ]; then
    err "Seulement ${#HEAPS[@]} profil(s) capturé(s) — il en faut ≥ 2."
    err "Relance avec une durée plus longue (≥ 15m)."
    exit 1
fi
FIRST="${HEAPS[0]}"
LAST="${HEAPS[-1]}"
REPORT="$OUTDIR/leak-report-$(date +%Y%m%dT%H%M%S).txt"

log "Analyse du diff : ${FIRST##*/}  →  ${LAST##*/}  (${#HEAPS[@]} profils)"
{
    echo "==================================================================="
    echo " Rapport fuite heap — daly-bms-server"
    echo " Date    : $(date)"
    echo " Binaire : $BIN"
    echo " Fenêtre : ${FIRST##*/} → ${LAST##*/} (${#HEAPS[@]} profils)"
    echo "==================================================================="
    echo
    echo "### Allocations qui ont CRÛ pendant la fenêtre (= la fuite) ########"
    echo "# (diff inuse_space, trié par octets décroissants)"
    echo
    jeprof --show_bytes --text --base="$FIRST" "$BIN" "$LAST" 2>/dev/null | head -40
    echo
    echo "### Top allocations vivantes au dernier profil (référence) #########"
    echo
    jeprof --show_bytes --text "$BIN" "$LAST" 2>/dev/null | head -25
} | tee "$REPORT"

# conserver les deux profils clés à côté du rapport
cp -f "$FIRST" "$LAST" "$OUTDIR"/ 2>/dev/null || true
log "Rapport écrit : $REPORT"
log "Profils conservés dans $OUTDIR/ (à joindre si besoin d'analyse)."

# restauration via trap EXIT
