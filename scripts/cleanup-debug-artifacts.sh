#!/usr/bin/env bash
# cleanup-debug-artifacts.sh — Retire tous les artefacts de test/debug du Pi5.
#
# À lancer sur le Pi5 après la fin de l'investigation fuite mémoire
# (mai 2026). Ne touche ni au code source, ni aux DB de prod.
#
# Usage :
#   bash scripts/cleanup-debug-artifacts.sh             # mode interactif (confirme)
#   bash scripts/cleanup-debug-artifacts.sh --force     # supprime sans confirmer
#
# Ce que le script supprime (côté Pi5) :
#   - Drop-ins systemd de test :
#       /etc/systemd/system/daly-bms.service.d/disable-monitor.conf
#       /etc/systemd/system/daly-bms.service.d/heaptrack.conf
#   - Fichiers heaptrack résiduels :
#       /var/lib/daly-bms/heaptrack-daly*
#       /tmp/heaptrack_fifo*
#       /tmp/heaptrack-report.txt
#   - Configs / logs / DB de test :
#       /tmp/daly-valgrind.toml
#       /tmp/valgrind*.log /tmp/valgrind*.txt
#       /tmp/valgrind-metrics.redb /tmp/valgrind-alerts.db
#       /tmp/rss-*.log /tmp/jestats-*.json
#   - Backups Config.toml automatiques générés par scripts deploy/test :
#       /etc/daly-bms/config.toml.bak*
#       /etc/daly-bms/config.toml.before-autorepair-*
#
# Ce que le script CONSERVE (volontairement) :
#   - Le service file principal (/etc/systemd/system/daly-bms.service)
#     avec RuntimeMaxSec=86400 et tuning jemalloc — filet de sécurité.
#   - La règle polkit /etc/polkit-1/rules.d/50-dalybms-systemd-restart.rules
#   - La config /etc/daly-bms/config.toml en cours d'utilisation.
#   - Les DB de prod /var/lib/daly-bms/alerts.db, dashboards.db et
#     /mnt/nvme/daly-bms/metrics.redb.
#   - Le code source du repo : flags `DALY_DISABLE_MONITOR` et
#     `DALY_DISABLE_RS485` conservés (inactifs par défaut).
#   - Le script scripts/valgrind-leak-hunt.sh (utile pour futures
#     investigations si la pente remonte).

set -uo pipefail

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; CYAN='\033[0;36m'; NC='\033[0m'
info()  { echo -e "${GREEN}[OK]${NC} $*"; }
step()  { echo -e "${CYAN}[>>]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!!]${NC} $*"; }

FORCE=false
[[ "${1:-}" == "--force" ]] && FORCE=true

# Liste des paths à nettoyer (un par ligne pour visibilité).
PATHS_TO_REMOVE=(
    /etc/systemd/system/daly-bms.service.d/disable-monitor.conf
    /etc/systemd/system/daly-bms.service.d/heaptrack.conf
    /var/lib/daly-bms/heaptrack-daly*
    /tmp/heaptrack_fifo*
    /tmp/heaptrack-report.txt
    /tmp/heaptrack-daly*
    /tmp/daly-valgrind.toml
    /tmp/main.rs.before-valgrind
    /tmp/valgrind.log
    /tmp/valgrind-summary.txt
    /tmp/valgrind-metrics.redb
    /tmp/valgrind-alerts.db
    /tmp/rss-*.log
    /tmp/jestats-*.json
    /tmp/test-config-awk.toml
    /etc/daly-bms/config.toml.bak*
    /etc/daly-bms/config.toml.before-autorepair-*
)

# Détection des fichiers/dossiers existants à supprimer.
step "Recherche des artefacts à nettoyer…"
FOUND=()
for pattern in "${PATHS_TO_REMOVE[@]}"; do
    # shellcheck disable=SC2086
    for path in $pattern; do
        [[ -e "$path" ]] && FOUND+=("$path")
    done
done

if [[ ${#FOUND[@]} -eq 0 ]]; then
    info "Aucun artefact de test trouvé — le Pi5 est déjà propre."
    exit 0
fi

echo
warn "Artefacts détectés (${#FOUND[@]}) :"
for f in "${FOUND[@]}"; do
    SIZE=$(sudo du -sh "$f" 2>/dev/null | cut -f1 || echo "?")
    echo "  $SIZE  $f"
done
echo

if ! $FORCE; then
    read -rp "Supprimer tous ces fichiers ? [o/N] " ANSWER
    if [[ "$ANSWER" != "o" && "$ANSWER" != "O" && "$ANSWER" != "y" && "$ANSWER" != "Y" ]]; then
        warn "Annulé par l'utilisateur."
        exit 0
    fi
fi

step "Suppression…"
for f in "${FOUND[@]}"; do
    sudo rm -rf "$f"
done
info "${#FOUND[@]} artefacts supprimés"

# Rechargement systemd si des drop-ins ont été retirés.
if [[ " ${FOUND[*]} " == *"/etc/systemd/system/daly-bms.service.d/"* ]]; then
    step "Rechargement systemd (drop-ins retirés)…"
    sudo systemctl daemon-reload
    info "daemon-reload effectué"

    # Si le service tourne, le restarter proprement pour qu'il prenne en compte
    # le retrait des drop-ins.
    if systemctl is-active --quiet daly-bms; then
        step "Restart daly-bms pour prise en compte…"
        sudo systemctl restart daly-bms
        sleep 3
        if systemctl is-active --quiet daly-bms; then
            info "daly-bms actif"
        else
            warn "daly-bms ne démarre pas — vérifier journalctl -u daly-bms"
        fi
    fi
fi

echo
echo -e "${GREEN}═══════════════════════════════════════${NC}"
echo -e "${GREEN}  Cleanup terminé ✓${NC}"
echo -e "${GREEN}═══════════════════════════════════════${NC}"
echo
echo "État conservé (filet de sécurité et outils diagnostic) :"
echo "  - /etc/systemd/system/daly-bms.service (RuntimeMaxSec=86400)"
echo "  - /etc/polkit-1/rules.d/50-dalybms-systemd-restart.rules"
echo "  - /etc/daly-bms/config.toml (config prod)"
echo "  - Code repo : flags DALY_DISABLE_* (inactifs par défaut)"
echo "  - scripts/valgrind-leak-hunt.sh"
echo
echo "Pour réinvestiguer la fuite mémoire dans le futur :"
echo "  bash scripts/valgrind-leak-hunt.sh 600 --full"
