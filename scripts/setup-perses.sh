#!/usr/bin/env bash
#
# setup-perses.sh
# ----------------------------------------------------------------------------
# Installation et configuration complète de Perses sur Raspberry Pi 5 (aarch64)
# pour le monitoring PV solaire avec VictoriaMetrics.
#
# Cible : Raspberry Pi 5 — Raspberry Pi OS 64 bits (aarch64)
# Source : ~/Daly-BMS-Rust  (lancer via : bash scripts/setup-perses.sh)
#
# Ce que fait le script :
#   1. Télécharge le binaire Perses ARM64 (dernière release GitHub)
#   2. Extrait les plugins dans /etc/perses/plugins/ (auto-découverte Perses)
#   3. Crée /etc/perses/config.yaml (minimal : database + provisioning.folders)
#   4. Copie les ressources de provisioning (Project, GlobalDatasource, Dashboard)
#   5. Crée le service systemd (port 8090, WorkingDirectory=/etc/perses)
#   6. Healthcheck + connexion percli + vérification
#
# Options :
#   --nvme           : stocke la DB dans /mnt/nvme/perses/db
#   --data-path=PATH : chemin custom pour la DB
#   --port=N         : port d'écoute (défaut 8090)
#   --vm-url=URL     : URL VictoriaMetrics (défaut http://127.0.0.1:8428)
#   --no-firewall    : ignore UFW
#   --uninstall      : désinstalle Perses complètement
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
error() { echo -e "${RED}[EE]${NC} $*" >&2; exit 1; }

# ── Paramètres par défaut ─────────────────────────────────────────────────────
PERSES_PORT="8090"
VM_URL="http://127.0.0.1:8428"
PERSES_DATA_PATH=""
SKIP_FIREWALL=false
UNINSTALL=false
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PERSES_PROVISION_SRC="${REPO_ROOT}/contrib/perses"

# ── Parsing args ──────────────────────────────────────────────────────────────
for arg in "$@"; do
    case "$arg" in
        --nvme)           PERSES_DATA_PATH="/mnt/nvme/perses" ;;
        --no-firewall)    SKIP_FIREWALL=true ;;
        --uninstall)      UNINSTALL=true ;;
        --port=*)         PERSES_PORT="${arg#*=}" ;;
        --vm-url=*)       VM_URL="${arg#*=}" ;;
        --data-path=*)    PERSES_DATA_PATH="${arg#*=}" ;;
        -h|--help)
            sed -n '3,37p' "$0"
            exit 0
            ;;
        *) error "Option inconnue: $arg (essayez --help)" ;;
    esac
done

# ── Sudo helper ───────────────────────────────────────────────────────────────
if [[ $EUID -ne 0 ]]; then
    SUDO="sudo"
else
    SUDO=""
fi

# Utilisateur propriétaire des fichiers Perses (non-root)
PERSES_USER="${SUDO_USER:-pi5compute}"

# ── Architecture ──────────────────────────────────────────────────────────────
ARCH="$(dpkg --print-architecture 2>/dev/null || uname -m)"
if [[ "$ARCH" != "arm64" && "$ARCH" != "aarch64" ]]; then
    warn "Architecture détectée: $ARCH — optimisé pour arm64/aarch64 (Pi5)"
fi

# ── Désinstallation ───────────────────────────────────────────────────────────
if $UNINSTALL; then
    step "Désinstallation de Perses…"
    $SUDO systemctl stop perses 2>/dev/null || true
    $SUDO systemctl disable perses 2>/dev/null || true
    $SUDO rm -f /usr/local/bin/perses /usr/local/bin/percli
    $SUDO rm -f /etc/systemd/system/perses.service
    $SUDO rm -rf /etc/perses
    local_db="${PERSES_DATA_PATH:-/var/lib/perses}"
    $SUDO rm -rf "${local_db}"
    $SUDO systemctl daemon-reload
    info "Perses désinstallé"
    exit 0
fi

# ── Vérification NVMe ─────────────────────────────────────────────────────────
if [[ -n "$PERSES_DATA_PATH" && "$PERSES_DATA_PATH" == /mnt/nvme* ]]; then
    if ! mountpoint -q /mnt/nvme 2>/dev/null; then
        error "/mnt/nvme n'est pas monté."
    fi
    info "NVMe monté — DB Perses → ${PERSES_DATA_PATH}/db"
fi

DB_PATH="${PERSES_DATA_PATH:-/var/lib/perses}/db"

# ── 1. Téléchargement du binaire Perses (ARM64) ───────────────────────────────
step "Téléchargement de la dernière version de Perses (ARM64)…"

LATEST=$(curl -fsSL https://api.github.com/repos/perses/perses/releases/latest \
    | grep '"tag_name"' | cut -d '"' -f 4)
VERSION="${LATEST#v}"

TMPDIR_PERSES="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_PERSES"' EXIT

ARCHIVE_URL="https://github.com/perses/perses/releases/download/${LATEST}/perses_${VERSION}_linux_arm64.tar.gz"
step "Téléchargement : ${ARCHIVE_URL}"
curl -fsSL "${ARCHIVE_URL}" -o "${TMPDIR_PERSES}/perses.tar.gz"
tar xzf "${TMPDIR_PERSES}/perses.tar.gz" -C "${TMPDIR_PERSES}"

EXTRACT_DIR="${TMPDIR_PERSES}/perses_${VERSION}_linux_arm64"

$SUDO install -m 0755 "${EXTRACT_DIR}/perses"  /usr/local/bin/perses
$SUDO install -m 0755 "${EXTRACT_DIR}/percli"  /usr/local/bin/percli

info "Perses ${VERSION} installé (perses + percli dans /usr/local/bin)"

# ── 2. Répertoires ────────────────────────────────────────────────────────────
step "Création des répertoires Perses…"

$SUDO mkdir -p \
    /etc/perses/plugins \
    /etc/perses/provisioning \
    "${DB_PATH}"

# ── 3. Extraction des plugins ─────────────────────────────────────────────────
# Perses auto-découvre les plugins dans ./plugins/ relatif au WorkingDirectory.
# Les plugins viennent du dossier plugins-archive/ de l'archive de release.
step "Installation des plugins Perses dans /etc/perses/plugins/…"

PLUGINS_ARCHIVE_DIR="${EXTRACT_DIR}/plugins-archive"
if [[ -d "${PLUGINS_ARCHIVE_DIR}" ]]; then
    plugin_count=0
    for plugin_tar in "${PLUGINS_ARCHIVE_DIR}"/*.tar.gz; do
        [[ -f "$plugin_tar" ]] || continue
        plugin_name="$(basename "$plugin_tar" .tar.gz)"
        plugin_dir="/etc/perses/plugins/${plugin_name}"
        $SUDO mkdir -p "${plugin_dir}"
        $SUDO tar xzf "$plugin_tar" -C "${plugin_dir}" 2>/dev/null || \
            $SUDO tar xzf "$plugin_tar" -C /etc/perses/plugins/ 2>/dev/null || true
        plugin_count=$((plugin_count + 1))
    done
    info "${plugin_count} plugin(s) extrait(s) dans /etc/perses/plugins/"
else
    warn "Aucun dossier plugins-archive/ trouvé dans l'archive — plugins absents."
fi

# ── 4. Fichier de configuration ───────────────────────────────────────────────
# IMPORTANT : Pas de bloc "server:" — le port est passé en flag CLI.
# Le répertoire de travail = /etc/perses → Perses découvre ./plugins/ automatiquement.
step "Génération de /etc/perses/config.yaml…"

$SUDO tee /etc/perses/config.yaml > /dev/null <<EOF
# Perses config — généré par setup-perses.sh
database:
  file:
    folder: ${DB_PATH}
    extension: json

plugins:
  archive_path: ${PERSES_PLUGINS_DIR}

provisioning:
  interval: 1m
  folders:
    - /etc/perses/provisioning
EOF

info "config.yaml créé (database + provisioning.folders)"

# ── 5. Ressources de provisioning ─────────────────────────────────────────────
step "Copie des ressources de provisioning…"

# Projet "default"
$SUDO tee /etc/perses/provisioning/project-default.yaml > /dev/null <<EOF
kind: Project
metadata:
  name: default
spec: {}
EOF

# GlobalDatasource VictoriaMetrics (plugin Prometheus)
$SUDO tee /etc/perses/provisioning/victoriametrics-datasource.yaml > /dev/null <<EOF
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
EOF

# Dashboard PV Solaire (depuis le repo)
DASH_SRC="${PERSES_PROVISION_SRC}/dashboards/pv-solar-5y.yaml"
if [[ -f "${DASH_SRC}" ]]; then
    $SUDO cp "${DASH_SRC}" /etc/perses/provisioning/pv-solar-5y.yaml
    info "Dashboard PV Solaire copié depuis contrib/"
else
    warn "Dashboard non trouvé dans ${DASH_SRC} — à copier manuellement"
fi

# Propriété fichiers
$SUDO chown -R "${PERSES_USER}:${PERSES_USER}" /etc/perses "${DB_PATH}" 2>/dev/null || \
    $SUDO chown -R "${PERSES_USER}" /etc/perses "${DB_PATH}" 2>/dev/null || true

info "Ressources de provisioning prêtes dans /etc/perses/provisioning/"

# ── 6. Service systemd ────────────────────────────────────────────────────────
step "Création du service systemd perses…"

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

$SUDO tee /etc/systemd/system/perses.service > /dev/null <<SYSTEMD_UNIT
[Unit]
Description=Perses Monitoring Dashboard
Documentation=https://perses.dev
After=network.target
Wants=victoriametrics.service

[Service]
Type=simple
User=${PERSES_USER}
WorkingDirectory=/etc/perses
ExecStart=/usr/local/bin/perses --config /etc/perses/config.yaml --web.listen-address=:${PERSES_PORT}
Restart=on-failure
RestartSec=10
LimitNOFILE=65535
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
SYSTEMD_UNIT

$SUDO chown -R "${SERVICE_USER}:${SERVICE_USER}" /etc/perses "$PERSES_DB_PATH" 2>/dev/null || true

$SUDO systemctl daemon-reload
$SUDO systemctl enable --now perses
info "Service perses activé et démarré"

# ── 7. Pare-feu UFW ───────────────────────────────────────────────────────────
if ! $SKIP_FIREWALL && command -v ufw >/dev/null 2>&1; then
    if $SUDO ufw status 2>/dev/null | grep -q "Status: active"; then
        $SUDO ufw allow "${PERSES_PORT}/tcp" >/dev/null
        info "UFW: port ${PERSES_PORT}/tcp autorisé"
    fi
fi

# ── 8. Healthcheck ────────────────────────────────────────────────────────────
step "Attente démarrage Perses (max 30s)…"
PERSES_URL="http://127.0.0.1:${PERSES_PORT}"
ok=false
for i in $(seq 1 15); do
    sleep 2
    if curl -sf "${PERSES_URL}/api/v1/health" >/dev/null 2>&1; then
        ok=true
        break
    fi
done

if ! $ok; then
    warn "Perses ne répond pas après 30s. Vérifiez les logs :"
    warn "  journalctl -u perses -n 30 --no-pager"
    $SUDO journalctl -u perses -n 20 --no-pager 2>/dev/null || true
    exit 1
fi
info "Perses répond sur ${PERSES_URL}"

# ── 9. Connexion percli ───────────────────────────────────────────────────────
step "Configuration percli (login)…"
PERCLI_HOME="${HOME}/.perses"
mkdir -p "${PERCLI_HOME}"
percli login "${PERSES_URL}" 2>/dev/null && info "percli connecté à ${PERSES_URL}" || \
    warn "percli login a échoué — les ressources sont provisionnées via fichiers"

# ── Résumé ────────────────────────────────────────────────────────────────────
IP="$(hostname -I 2>/dev/null | awk '{print $1}')"
echo -e "
${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}
${GREEN}Installation Perses ${VERSION} terminée${NC}
${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}

  URL          : http://${IP:-localhost}:${PERSES_PORT}
  Datasource   : VictoriaMetrics → ${VM_URL}
  Dashboard    : PV Solaire (projet 'default')
  Plugins      : /etc/perses/plugins/
  DB           : ${DB_PATH}
  Config       : /etc/perses/config.yaml
  Provisioning : /etc/perses/provisioning/

  Logs         : journalctl -u perses -f
  Redémarrer   : sudo systemctl restart perses
  Désinstaller : sudo bash scripts/setup-perses.sh --uninstall

  Note : le provisioning (datasource, dashboard) peut prendre jusqu'à 1 minute.
         Actualiser l'interface si rien n'apparaît immédiatement.
"
