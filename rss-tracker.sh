#!/bin/bash
# rss-tracker.sh — Tracker mémoire daly-bms-server toutes les 5 minutes
# Usage: ./rss-tracker.sh [PID] [interval_sec]

PID="${1:-$(pgrep -f 'daly-bms-server$' | head -1)}"
INTERVAL="${2:-300}"  # 5 minutes par défaut
LOGFILE="/tmp/rss-leak-$(date +%Y%m%d-%H%M).log"

if [ -z "$PID" ]; then
    echo "❌ daly-bms-server non trouvé. Spécifie le PID en argument."
    echo "Usage: $0 [PID] [interval_sec]"
    exit 1
fi

echo "📊 Tracker démarré — PID=$PID — intervalle=${INTERVAL}s"
echo "📁 Log: $LOGFILE"
echo "Appuyez sur Ctrl+C pour arrêter."
echo ""

# Header CSV
echo "timestamp,VmRSS_kb,RssAnon_kb,VmPeak_kb,VmSize_kb,RssFile_kb" > "$LOGFILE"

while true; do
    if ! kill -0 "$PID" 2>/dev/null; then
        echo "$(date -Iseconds) ❌ Processus $PID mort — arrêt."
        break
    fi

    TS=$(date -Iseconds)
    STATUS="/proc/$PID/status"
    
    # Extraction des valeurs en kB
    VMRSS=$(awk '/^VmRSS/ {print $2}' "$STATUS")
    RSSANON=$(awk '/^RssAnon/ {print $2}' "$STATUS")
    VMPEAK=$(awk '/^VmPeak/ {print $2}' "$STATUS")
    VMSIZE=$(awk '/^VmSize/ {print $2}' "$STATUS")
    RSSFILE=$(awk '/^RssFile/ {print $2}' "$STATUS")
    
    # Fallback si champ absent (RssAnon/RssFile sur certains noyaux)
    VMRSS="${VMRSS:-0}"
    RSSANON="${RSSANON:-0}"
    VMPEAK="${VMPEAK:-0}"
    VMSIZE="${VMSIZE:-0}"
    RSSFILE="${RSSFILE:-0}"

    # Log CSV
    echo "$TS,$VMRSS,$RSSANON,$VMPEAK,$VMSIZE,$RSSFILE" >> "$LOGFILE"
    
    # Console — format lisible
    printf "[%s] RSS=%6s kB | Anon=%6s kB | Peak=%7s kB | ΔPeak=%+6s kB\n" \
        "$TS" \
        "$VMRSS" \
        "$RSSANON" \
        "$VMPEAK" \
        "$((VMPEAK - VMRSS))"

    sleep "$INTERVAL"
done
