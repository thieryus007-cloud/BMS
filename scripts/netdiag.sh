#!/usr/bin/env bash
# netdiag.sh — capture le coupable d'un pic de trafic réseau sortant sur le Pi5.
#
# Le débit mesuré ici reproduit EXACTEMENT celui de la page monitoring du
# serveur (`pi5_net_tx_bps`) : somme des octets/s TX de toutes les interfaces
# non-loopback (/proc/net/dev). Aucun outil externe requis (ss/iproute2 +
# coreutils suffisent ; iftop/nethogs facultatifs).
#
# Usage :
#   sudo bash scripts/netdiag.sh                 # 1 capture immédiate (~10 s)
#   sudo bash scripts/netdiag.sh --once 15       # 1 capture sur 15 s
#   sudo bash scripts/netdiag.sh --watch         # veille : capture auto au-delà du seuil
#   sudo bash scripts/netdiag.sh --watch 150000 5  # seuil 150 Ko/s, poll 5 s
#
# En mode --watch, dès que le TX dépasse le seuil (octets/s) un rapport
# horodaté est écrit dans /tmp/netdiag-<date>.txt, puis cooldown 60 s.
set -u

INTERVAL_DEFAULT=10
THRESHOLD_DEFAULT=150000   # octets/s (~150 Ko/s) — bien au-dessus du repos
POLL_DEFAULT=5
COOLDOWN=60

# ── lecture des compteurs réseau (mêmes règles que monitor.rs) ───────────────
read_tx_total() {
    local tx=0
    while IFS= read -r line; do
        case "$line" in *:*) : ;; *) continue ;; esac
        local iface="${line%%:*}"; iface="${iface// /}"
        [ "$iface" = "lo" ] && continue
        # data après le ':' ; tx_bytes = 9e champ après rx (rx=1, +7 skip, tx=9)
        local data="${line#*:}"
        # shellcheck disable=SC2206
        local f=($data)
        # f[0]=rx_bytes ... f[8]=tx_bytes (0-indexé)
        tx=$(( tx + ${f[8]:-0} ))
    done < /proc/net/dev
    echo "$tx"
}

per_iface_rates() {
    local secs="$1"
    declare -A before
    while IFS= read -r line; do
        case "$line" in *:*) : ;; *) continue ;; esac
        local iface="${line%%:*}"; iface="${iface// /}"
        [ "$iface" = "lo" ] && continue
        local data="${line#*:}"; local f=($data)
        before["$iface"]="${f[8]:-0}"
    done < /proc/net/dev
    sleep "$secs"
    while IFS= read -r line; do
        case "$line" in *:*) : ;; *) continue ;; esac
        local iface="${line%%:*}"; iface="${iface// /}"
        [ "$iface" = "lo" ] && continue
        local data="${line#*:}"; local f=($data)
        local now="${f[8]:-0}"; local b="${before[$iface]:-0}"
        printf '  %-8s %10d B/s\n' "$iface" $(( (now - b) / secs ))
    done < /proc/net/dev
}

# ── attribution par socket (qui envoie ?) ────────────────────────────────────
ss_snapshot() {
    # local|peer|proc|bytes_sent
    ss -tniHOp state established 2>/dev/null | awk '
        {
            local=$4; peer=$5; proc="-"; bs="0";
            if (match($0, /users:\(\("[^"]+"/)) proc=substr($0,RSTART+9,RLENGTH-10);
            if (match($0, /bytes_sent:[0-9]+/)) bs=substr($0,RSTART+11,RLENGTH-11);
            print local"|"peer"|"proc"|"bs;
        }'
}

label_port() {
    case "${1##*:}" in
        8080) echo "daly-bms (API/WS)";;
        8081) echo "energy-manager";;
        3000) echo "grafana";;
        1883|9001) echo "mosquitto";;
        *) echo "";;
    esac
}

capture() {
    local interval="${1:-$INTERVAL_DEFAULT}"
    local ts; ts="$(date '+%Y-%m-%d %H:%M:%S')"
    echo "════════════════════════════════════════════════════════════"
    echo " netdiag — capture du $ts  (fenêtre ${interval}s)"
    echo "════════════════════════════════════════════════════════════"

    # 1) débit global (= métrique serveur) + par interface
    local t1 t2
    t1="$(read_tx_total)"
    local s1; s1="$(ss_snapshot)"
    sleep "$interval"
    t2="$(read_tx_total)"
    local s2; s2="$(ss_snapshot)"
    local rate=$(( (t2 - t1) / interval ))
    echo
    echo "▶ TX total (toutes ifaces non-lo) : ${rate} B/s  (~$(( rate / 1024 )) Ko/s, $(awk "BEGIN{printf \"%.2f\", $rate*8/1000000}") Mbit/s)"
    echo
    echo "▶ Débit TX par interface :"
    # recalcul rapide par iface sur 2 s (indicatif)
    per_iface_rates 2

    # 2) top sockets par octets envoyés sur la fenêtre
    echo
    echo "▶ Top connexions par octets ENVOYÉS sur la fenêtre :"
    printf '%12s  %-22s  %-22s  %s\n' "B/s" "local" "pair" "process"
    paste -d'\n' <(echo "$s1") <(echo "$s2") >/dev/null 2>&1 || true
    awk -F'|' -v secs="$interval" '
        NR==FNR { a[$1"|"$2]=$4; next }
        {
            k=$1"|"$2; prev=(k in a)?a[k]:0; d=$4-prev;
            if (d>0) printf "%d\t%s\t%s\t%s\n", d/secs, $1, $2, $3;
        }' <(echo "$s1") <(echo "$s2") | sort -rn | head -12 | \
    while IFS=$'\t' read -r bps loc peer proc; do
        local tag; tag="$(label_port "$loc")"
        [ -n "$tag" ] && proc="$proc  [$tag]"
        printf '%12s  %-22s  %-22s  %s\n' "$bps" "$loc" "$peer" "$proc"
    done

    # 3) nombre de connexions établies par port local (détecte un afflux de clients WS)
    echo
    echo "▶ Connexions établies par port local (count) :"
    ss -tnH state established 2>/dev/null | awk '{print $4}' | sed 's/.*://' \
        | sort | uniq -c | sort -rn | head -10 | \
        while read -r c p; do printf '  %4s  :%-6s %s\n' "$c" "$p" "$(label_port ":$p")"; done

    # 4) top process CPU (un pic réseau s'accompagne souvent d'un pic CPU)
    echo
    echo "▶ Top process CPU :"
    ps -eo pid,comm,%cpu,%mem --sort=-%cpu 2>/dev/null | head -8 | sed 's/^/  /'

    # 5) MQTT broker (octets/s sortants, si mosquitto_sub dispo)
    if command -v mosquitto_sub >/dev/null 2>&1; then
        echo
        echo "▶ MQTT broker \$SYS (1 min) :"
        for t in 'load/bytes/sent/1min' 'load/messages/sent/1min' 'clients/connected'; do
            v="$(timeout 3 mosquitto_sub -h 127.0.0.1 -t "\$SYS/broker/$t" -C 1 2>/dev/null)"
            printf '  %-26s %s\n' "$t" "${v:-n/a}"
        done
    fi

    # 6) logs récents daly-bms (indices : flood WS, requêtes, erreurs)
    echo
    echo "▶ daly-bms — 20 dernières lignes :"
    journalctl -u daly-bms -n 20 --no-pager 2>/dev/null | sed 's/^/  /'
    echo "════════════════════════════════════════════════════════════"
}

watch_mode() {
    local threshold="${1:-$THRESHOLD_DEFAULT}"
    local poll="${2:-$POLL_DEFAULT}"
    echo "netdiag --watch : seuil ${threshold} B/s, poll ${poll}s. Ctrl-C pour arrêter."
    local prev; prev="$(read_tx_total)"
    while true; do
        sleep "$poll"
        local now; now="$(read_tx_total)"
        local rate=$(( (now - prev) / poll ))
        prev="$now"
        printf '%s  TX=%d B/s\n' "$(date '+%H:%M:%S')" "$rate"
        if [ "$rate" -ge "$threshold" ]; then
            local out="/tmp/netdiag-$(date '+%Y%m%d-%H%M%S').txt"
            echo ">>> PIC détecté (${rate} B/s) — capture → $out"
            capture "$INTERVAL_DEFAULT" | tee "$out"
            echo ">>> cooldown ${COOLDOWN}s"
            sleep "$COOLDOWN"
            prev="$(read_tx_total)"
        fi
    done
}

case "${1:-}" in
    --watch) shift; watch_mode "${1:-}" "${2:-}";;
    --once)  shift; capture "${1:-$INTERVAL_DEFAULT}";;
    "")      capture "$INTERVAL_DEFAULT";;
    *)       capture "${1}";;
esac
