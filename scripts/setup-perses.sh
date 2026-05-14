#!/usr/bin/env bash
#
# setup-perses.sh
# ----------------------------------------------------------------------------
# Installation et configuration complète de Perses sur Raspberry Pi 5 (aarch64)
# pour le monitoring PV solaire avec VictoriaMetrics.
#
# Cible : Raspberry Pi 5 — Raspberry Pi OS 64 bits
# Lancer via : bash scripts/setup-perses.sh
#
# Fonctionnalités :
#   - Télécharge et installe le binaire officiel ARM64
#   - Installe les plugins (plugins-archive/) requis par Perses 0.50+
#   - Crée un service systemd sur le port 8090 (8080 déjà pris par daly-bms)
#   - Provisionne automatiquement la datasource VictoriaMetrics (mode proxy LAN)
#   - Provisionne le dashboard PV Solaire (format Perses natif YAML)
#   - Compatible NVMe pour les données
#   - Idempotent et avec option --uninstall
#   - Coexiste avec Grafana (port 3000) pour une phase d'essai parallèle
#
# Options :
#   --nvme             stocke les données Perses sur /mnt/nvme/perses
#   --data-path=PATH   chemin custom pour les données
#   --port=N           port d'écoute (défaut 8090)
#   --vm-url=URL       URL VictoriaMetrics (défaut http://127.0.0.1:8428)
#   --version=X.Y.Z    version Perses à installer (défaut : latest)
#   --no-firewall      désactive l'ajout de règle UFW
#   --uninstall        désinstalle Perses
#
# Exemples :
#   bash scripts/setup-perses.sh
#   bash scripts/setup-perses.sh --nvme
#   bash scripts/setup-perses.sh --port=8091 --vm-url=http://127.0.0.1:8428
#   bash scripts/setup-perses.sh --version=0.49.0 --nvme
#   sudo bash scripts/setup-perses.sh --uninstall
# ----------------------------------------------------------------------------

set -euo pipefail

# ── Couleurs ──────────────────────────────────────────────────────────────────
GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; BLUE='\033[0;34m'; NC='\033[0m'
info()  { echo -e "${GREEN}[OK]${NC} $*"; }
step()  { echo -e "${BLUE}[>>]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!!]${NC} $*"; }
error() { echo -e "${RED}[!!]${NC} $*" >&2; exit 1; }

# ── Paramètres par défaut ─────────────────────────────────────────────────────
PERSES_PORT="8090"
VM_URL="http://127.0.0.1:8428"
PERSES_DATA_PATH=""          # vide = /var/lib/perses
PERSES_VERSION=""            # vide = latest
SKIP_FIREWALL=false
UNINSTALL=false
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.."; pwd)"
PERSES_PROVISION_SRC="${REPO_ROOT}/contrib/perses"

# ── Parsing args ──────────────────────────────────────────────────────────────
for arg in "$@"; do
    case "$arg" in
        --nvme)            PERSES_DATA_PATH="/mnt/nvme/perses" ;;
        --no-firewall)     SKIP_FIREWALL=true ;;
        --uninstall)       UNINSTALL=true ;;
        --port=*)          PERSES_PORT="${arg#*=}" ;;
        --vm-url=*)        VM_URL="${arg#*=}" ;;
        --data-path=*)     PERSES_DATA_PATH="${arg#*=}" ;;
        --version=*)       PERSES_VERSION="${arg#*=}" ;;
        -h|--help)
            sed -n '3,37p' "$0"
            exit 0
            ;;
        *) error "Option inconnue: $arg (essayez --help)" ;;
    esac
done

# Résolution chemins
PERSES_DB_PATH="${PERSES_DATA_PATH:-/var/lib/perses}"
PERSES_PROV_DIR="/etc/perses/provisioning"
PERSES_PLUGINS_DIR="/etc/perses/plugins-archive"

# ── Sudo ──────────────────────────────────────────────────────────────────────
if [[ $EUID -ne 0 ]]; then
    SUDO="sudo"
else
    SUDO=""
fi

# ── Vérifications préalables ──────────────────────────────────────────────────
ARCH="$(dpkg --print-architecture 2>/dev/null || uname -m)"
if [[ "$ARCH" != "arm64" && "$ARCH" != "aarch64" ]]; then
    warn "Architecture détectée: $ARCH — non ARM64, le binaire arm64 peut ne pas fonctionner"
fi

if [[ "$PERSES_DATA_PATH" == /mnt/nvme* ]]; then
    mountpoint -q /mnt/nvme 2>/dev/null \
        || error "/mnt/nvme n'est pas monté. Monte le SSD NVMe avant d'installer Perses."
    info "NVMe monté — données Perses → $PERSES_DATA_PATH"
fi

# ── Désinstallation ───────────────────────────────────────────────────────────
if $UNINSTALL; then
    step "Désinstallation de Perses…"
    $SUDO systemctl stop perses 2>/dev/null || true
    $SUDO systemctl disable perses 2>/dev/null || true
    $SUDO rm -f /usr/local/bin/perses /usr/local/bin/percli
    $SUDO rm -f /etc/systemd/system/perses.service
    $SUDO rm -rf /etc/perses
    $SUDO rm -rf "$PERSES_DB_PATH"
    $SUDO systemctl daemon-reload
    info "Perses désinstallé"
    exit 0
fi

# ── 1. Résolution de la version ───────────────────────────────────────────────
step "Résolution de la version Perses…"

if [[ -z "$PERSES_VERSION" ]]; then
    LATEST_TAG=$(curl -sf https://api.github.com/repos/perses/perses/releases/latest \
        | grep '"tag_name"' | cut -d '"' -f 4)
    [[ -z "$LATEST_TAG" ]] && error "Impossible de récupérer la dernière version (vérifier la connexion)"
    PERSES_VERSION="${LATEST_TAG#v}"
    info "Dernière version disponible : $PERSES_VERSION"
else
    info "Version demandée : $PERSES_VERSION"
fi

# Vérifier si déjà installé à la bonne version (idempotence)
if command -v perses >/dev/null 2>&1; then
    INSTALLED_VER=$(perses --version 2>/dev/null | grep -oP '\d+\.\d+\.\d+' | head -1 || echo "")
    if [[ "$INSTALLED_VER" == "$PERSES_VERSION" ]]; then
        info "Perses $PERSES_VERSION déjà installé — passage à la configuration"
    else
        [[ -n "$INSTALLED_VER" ]] && info "Mise à jour $INSTALLED_VER → $PERSES_VERSION"
    fi
fi

# ── 2. Téléchargement, installation binaires + plugins ──────────────────────────
step "Téléchargement de Perses ${PERSES_VERSION} (ARM64)…"

EXTRACT_DIR="/tmp/perses-extract-$$"
mkdir -p "$EXTRACT_DIR"

wget -q \
    "https://github.com/perses/perses/releases/download/v${PERSES_VERSION}/perses_${PERSES_VERSION}_linux_arm64.tar.gz" \
    -O "$EXTRACT_DIR/perses.tar.gz" \
    || error "Téléchargement échoué — vérifier la version $PERSES_VERSION sur github.com/perses/perses/releases"

tar xzf "$EXTRACT_DIR/perses.tar.gz" -C "$EXTRACT_DIR"

# Binaires
PERSES_BIN=$(find "$EXTRACT_DIR" -name "perses" -type f ! -name "*.tar.gz" | head -1)
PERCLI_BIN=$(find "$EXTRACT_DIR" -name "percli" -type f | head -1)
[[ -z "$PERSES_BIN" ]] && error "Binaire 'perses' introuvable dans l'archive"

$SUDO install -m 0755 "$PERSES_BIN" /usr/local/bin/perses
[[ -n "$PERCLI_BIN" ]] && $SUDO install -m 0755 "$PERCLI_BIN" /usr/local/bin/percli \
    || warn "'percli' absent de l'archive"

info "Perses ${PERSES_VERSION} installé (/usr/local/bin/)"

# Plugins (plugins-archive/) — Perses 0.50+ charge les plugins depuis le WorkingDirectory.
# Structure dans l'archive : plugins-archive/*.tar.gz (Prometheus, StatChart, etc.)
$SUDO mkdir -p /etc/perses
PLUGINS_SRC=$(find "$EXTRACT_DIR" -type d -name "plugins-archive" | head -1)
if [[ -n "$PLUGINS_SRC" ]]; then
    $SUDO mkdir -p "$PERSES_PLUGINS_DIR"
    $SUDO cp -r "$PLUGINS_SRC"/. "$PERSES_PLUGINS_DIR"/
    PLUGIN_COUNT=$(ls "$PERSES_PLUGINS_DIR"/*.tar.gz 2>/dev/null | wc -l)
    info "$PLUGIN_COUNT plugins installés dans $PERSES_PLUGINS_DIR/"
else
    warn "Dossier 'plugins-archive' non trouvé dans l'archive"
fi

rm -rf "$EXTRACT_DIR"

# Détecter le flag d'écoute
LISTEN_FLAG=""
if perses --help 2>&1 | grep -q 'web.listen-address'; then
    LISTEN_FLAG="--web.listen-address=:${PERSES_PORT}"
elif perses --help 2>&1 | grep -q '\--addr'; then
    LISTEN_FLAG="--addr=:${PERSES_PORT}"
else
    warn "Flag d'écoute inconnu — Perses écoutera sur son port par défaut"
fi

# ── 3. Configuration (minimale — Perses 0.50+ gère les plugins automatiquement) ────────
step "Configuration Perses…"

$SUDO mkdir -p "$PERSES_PROV_DIR" "${PERSES_DB_PATH}/db"

$SUDO tee /etc/perses/config.yaml > /dev/null <<PERSES_CFG
database:
  file:
    folder: ${PERSES_DB_PATH}/db
    extension: json

provisioning:
  interval: 1m
  folders:
    - ${PERSES_PROV_DIR}
PERSES_CFG

info "Configuration créée : /etc/perses/config.yaml"

# ── 4. Fichiers de provisioning ───────────────────────────────────────────────
step "Provisioning des ressources Perses…"

# 4a. Projet "default" (requis avant tout dashboard)
$SUDO tee "${PERSES_PROV_DIR}/project-default.yaml" > /dev/null <<PROJ
kind: Project
metadata:
  name: default
spec: {}
PROJ

# 4b. Datasource VictoriaMetrics
# Plugin kind = "Prometheus" (nom du plugin dans plugins-archive/Prometheus-*.tar.gz)
$SUDO tee "${PERSES_PROV_DIR}/victoriametrics-datasource.yaml" > /dev/null <<DS_CFG
kind: GlobalDatasource
metadata:
  name: victoriametrics
spec:
  default: true
  plugin:
    kind: Prometheus
    spec:
      proxy:
        kind: HTTPProxy
        spec:
          url: "${VM_URL}"
          allowedEndpoints:
            - endpointPattern: "/api/v1/.*"
              method: GET
            - endpointPattern: "/api/v1/.*"
              method: POST
DS_CFG

# 4c. Dashboard PV Solaire
DASHBOARD_SRC="${PERSES_PROVISION_SRC}/dashboards/pv-solar-5y.yaml"
if [[ -f "$DASHBOARD_SRC" ]]; then
    $SUDO cp "$DASHBOARD_SRC" "${PERSES_PROV_DIR}/pv-solar-5y.yaml"
    info "Dashboard PV Solaire copié dans ${PERSES_PROV_DIR}/"
else
    warn "Dashboard non trouvé : $DASHBOARD_SRC"
fi

info "Ressources provisionnées dans ${PERSES_PROV_DIR}/"

# ── 5. Service systemd ────────────────────────────────────────────────────────
step "Création du service systemd perses…"

if id "pi5compute" &>/dev/null; then
    SERVICE_USER="pi5compute"
elif [[ -n "${SUDO_USER:-}" ]] && id "${SUDO_USER}" &>/dev/null; then
    SERVICE_USER="$SUDO_USER"
else
    SERVICE_USER="$(logname 2>/dev/null || echo pi)"
    warn "Utilisateur pi5compute non trouvé — service lancé sous $SERVICE_USER"
fi

EXEC_CMD="/usr/local/bin/perses --config /etc/perses/config.yaml"
[[ -n "$LISTEN_FLAG" ]] && EXEC_CMD="$EXEC_CMD $LISTEN_FLAG"

# WorkingDirectory=/etc/perses : Perses charge plugins-archive/ depuis ce dossier.
$SUDO tee /etc/systemd/system/perses.service > /dev/null <<SYSTEMD_UNIT
[Unit]
Description=Perses Monitoring Dashboard (essai parallèle a Grafana)
Documentation=https://perses.dev
After=network.target
Wants=victoriametrics.service

[Service]
Type=simple
User=${SERVICE_USER}
ExecStart=${EXEC_CMD}
Restart=always
RestartSec=5
LimitNOFILE=65535
WorkingDirectory=/etc/perses
StandardOutput=journal
StandardError=journal
SyslogIdentifier=perses

[Install]
WantedBy=multi-user.target
SYSTEMD_UNIT

$SUDO chown -R "${SERVICE_USER}:${SERVICE_USER}" /etc/perses "$PERSES_DB_PATH" 2>/dev/null || true

$SUDO systemctl daemon-reload
$SUDO systemctl enable --now perses

info "Service perses activé (User=${SERVICE_USER}, WorkingDirectory=/etc/perses)"
info "ExecStart: ${EXEC_CMD}"

# ── 6. Pare-feu ───────────────────────────────────────────────────────────────
if ! $SKIP_FIREWALL && command -v ufw >/dev/null 2>&1; then
    if $SUDO ufw status 2>/dev/null | grep -q "Status: active"; then
        step "Ouverture port UFW ${PERSES_PORT}/tcp…"
        $SUDO ufw allow "${PERSES_PORT}/tcp" >/dev/null
        info "UFW: port ${PERSES_PORT} autorisé"
    fi
fi

# ── 7. Healthcheck + projet + percli apply ──────────────────────────────────
step "Attente du démarrage Perses (max 20s)…"
STARTED=false
for i in {1..10}; do
    sleep 2
    if curl -sf "http://127.0.0.1:${PERSES_PORT}/api/v1/health" >/dev/null 2>&1; then
        STARTED=true
        break
    fi
done

if $STARTED; then
    info "Perses opérationnel sur http://127.0.0.1:${PERSES_PORT}"

    # Créer le projet "default" via l'API si absent
    step "Création du projet \"default\" via l'API…"
    HTTP_STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
        "http://127.0.0.1:${PERSES_PORT}/api/v1/projects/default" 2>/dev/null || echo "000")
    if [[ "$HTTP_STATUS" == "200" ]]; then
        info "Projet \"default\" déjà existant"
    else
        CREATE_STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
            "http://127.0.0.1:${PERSES_PORT}/api/v1/projects" \
            -H 'Content-Type: application/json' \
            -d '{"kind":"Project","metadata":{"name":"default"},"spec":{}}' 2>/dev/null || echo "000")
        if [[ "$CREATE_STATUS" == "200" || "$CREATE_STATUS" == "201" ]]; then
            info "Projet \"default\" créé"
        else
            warn "Création projet : status $CREATE_STATUS"
        fi
    fi

    # Appliquer les ressources avec percli
    if command -v percli >/dev/null 2>&1; then
        percli login "http://localhost:${PERSES_PORT}" >/dev/null 2>&1 || true
        step "Application des ressources via percli…"
        sleep 1
        percli apply -f "${PERSES_PROV_DIR}/victoriametrics-datasource.yaml" 2>/dev/null \
            && info "Datasource VictoriaMetrics appliquée" \
            || warn "percli apply datasource : erreur — relancer : percli apply -f ${PERSES_PROV_DIR}/victoriametrics-datasource.yaml"
        percli apply -f "${PERSES_PROV_DIR}/pv-solar-5y.yaml" 2>/dev/null \
            && info "Dashboard PV Solaire appliqué" \
            || warn "percli apply dashboard : erreur — relancer : percli apply -f ${PERSES_PROV_DIR}/pv-solar-5y.yaml"
    fi
else
    warn "Perses ne répond pas encore"
    warn "Vérifier  : journalctl -u perses -n 20"
    warn "Déboguer  : sudo -u ${SERVICE_USER} $EXEC_CMD"
fi

# ── Résumé final ──────────────────────────────────────────────────────────────
IP="$(hostname -I 2>/dev/null | awk '{print $1}')"

GRAFANA_LINE=""
if systemctl is-active grafana-server >/dev/null 2>&1; then
    GRAFANA_PORT=$(grep -oP '(?<=http_port = )\d+' /etc/grafana/grafana.ini 2>/dev/null || echo "3000")
    GRAFANA_LINE="  Grafana (en parallèle) : http://${IP:-localhost}:${GRAFANA_PORT}\n"
fi

cat <<EOF

${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}
${GREEN}Installation Perses terminée${NC}
${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}

  Perses (nouveau)  : http://${IP:-localhost}:${PERSES_PORT}
$(echo -e "$GRAFANA_LINE")
  Datasource        : VictoriaMetrics → ${VM_URL} (proxy)
  Ressources        : ${PERSES_PROV_DIR}/
  Plugins           : /etc/perses/plugins-archive/
  Données           : ${PERSES_DB_PATH}
  Service           : User=${SERVICE_USER}

  Logs              : journalctl -u perses -f
  Config            : /etc/perses/config.yaml
  Mise à jour       : relancer le script (idempotent)
  Désinstaller      : sudo bash scripts/setup-perses.sh --uninstall

${YELLOW}Phase d'essai : Grafana et Perses coexistent sans interférence.${NC}
${YELLOW}Pour migrer (après validation) : sudo systemctl disable --now grafana-server${NC}

EOF
