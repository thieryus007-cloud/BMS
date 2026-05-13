#!/usr/bin/env bash
# =============================================================================
# cleanup-docker.sh
# Nettoyage post-migration : retire les artefacts Docker Mosquitto sur Pi5
# (conteneur dalybms-mosquitto, volumes, image eclipse-mosquitto).
#
# Prérequis : mosquitto-broker.service natif déjà actif et stable depuis >24h.
# Ce script REFUSE de tourner si le service natif n'est pas actif.
#
# Usage :
#   ./contrib/mosquitto/cleanup-docker.sh                  # interactif
#   ./contrib/mosquitto/cleanup-docker.sh --yes            # non interactif
#   ./contrib/mosquitto/cleanup-docker.sh --purge-daemon   # va plus loin : désactive docker.service
# =============================================================================

set -euo pipefail

ASSUME_YES=0
PURGE_DAEMON=0
for arg in "$@"; do
    case "$arg" in
        -y|--yes)        ASSUME_YES=1 ;;
        --purge-daemon)  PURGE_DAEMON=1 ;;
        -h|--help)
            sed -n '2,15p' "$0"
            exit 0
            ;;
        *) echo "Arg inconnu : $arg" ; exit 2 ;;
    esac
done

# ----- Couleurs -----
if [ -t 1 ]; then
    RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'
    BLUE=$'\033[34m'; BOLD=$'\033[1m'; NC=$'\033[0m'
else
    RED='' GREEN='' YELLOW='' BLUE='' BOLD='' NC=''
fi
log()  { echo "${BLUE}[INFO]${NC} $*"; }
ok()   { echo "${GREEN}[ OK ]${NC} $*"; }
warn() { echo "${YELLOW}[WARN]${NC} $*"; }
err()  { echo "${RED}[ERR ]${NC} $*" >&2; }

confirm() {
    [ "$ASSUME_YES" -eq 1 ] && return 0
    read -rp "${BOLD}$1${NC} (y/N) " ans
    [ "${ans,,}" = "y" ] || [ "${ans,,}" = "yes" ]
}

# ----- Pré-vérifications de sûreté -----
log "Vérifications préalables"
[ "$EUID" -eq 0 ] && { err "Lancer SANS sudo — le script appelle sudo lui-même"; exit 1; }
command -v docker >/dev/null || { warn "docker absent — rien à nettoyer côté conteneurs"; }

if ! systemctl is-active --quiet mosquitto-broker.service 2>/dev/null; then
    err "mosquitto-broker.service n'est PAS actif — abandon"
    err "Le broker natif doit fonctionner avant de supprimer Docker"
    exit 1
fi
ok "mosquitto-broker.service actif"

# Port 1883 bien tenu par le natif ?
if command -v ss >/dev/null && ss -tlnp 2>/dev/null | grep -q ':1883 .*mosquitto'; then
    ok "Port 1883 tenu par mosquitto natif"
else
    warn "Impossible de confirmer que le port 1883 est tenu par mosquitto natif"
    confirm "Continuer quand même ?" || { warn "Abandon"; exit 0; }
fi

# ----- Plan -----
echo
echo "${BOLD}===== Plan de nettoyage Docker =====${NC}"
echo " 1. Arrêter + supprimer le conteneur dalybms-mosquitto"
echo " 2. Supprimer les volumes mosquitto-data et mosquitto-log"
echo " 3. Supprimer l'image eclipse-mosquitto:*"
if [ "$PURGE_DAEMON" -eq 1 ]; then
    echo " 4. [--purge-daemon] systemctl stop+disable docker.service"
    echo "    (le paquet docker reste installé — désinstaller via apt si besoin)"
fi
echo
confirm "Démarrer ?" || { warn "Abandon"; exit 0; }

# ----- 1. Conteneur -----
if command -v docker >/dev/null; then
    log "[1/3] Conteneur dalybms-mosquitto"
    if docker ps -a --format '{{.Names}}' 2>/dev/null | grep -qx 'dalybms-mosquitto'; then
        docker stop dalybms-mosquitto >/dev/null 2>&1 || true
        docker rm   dalybms-mosquitto >/dev/null 2>&1 || true
        ok "Conteneur supprimé"
    else
        ok "Aucun conteneur dalybms-mosquitto"
    fi

    # Tentative compose si le YAML est encore là (compat ancienne checkout)
    if [ -f docker-compose.infra.yml ]; then
        docker compose -f docker-compose.infra.yml down -v >/dev/null 2>&1 || true
    fi

    # ----- 2. Volumes -----
    log "[2/3] Volumes Docker mosquitto"
    REMOVED=0
    for vol in $(docker volume ls --format '{{.Name}}' 2>/dev/null | grep -E '(^|_)mosquitto-(data|log)$' || true); do
        docker volume rm "$vol" >/dev/null 2>&1 && { ok "Volume $vol supprimé"; REMOVED=$((REMOVED+1)); } || warn "Échec suppression $vol"
    done
    [ "$REMOVED" -eq 0 ] && ok "Aucun volume mosquitto à supprimer"

    # ----- 3. Image -----
    log "[3/3] Image eclipse-mosquitto"
    IMGS=$(docker images --format '{{.Repository}}:{{.Tag}}' 2>/dev/null | grep '^eclipse-mosquitto:' || true)
    if [ -n "$IMGS" ]; then
        echo "$IMGS" | while read -r img; do
            docker rmi "$img" >/dev/null 2>&1 && ok "Image $img supprimée" || warn "Échec suppression $img"
        done
    else
        ok "Aucune image eclipse-mosquitto"
    fi
else
    warn "docker absent — étapes 1-3 ignorées"
fi

# ----- 4. (Optionnel) docker.service -----
if [ "$PURGE_DAEMON" -eq 1 ]; then
    log "[4/4] Désactivation docker.service"
    OTHER=$(docker ps -q 2>/dev/null | wc -l || echo 0)
    if [ "$OTHER" -gt 0 ]; then
        err "$OTHER conteneur(s) encore actif(s) — refus de désactiver docker.service"
        docker ps
        exit 1
    fi
    sudo systemctl stop    docker.service docker.socket 2>/dev/null || true
    sudo systemctl disable docker.service docker.socket 2>/dev/null || true
    ok "docker.service arrêté + désactivé"
    echo "    Pour désinstaller complètement : sudo apt remove --purge docker.io docker-ce docker-compose-plugin"
fi

echo
ok "Nettoyage terminé"
echo "Vérifier l'état :"
echo "  systemctl status mosquitto-broker --no-pager | head"
echo "  ss -tlnp | grep -E ':(1883|9001) '"
[ "$PURGE_DAEMON" -eq 0 ] && echo "  (Docker daemon laissé en place — relancer avec --purge-daemon pour le désactiver)"
