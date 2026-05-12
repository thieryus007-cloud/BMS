#!/usr/bin/env bash
# =============================================================================
# deploy-mosquitto-native.sh
# Bascule Mosquitto Docker -> Mosquitto natif sur Pi5
# Couvre les étapes §10/§11/§12 du plan
# docs/migration-mosquitto-docker-to-native-v2.md
#
# Usage :
#   ./contrib/mosquitto/deploy-mosquitto-native.sh           # interactif
#   ./contrib/mosquitto/deploy-mosquitto-native.sh --yes     # non interactif
#   ./contrib/mosquitto/deploy-mosquitto-native.sh --prepare # config + units, pas de bascule
# =============================================================================

set -euo pipefail

# ----- Args -----
ASSUME_YES=0
PREPARE_ONLY=0
for arg in "$@"; do
    case "$arg" in
        -y|--yes)     ASSUME_YES=1 ;;
        --prepare)    PREPARE_ONLY=1 ;;
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
    local prompt="$1"
    [ "$ASSUME_YES" -eq 1 ] && return 0
    read -rp "${BOLD}${prompt}${NC} (y/N) " ans
    [ "${ans,,}" = "y" ] || [ "${ans,,}" = "yes" ]
}

# ----- Localiser le repo -----
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

TIMESTAMP="$(date +%Y%m%d_%H%M%S)"

# ----- Vérifs préalables -----
log "Vérifications préalables"
[ "$EUID" -eq 0 ] && { err "Lancer SANS sudo — le script appelle sudo lui-même"; exit 1; }
command -v sudo       >/dev/null || { err "sudo requis"; exit 1; }
command -v mosquitto  >/dev/null || { err "Mosquitto natif non installé (sudo apt install mosquitto mosquitto-clients)"; exit 1; }
[ -f contrib/mosquitto/mosquitto.conf ]      || { err "contrib/mosquitto/mosquitto.conf manquant"; exit 1; }
[ -f contrib/mosquitto-broker.service ]      || { err "contrib/mosquitto-broker.service manquant"; exit 1; }
[ -f contrib/daly-bms.service ]              || { err "contrib/daly-bms.service manquant"; exit 1; }
[ -f contrib/energy-manager.service ]        || { err "contrib/energy-manager.service manquant"; exit 1; }
[ -f contrib/mosquitto/verify-no-loop.sh ]   || { err "contrib/mosquitto/verify-no-loop.sh manquant"; exit 1; }
[ -f Config.toml ]                           || { err "Config.toml manquant à la racine du repo"; exit 1; }
ok "Tous les fichiers source présents"

# Ports déjà occupés par notre futur broker ?
if sudo systemctl is-active --quiet mosquitto.service 2>/dev/null; then
    warn "Service Debian par défaut mosquitto.service actif — désactivation"
    sudo systemctl stop mosquitto.service
    sudo systemctl disable mosquitto.service 2>/dev/null || true
fi

# ----- Annonce -----
echo
echo "${BOLD}===== Plan d'exécution =====${NC}"
echo " 1. Backups + déploiement /etc/mosquitto/mosquitto.conf"
echo " 2. mosquitto -c ... -t (vérif syntaxe)"
echo " 3. Installation verify-no-loop.sh -> /usr/local/bin/"
echo " 4. Anti-boucle (verify-no-loop.sh)"
echo " 5. Installation 3 unités systemd + daemon-reload + enable mosquitto-broker"
echo " 6. Déploiement Config.toml -> /etc/daly-bms/config.toml"
if [ "$PREPARE_ONLY" -eq 0 ]; then
    echo " 7. BASCULE : arrêt daly-bms+energy-manager+Docker -> démarrage natif"
    echo " 8. Tests pub/sub local + état bridges"
    echo " 9. Redémarrage daly-bms + energy-manager"
else
    echo "    [--prepare] : pas de bascule, services Docker laissés en l'état"
fi
echo
confirm "Démarrer ?" || { warn "Abandon"; exit 0; }

# =============================================================================
# Étape 1 — mosquitto.conf
# =============================================================================
log "[1/9] Déploiement /etc/mosquitto/mosquitto.conf"
sudo mkdir -p /etc/mosquitto /var/lib/mosquitto /var/log/mosquitto
sudo chown mosquitto:mosquitto /var/lib/mosquitto /var/log/mosquitto
sudo chmod 750 /var/lib/mosquitto
sudo chmod 755 /var/log/mosquitto

if [ -f /etc/mosquitto/mosquitto.conf ]; then
    sudo cp -a /etc/mosquitto/mosquitto.conf "/etc/mosquitto/mosquitto.conf.bak.${TIMESTAMP}"
    ok "Backup /etc/mosquitto/mosquitto.conf.bak.${TIMESTAMP}"
fi
sudo install -m 644 -o root -g root contrib/mosquitto/mosquitto.conf /etc/mosquitto/mosquitto.conf
ok "/etc/mosquitto/mosquitto.conf déployé"

# =============================================================================
# Étape 2 — Vérification syntaxe
# =============================================================================
# Mosquitto 2.0 n'a pas d'option dédiée -t. On démarre brièvement en tant
# qu'utilisateur mosquitto : une erreur de config est émise AVANT le bind
# des ports. Un "Address already in use" est attendu si Docker tient encore
# les ports — la config est alors validée.
log "[2/9] Vérification syntaxe mosquitto.conf"
VALIDATE_OUT="$(mktemp)"
sudo -u mosquitto timeout 2 mosquitto -c /etc/mosquitto/mosquitto.conf \
    >"$VALIDATE_OUT" 2>&1 || true

if grep -qE "Unknown configuration variable|Error found at|Invalid (value|bridge|listener)" "$VALIDATE_OUT"; then
    err "Syntaxe invalide :"
    sed 's/^/      /' "$VALIDATE_OUT" >&2
    rm -f "$VALIDATE_OUT"
    if [ -f "/etc/mosquitto/mosquitto.conf.bak.${TIMESTAMP}" ]; then
        sudo cp -a "/etc/mosquitto/mosquitto.conf.bak.${TIMESTAMP}" /etc/mosquitto/mosquitto.conf
        warn "Rollback vers backup ${TIMESTAMP}"
    fi
    exit 1
fi

if grep -qiE "Address already in use|Error: Unable to (bind|create)" "$VALIDATE_OUT"; then
    ok "Syntaxe OK (port 1883/9001 occupé par Docker — attendu avant bascule)"
else
    ok "Syntaxe OK"
fi
rm -f "$VALIDATE_OUT"

# =============================================================================
# Étape 3 — verify-no-loop.sh
# =============================================================================
log "[3/9] Installation verify-no-loop.sh -> /usr/local/bin/"
sudo install -m 755 -o root -g root contrib/mosquitto/verify-no-loop.sh /usr/local/bin/verify-no-loop.sh
ok "/usr/local/bin/verify-no-loop.sh installé"

# =============================================================================
# Étape 4 — Anti-boucle
# =============================================================================
log "[4/9] Vérification anti-boucle"
if sudo /usr/local/bin/verify-no-loop.sh; then
    ok "Aucun topic en double IN/OUT"
else
    err "Boucle détectée — abandon avant bascule"
    exit 1
fi

# =============================================================================
# Étape 5 — Unités systemd
# =============================================================================
log "[5/9] Déploiement unités systemd"
for unit in mosquitto-broker.service daly-bms.service energy-manager.service; do
    src="contrib/${unit}"
    dst="/etc/systemd/system/${unit}"
    [ "$unit" = "mosquitto-broker.service" ] && src="contrib/mosquitto-broker.service"
    if [ -f "$dst" ]; then
        sudo cp -a "$dst" "${dst}.bak.${TIMESTAMP}"
    fi
    sudo install -m 644 -o root -g root "$src" "$dst"
    ok "$dst"
done
sudo systemctl daemon-reload
sudo systemctl enable mosquitto-broker.service
ok "daemon-reload + enable mosquitto-broker"

# =============================================================================
# Étape 6 — Config.toml
# =============================================================================
log "[6/9] Déploiement Config.toml -> /etc/daly-bms/config.toml"
sudo mkdir -p /etc/daly-bms
if [ -f /etc/daly-bms/config.toml ]; then
    sudo cp -a /etc/daly-bms/config.toml "/etc/daly-bms/config.toml.bak.${TIMESTAMP}"
    ok "Backup /etc/daly-bms/config.toml.bak.${TIMESTAMP}"
fi
sudo cp Config.toml /etc/daly-bms/config.toml
ok "/etc/daly-bms/config.toml déployé"

if [ "$PREPARE_ONLY" -eq 1 ]; then
    echo
    ok "Phase préparation terminée (--prepare)"
    warn "Bascule non effectuée — Docker Mosquitto toujours actif"
    echo "Relancer sans --prepare pour basculer."
    exit 0
fi

# =============================================================================
# Étape 7 — Bascule (DESTRUCTIVE)
# =============================================================================
echo
echo "${BOLD}===== BASCULE — fenêtre indisponible ~10-30s =====${NC}"
echo "  - Arrêt daly-bms + energy-manager"
echo "  - docker compose down (Mosquitto)"
echo "  - Démarrage mosquitto-broker.service"
echo "  - Redémarrage daly-bms + energy-manager"
echo
confirm "Basculer maintenant ?" || { warn "Bascule annulée. Reprendre avec :"
    echo "  $0 --yes  (passera direct à la bascule car prepare déjà fait)"
    exit 0; }

log "[7/9] Arrêt daly-bms + energy-manager (s'ils tournent)"
sudo systemctl stop energy-manager 2>/dev/null || true
sudo systemctl stop daly-bms 2>/dev/null || true

if [ -f docker-compose.infra.yml ]; then
    if docker ps --format '{{.Names}}' 2>/dev/null | grep -q mosquitto; then
        log "Arrêt Docker Mosquitto"
        docker compose -f docker-compose.infra.yml down
    else
        log "Docker Mosquitto déjà arrêté"
    fi
fi

# Ports libres ?
sleep 1
if ss -tlnp 2>/dev/null | awk '{print $4}' | grep -qE ':(1883|9001)$'; then
    err "Ports 1883/9001 toujours occupés :"
    ss -tlnp | grep -E ':(1883|9001)\b'
    exit 1
fi
ok "Ports 1883/9001 libres"

log "Démarrage mosquitto-broker.service"
sudo systemctl start mosquitto-broker.service
sleep 3

if ! systemctl is-active --quiet mosquitto-broker.service; then
    err "mosquitto-broker n'a pas démarré"
    sudo journalctl -u mosquitto-broker -n 30 --no-pager
    exit 1
fi
ok "mosquitto-broker actif"

# =============================================================================
# Étape 8 — Tests
# =============================================================================
log "[8/9] Test pub/sub local"
PROBE_OUT="$(mktemp)"
( timeout 5 mosquitto_sub -h 127.0.0.1 -p 1883 -t 'test/probe' -C 1 > "$PROBE_OUT" 2>/dev/null ) &
SUB_PID=$!
sleep 1
mosquitto_pub -h 127.0.0.1 -p 1883 -t 'test/probe' -m "ping-${TIMESTAMP}"
wait "$SUB_PID" 2>/dev/null || true
if grep -q "ping-${TIMESTAMP}" "$PROBE_OUT"; then
    ok "Pub/sub local OK"
else
    warn "Pub/sub local n'a pas reçu le message en 5s — investiguer"
fi
rm -f "$PROBE_OUT"

log "État des bridges (5s d'écoute)"
timeout 5 mosquitto_sub -h 127.0.0.1 -t '$SYS/broker/bridge/+/state' -v 2>/dev/null \
    | sed 's/^/      /' || true

log "Trafic NanoPi -> Pi5 (sample 3s sur N/c0619ab9929a/#)"
NPI_COUNT=$(timeout 3 mosquitto_sub -h 127.0.0.1 -t 'N/c0619ab9929a/#' -v 2>/dev/null | wc -l || true)
if [ "${NPI_COUNT:-0}" -gt 0 ]; then
    ok "Bridge INGRESS reçoit du NanoPi (${NPI_COUNT} msg/3s)"
else
    warn "Aucun message NanoPi reçu en 3s — bridge peut-être encore en cours de connexion"
fi

# =============================================================================
# Étape 9 — Services métier
# =============================================================================
log "[9/9] Redémarrage daly-bms + energy-manager"
sudo systemctl start daly-bms
sleep 5
sudo systemctl start energy-manager
sleep 3

echo
echo "${BOLD}===== État final =====${NC}"
for svc in mosquitto-broker daly-bms energy-manager; do
    if systemctl is-active --quiet "$svc"; then
        printf "  %-22s ${GREEN}active${NC}\n" "$svc"
    else
        printf "  %-22s ${RED}INACTIF${NC}\n" "$svc"
    fi
done

# Docker toujours là ?
if docker ps --format '{{.Names}}' 2>/dev/null | grep -q mosquitto; then
    warn "Conteneur mosquitto encore présent — anormal"
else
    ok "Aucun conteneur mosquitto actif"
fi

echo
ok "Migration appliquée. Surveiller :"
echo "  journalctl -u mosquitto-broker -u daly-bms -u energy-manager -f"
echo "  mosquitto_sub -h 127.0.0.1 -t 'santuario/#' -v   # trafic local"
echo "  VRM : https://vrm.victronenergy.com  (tous devices 'just now')"
echo
echo "Rollback en cas de problème : voir §15 du plan."
