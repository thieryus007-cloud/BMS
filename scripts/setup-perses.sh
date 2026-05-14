#!/usr/bin/env bash
#
# setup-perses.sh
# ----------------------------------------------------------------------------
# Installation et configuration complète de Perses sur Raspberry Pi 5 (aarch64)
# pour le monitoring PV solaire avec VictoriaMetrics.
#
# Cible : Raspberry Pi 5 — Raspberry Pi OS 64 bits
# Source : ~/Daly-BMS-Rust  (lance via : bash scripts/setup-perses.sh)
#
# Fonctionnalités :
#   - Télécharge et installe le binaire officiel ARM64
#   - Crée un service systemd sur le port 8090 (8080 déjà utilisé)
#   - Provisionne automatiquement la datasource VictoriaMetrics
#   - Provisionne le dashboard PV Solaire (format Perses)
#   - Support NVMe pour les données
#   - Idempotent et avec option --uninstall
#
# Options :
#   --nvme           : stocke les données Perses sur /mnt/nvme/perses
#   --data-path=PATH : chemin custom pour les données
#   --port=N         : port d'écoute (défaut 8090)
#   --vm-url=URL     : URL VictoriaMetrics (défaut http://127.0.0.1:8428)
#   --no-firewall    : désactive l'ajout de règle UFW
#   --uninstall      : désinstalle Perses
#
# Exemples :
#   bash scripts/setup-perses.sh --nvme
#   bash scripts/setup-perses.sh --nvme --port=8090
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
PERS ES_PORT="8090"
VM_URL="http://127.0.0.1:8428"
PERS ES_DATA_PATH=""          # vide = /var/lib/perses
SKIP_FIREWALL=false
UNINSTALL=false
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PERS ES_PROVISION_SRC="${REPO_ROOT}/contrib/perses"

# ── Parsing args ──────────────────────────────────────────────────────────────
for arg in "$@"; do
    case "$arg" in
        --nvme)           PERS ES_DATA_PATH="/mnt/nvme/perses" ;;
        --no-firewall)    SKIP_FIREWALL=true ;;
        --uninstall)      UNINSTALL=true ;;
        --port=*)         PERS ES_PORT="${arg#*=}" ;;
        --vm-url=*)       VM_URL="${arg#*=}" ;;
        --data-path=*)    PERS ES_DATA_PATH="${arg#*=}" ;;
        -h|--help)
            sed -n '3,35p' "$0"
            exit 0
            ;;
        *) error "Option inconnue: $arg (essayez --help)" ;;
    esac
done

# ── Vérifications préalables ──────────────────────────────────────────────────
if [[ $EUID -ne 0 ]]; then
    SUDO="sudo"
else
    SUDO=""
fi

ARCH="$(dpkg --print-architecture 2>/dev/null || uname -m)"
if [[ "$ARCH" != "arm64" && "$ARCH" != "aarch64" ]]; then
    warn "Architecture détectée: $ARCH — installation possible mais non optimisée pour Pi 5"
fi

# Vérifier NVMe
if [[ -n "$PERS ES_DATA_PATH" && "$PERS ES_DATA_PATH" == /mnt/nvme* ]]; then
    if ! mountpoint -q /mnt/nvme 2>/dev/null; then
        error "/mnt/nvme n'est pas monté."
    fi
    info "NVMe monté — données Perses → $PERS ES_DATA_PATH"
fi

# ── Désinstallation ───────────────────────────────────────────────────────────
if $UNINSTALL; then
    step "Désinstallation de Perses…"
    $SUDO systemctl stop perses 2>/dev/null || true
    $SUDO systemctl disable perses 2>/dev/null || true
    $SUDO rm -f /usr/local/bin/perses /usr/local/bin/percli
    $SUDO rm -f /etc/systemd/system/perses.service
    $SUDO rm -rf /etc/perses
    $SUDO rm -rf "${PERS ES_DATA_PATH:-/var/lib/perses}"
    $SUDO systemctl daemon-reload
    info "Perses désinstallé"
    exit 0
fi

# ── 1. Téléchargement du binaire Perses (ARM64) ───────────────────────────────
step "Téléchargement de la dernière version de Perses…"
LATEST=$(curl -s https://api.github.com/repos/perses/perses/releases/latest | grep '"tag_name"' | cut -d '"' -f 4)
VERSION=${LATEST#v}

cd /tmp
wget -q "https://github.com/perses/perses/releases/download/${LATEST}/perses_${VERSION}_linux_arm64.tar.gz" -O perses.tar.gz
tar xzf perses.tar.gz

$SUDO install -m 0755 perses_${VERSION}_linux_arm64/perses /usr/local/bin/
$SUDO install -m 0755 perses_${VERSION}_linux_arm64/percli /usr/local/bin/

info "Perses ${VERSION} installé (perses + percli)"

# ── 2. Création de la configuration ───────────────────────────────────────────
step "Configuration Perses…"

$SUDO mkdir -p /etc/perses /var/lib/perses/db "${PERS ES_DATA_PATH:+${PERS ES_DATA_PATH}/db}"

cat <<EOF | $SUDO tee /etc/perses/config.yaml > /dev/null
server:
  port: ${PERS ES_PORT}
  enableUI: true

database:
  file:
    folder: ${PERS ES_DATA_PATH:-/var/lib/perses}/db
    extension: json

provisioning:
  dashboards:
    - folder: /etc/perses/dashboards
  datasources:
    - folder: /etc/perses/datasources
EOF

$SUDO mkdir -p /etc/perses/{dashboards,datasources}
info "Fichier de configuration créé (/etc/perses/config.yaml)"

# ── 3. Provisioning Datasource VictoriaMetrics ───────────────────────────────
step "Provisioning datasource VictoriaMetrics…"

cat <<EOF | $SUDO tee /etc/perses/datasources/victoriametrics.yaml > /dev/null
apiVersion: 1
kind: Datasource
metadata:
  name: victoriametrics
  project: perses
spec:
  display:
    name: VictoriaMetrics
  plugin:
    kind: Prometheus
    spec:
      url: ${VM_URL}
      proxy: true
      timeInterval: 10s
EOF

info "Datasource VictoriaMetrics provisionnée"

# ── 4. Dashboard PV Solaire (à migrer manuellement une première fois) ────────
step "Préparation du dossier dashboard…"
$SUDO mkdir -p /etc/perses/dashboards

if [[ -f "${REPO_ROOT}/contrib/perses/dashboards/pv-solar-5y.yaml" ]]; then
    $SUDO cp "${REPO_ROOT}/contrib/perses/dashboards/pv-solar-5y.yaml" /etc/perses/dashboards/
    info "Dashboard PV Solaire provisionné"
else
    warn "Dashboard Perses non trouvé dans contrib/perses."
    warn "Pense à migrer ton dashboard Grafana avec :"
    warn "   percli migrate -f pv-solar-5y.json --online -o yaml > pv-solar-5y.yaml"
fi

# ── 5. Service systemd ────────────────────────────────────────────────────────
step "Création du service systemd…"

$SUDO tee /etc/systemd/system/perses.service > /dev/null <<EOF
[Unit]
Description=Perses Monitoring Dashboard
After=network.target

[Service]
Type=simple
User=${SUDO_USER:-pi}
ExecStart=/usr/local/bin/perses --config /etc/perses/config.yaml
Restart=always
RestartSec=5
LimitNOFILE=65535
WorkingDirectory=${PERS ES_DATA_PATH:-/var/lib/perses}

[Install]
WantedBy=multi-user.target
EOF

$SUDO systemctl daemon-reload
$SUDO systemctl enable --now perses

# ── 6. Pare-feu ───────────────────────────────────────────────────────────────
if ! $SKIP_FIREWALL && command -v ufw >/dev/null 2>&1; then
    if $SUDO ufw status 2>/dev/null | grep -q "Status: active"; then
        step "Ouverture port UFW ${PERS ES_PORT}/tcp…"
        $SUDO ufw allow "${PERS ES_PORT}/tcp" >/dev/null
        info "UFW: port $PERS ES_PORT autorisé"
    fi
fi

# ── 7. Healthcheck ────────────────────────────────────────────────────────────
step "Vérification Perses…"
sleep 4
for i in {1..8}; do
    if curl -sf "http://127.0.0.1:${PERS ES_PORT}/api/v1/health" >/dev/null 2>&1; then
        info "Perses répond sur http://127.0.0.1:${PERS ES_PORT}"
        break
    fi
    sleep 2
done

# ── Résumé final ──────────────────────────────────────────────────────────────
IP="$(hostname -I 2>/dev/null | awk '{print $1}')"
cat <<EOF

${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}
${GREEN}Installation Perses terminée${NC}
${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}

  URL          : http://${IP:-localhost}:${PERS ES_PORT}
  Port         : ${PERS ES_PORT}
  Datasource   : VictoriaMetrics → ${VM_URL}
  Dashboard    : PV Solaire (dans /etc/perses/dashboards)
${PERS ES_DATA_PATH:+  Données     : ${PERS ES_DATA_PATH}}

  Logs         : journalctl -u perses -f
  Config       : /etc/perses/config.yaml
  Mise à jour  : relancer le script

  Désinstaller : sudo bash scripts/setup-perses.sh --uninstall

EOF
