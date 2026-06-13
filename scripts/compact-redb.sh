#!/usr/bin/env bash
# =============================================================================
# compact-redb.sh — compaction hors-ligne de la base metrics-store (redb).
#
# Réduit la taille de /mnt/nvme/daly-bms/metrics.redb SANS perdre l'historique :
#   1. abaisse raw_retention_days dans la config (les données brutes anciennes
#      seront agrégées en horaires/journaliers — historique conservé) ;
#   2. arrête daly-bms (redb est mono-process) ;
#   3. lance `daly-bms-server --compact-db` (tiering + compaction physique) ;
#   4. redémarre daly-bms — y compris si le script est interrompu (trap).
#
# Usage : sudo bash scripts/compact-redb.sh [raw_retention_days]   (défaut : 7)
#
# ⚠ Pendant la compaction (quelques minutes sur une grosse base), daly-bms est
#   ARRÊTÉ : pas de collecte BMS/MQTT durant cette fenêtre.
# =============================================================================
set -euo pipefail

RAW_DAYS="${1:-7}"
SVC=daly-bms
BIN=/usr/local/bin/daly-bms-server
CONF=/etc/daly-bms/config.toml
DALY_USER=$(systemctl show "$SVC" --property=User --value 2>/dev/null || echo dalybms)
DALY_USER="${DALY_USER:-dalybms}"

log() { printf '\033[1;34m[compact]\033[0m %s\n' "$*"; }
err() { printf '\033[1;31m[compact]\033[0m %s\n' "$*" >&2; }

[ "$(id -u)" -eq 0 ] || { err "À lancer avec sudo."; exit 1; }
[ -f "$CONF" ] || { err "Config introuvable : $CONF"; exit 1; }

# NB : on ne détecte PAS le support de --compact-db par inspection du binaire.
# En build release, LLVM compile les comparaisons de petites chaînes
# (`a == "--compact-db"`) en comparaisons d'IMMÉDIATES — la chaîne littérale
# n'apparaît pas dans le binaire alors que le flag fonctionne. `strings`/`grep`
# donnent donc des faux négatifs. On exécute directement le flag (le binaire
# déployé est forcément récent via deploy-pi5.sh) avec un `timeout` en filet
# de sécurité (cf. exécution plus bas).

# Redémarrage garanti du service quoi qu'il arrive.
STOPPED=0
restart_svc() {
    [ "$STOPPED" -eq 1 ] || return 0
    log "Redémarrage de $SVC…"
    systemctl start "$SVC" || true
    for _ in $(seq 1 60); do
        systemctl is-active --quiet "$SVC" && { log "$SVC actif."; return 0; }
        systemctl is-failed --quiet "$SVC" && break
        sleep 5
    done
    err "$SVC n'est pas actif — vérifie : journalctl -u $SVC -n 40"
}
trap restart_svc EXIT INT TERM HUP

# 1. Abaisser raw_retention_days (persistant), avec backup.
CURRENT=$(awk '/^\[metrics_store\]/{s=1;next} /^\[/{s=0} s&&/^[[:space:]]*raw_retention_days[[:space:]]*=/{print $3;exit}' "$CONF" || true)
if [ "${CURRENT:-}" != "$RAW_DAYS" ]; then
    BACKUP="$CONF.before-compact-$(date +%Y%m%d_%H%M%S)"
    cp "$CONF" "$BACKUP"
    sed -i "s/^\([[:space:]]*raw_retention_days[[:space:]]*=[[:space:]]*\).*/\1$RAW_DAYS/" "$CONF"
    log "raw_retention_days : ${CURRENT:-?} → $RAW_DAYS (backup → $BACKUP)"
else
    log "raw_retention_days déjà à $RAW_DAYS"
fi

# 2. Arrêt du service (libère la base pour un accès exclusif).
log "Arrêt de $SVC (redb mono-process)…"
systemctl stop "$SVC"
STOPPED=1
sleep 3

# 3. Compaction (peut durer plusieurs minutes). `timeout` = filet de sécurité :
#    si jamais le binaire était trop ancien (flag ignoré → il démarrerait le
#    serveur au lieu de compacter et de sortir), on ne reste pas bloqué.
log "Compaction en cours (tiering + compaction physique)… patiente (≤ 30 min)."
set +e
sudo -u "$DALY_USER" timeout 1800 env DALY_CONFIG="$CONF" "$BIN" --compact-db
rc=$?
set -e
case "$rc" in
    0)   log "Compaction terminée." ;;
    124) err "Compaction interrompue (timeout 30 min) — base inchangée." ;;
    *)   err "Compaction en échec (code $rc) — service redémarré tel quel." ;;
esac

# 4. Redémarrage via le trap EXIT.
