#!/usr/bin/env bash
#
# setup-grafana.sh
# ----------------------------------------------------------------------------
# Installation et configuration complète de Grafana sur Raspberry Pi 5 (aarch64)
# pour le monitoring PV solaire avec VictoriaMetrics.
#
# Cible :  Raspberry Pi 5 — Raspberry Pi OS 64 bits — user pi5compute
# Source : ~/Daly-BMS-Rust  (lance via : bash scripts/setup-grafana.sh)
#
# Fonctionnalités :
#   - Installe Grafana OSS depuis le dépôt officiel (ARM64)
#   - Provisionne automatiquement la datasource VictoriaMetrics
#   - Provisionne automatiquement le dashboard "PV Solaire 5 ans"
#   - Option --nvme : stocke les données Grafana sur le NVMe (/mnt/nvme/grafana)
#     → la SD card ne porte que le binaire (~150 Mo)
#   - Configure le pare-feu (si UFW actif) et le port 3000
#   - Vérifie la connectivité à VictoriaMetrics (http://127.0.0.1:8428)
#   - Idempotent : peut être relancé sans casser une install existante
#
# Options :
#   --nvme           : stocke DB/logs/plugins sur /mnt/nvme/grafana (recommandé Pi5)
#   --data-path=PATH : chemin custom pour les données Grafana (remplace --nvme)
#   --port=N         : change le port Grafana (défaut 3000)
#   --admin-pwd=PASS : définit le mot de passe admin initial
#   --vm-url=URL     : URL VictoriaMetrics (défaut http://127.0.0.1:8428)
#   --no-firewall    : désactive l'ajout de règle UFW
#   --renderer       : installe grafana-image-renderer + chromium (~300 Mo)
#   --uninstall      : désinstalle Grafana et nettoie la configuration
#
# Exemples :
#   bash scripts/setup-grafana.sh --nvme
#   bash scripts/setup-grafana.sh --nvme --admin-pwd='ChangeMe!2026'
#   bash scripts/setup-grafana.sh --port=3000
#   sudo bash scripts/setup-grafana.sh --uninstall
# ----------------------------------------------------------------------------

set -euo pipefail

# ── Couleurs ──────────────────────────────────────────────────────────────────
GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; BLUE='\033[0;34m'; NC='\033[0m'
info()  { echo -e "${GREEN}[OK]${NC} $*"; }
step()  { echo -e "${BLUE}[>>]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!!]${NC} $*"; }
error() { echo -e "${RED}[!!]${NC} $*" >&2; exit 1; }

# ── Paramètres par défaut ─────────────────────────────────────────────────────
GRAFANA_PORT="3000"
ADMIN_PWD=""
VM_URL="http://127.0.0.1:8428"
GRAFANA_DATA_PATH=""          # vide = /var/lib/grafana (défaut Grafana)
INSTALL_RENDERER=false
SKIP_FIREWALL=false
UNINSTALL=false
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GRAFANA_PROVISION_SRC="${REPO_ROOT}/contrib/grafana"

# ── Parsing args ──────────────────────────────────────────────────────────────
for arg in "$@"; do
    case "$arg" in
        --nvme)           GRAFANA_DATA_PATH="/mnt/nvme/grafana" ;;
        --renderer)       INSTALL_RENDERER=true ;;
        --no-firewall)    SKIP_FIREWALL=true ;;
        --uninstall)      UNINSTALL=true ;;
        --port=*)         GRAFANA_PORT="${arg#*=}" ;;
        --admin-pwd=*)    ADMIN_PWD="${arg#*=}" ;;
        --vm-url=*)       VM_URL="${arg#*=}" ;;
        --data-path=*)    GRAFANA_DATA_PATH="${arg#*=}" ;;
        -h|--help)
            sed -n '3,38p' "$0"
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
case "$ARCH" in
    arm64|aarch64) info "Architecture détectée: $ARCH" ;;
    *) warn "Architecture non aarch64 ($ARCH) — installation possible mais non testée" ;;
esac

# Vérifier que le NVMe est monté si --nvme demandé
if [[ -n "$GRAFANA_DATA_PATH" && "$GRAFANA_DATA_PATH" == /mnt/nvme* ]]; then
    if ! mountpoint -q /mnt/nvme 2>/dev/null; then
        error "/mnt/nvme n'est pas monté. Vérifiez: mount | grep nvme"
    fi
    info "NVMe monté — données Grafana → $GRAFANA_DATA_PATH"
fi

# ── Désinstallation ───────────────────────────────────────────────────────────
if $UNINSTALL; then
    step "Désinstallation de Grafana…"
    $SUDO systemctl stop grafana-server 2>/dev/null || true
    $SUDO systemctl disable grafana-server 2>/dev/null || true
    $SUDO apt-get remove -y grafana 2>/dev/null || true
    $SUDO rm -f /etc/grafana/provisioning/datasources/victoriametrics.yaml
    $SUDO rm -f /etc/grafana/provisioning/dashboards/daly-bms.yaml
    $SUDO rm -rf /var/lib/grafana/dashboards
    if [[ -n "$GRAFANA_DATA_PATH" ]]; then
        warn "Données NVMe conservées dans $GRAFANA_DATA_PATH (suppression manuelle si besoin)"
    fi
    info "Grafana désinstallé"
    exit 0
fi

# ── 1. Dépendances système ────────────────────────────────────────────────────
step "Mise à jour APT et installation des dépendances…"
$SUDO apt-get update -qq
$SUDO apt-get install -y -qq \
    apt-transport-https \
    wget \
    curl \
    gnupg \
    ca-certificates
info "Dépendances installées"

# ── 2. Dépôt Grafana ──────────────────────────────────────────────────────────
KEYRING="/usr/share/keyrings/grafana.gpg"
SRC_LIST="/etc/apt/sources.list.d/grafana.list"

if [[ ! -f "$KEYRING" ]]; then
    step "Ajout de la clé GPG Grafana…"
    wget -q -O - https://apt.grafana.com/gpg.key | $SUDO gpg --dearmor -o "$KEYRING"
    info "Clé GPG installée"
else
    info "Clé GPG Grafana déjà présente"
fi

EXPECTED_SRC="deb [signed-by=${KEYRING}] https://apt.grafana.com stable main"
if [[ ! -f "$SRC_LIST" ]] || ! grep -qF "$EXPECTED_SRC" "$SRC_LIST"; then
    step "Configuration du dépôt Grafana…"
    echo "$EXPECTED_SRC" | $SUDO tee "$SRC_LIST" >/dev/null
    $SUDO apt-get update -qq
    info "Dépôt Grafana configuré"
else
    info "Dépôt Grafana déjà configuré"
fi

# ── 3. Installation Grafana ───────────────────────────────────────────────────
if ! dpkg -l grafana >/dev/null 2>&1; then
    step "Installation de Grafana OSS…"
    $SUDO apt-get install -y grafana
    info "Grafana installé"
else
    INSTALLED_VER="$(dpkg-query -W -f='${Version}' grafana)"
    info "Grafana déjà installé (version $INSTALLED_VER)"
fi

# ── 4. Stockage des données sur NVMe (optionnel) ─────────────────────────────
GRAFANA_INI="/etc/grafana/grafana.ini"

if [[ -n "$GRAFANA_DATA_PATH" ]]; then
    step "Configuration du stockage NVMe → $GRAFANA_DATA_PATH"

    # Créer les sous-dossiers avec le bon propriétaire
    for subdir in "" data log plugins temp; do
        DIR="${GRAFANA_DATA_PATH}${subdir:+/$subdir}"
        $SUDO mkdir -p "$DIR"
        $SUDO chown grafana:grafana "$DIR"
        $SUDO chmod 0755 "$DIR"
    done

    # Patcher la section [paths] dans grafana.ini
    # On insère/remplace les clés data, logs, plugins
    patch_ini_path() {
        local key="$1" val="$2"
        if $SUDO grep -qE "^${key}\s*=" "$GRAFANA_INI"; then
            $SUDO sed -i -E "s|^${key}\s*=.*|${key} = ${val}|" "$GRAFANA_INI"
        elif $SUDO grep -qE "^;${key}\s*=" "$GRAFANA_INI"; then
            $SUDO sed -i -E "s|^;${key}\s*=.*|${key} = ${val}|" "$GRAFANA_INI"
        else
            # Ajouter sous [paths] si la section existe
            if $SUDO grep -q "^\[paths\]" "$GRAFANA_INI"; then
                $SUDO sed -i "/^\[paths\]/a ${key} = ${val}" "$GRAFANA_INI"
            else
                printf "\n[paths]\n%s = %s\n" "$key" "$val" | $SUDO tee -a "$GRAFANA_INI" >/dev/null
            fi
        fi
    }

    patch_ini_path "data"    "${GRAFANA_DATA_PATH}/data"
    patch_ini_path "logs"    "${GRAFANA_DATA_PATH}/log"
    patch_ini_path "plugins" "${GRAFANA_DATA_PATH}/plugins"
    patch_ini_path "temp_data_lifetime" "24h"

    info "Chemins Grafana configurés sur NVMe:"
    echo "  • data    → ${GRAFANA_DATA_PATH}/data"
    echo "  • logs    → ${GRAFANA_DATA_PATH}/log"
    echo "  • plugins → ${GRAFANA_DATA_PATH}/plugins"

    # Migrer la DB existante si présente
    if [[ -f /var/lib/grafana/grafana.db && ! -f "${GRAFANA_DATA_PATH}/data/grafana.db" ]]; then
        step "Migration de la DB existante vers NVMe…"
        $SUDO cp /var/lib/grafana/grafana.db "${GRAFANA_DATA_PATH}/data/grafana.db"
        $SUDO chown grafana:grafana "${GRAFANA_DATA_PATH}/data/grafana.db"
        info "DB migrée vers ${GRAFANA_DATA_PATH}/data/grafana.db"
    fi
fi

# ── 5. Vérification accès VictoriaMetrics ────────────────────────────────────
step "Vérification VictoriaMetrics: $VM_URL …"
if curl -sf "$VM_URL/health" -o /dev/null --max-time 5; then
    info "VictoriaMetrics répond sur $VM_URL"
else
    warn "VictoriaMetrics injoignable sur $VM_URL — Grafana sera installé mais la"
    warn "datasource montrera une erreur tant que VictoriaMetrics n'est pas démarré."
    warn "Vérifiez:  systemctl status victoriametrics"
fi

# ── 6. Provisioning datasource + dashboard ───────────────────────────────────
step "Déploiement du provisioning Grafana…"

[[ -d "$GRAFANA_PROVISION_SRC" ]] || error "Dossier ${GRAFANA_PROVISION_SRC} introuvable (make sync ?)"

# Datasource
$SUDO install -m 0644 -o root -g grafana \
    "${GRAFANA_PROVISION_SRC}/provisioning/datasources/victoriametrics.yaml" \
    /etc/grafana/provisioning/datasources/victoriametrics.yaml

if [[ "$VM_URL" != "http://127.0.0.1:8428" ]]; then
    $SUDO sed -i "s|http://127.0.0.1:8428|${VM_URL}|g" \
        /etc/grafana/provisioning/datasources/victoriametrics.yaml
    info "Datasource patchée → $VM_URL"
fi

# Provider dashboards
$SUDO install -m 0644 -o root -g grafana \
    "${GRAFANA_PROVISION_SRC}/provisioning/dashboards/daly-bms.yaml" \
    /etc/grafana/provisioning/dashboards/daly-bms.yaml

# Dossier dashboards (dans /var/lib/grafana — petits fichiers JSON, OK sur SD)
$SUDO install -d -o grafana -g grafana -m 0755 /var/lib/grafana/dashboards
$SUDO install -m 0644 -o grafana -g grafana \
    "${GRAFANA_PROVISION_SRC}/dashboards/pv-solar-5y.json" \
    /var/lib/grafana/dashboards/pv-solar-5y.json

info "Provisioning déployé"

# ── 7. Configuration grafana.ini (port, admin) ───────────────────────────────
patch_ini_key() {
    local key="$1" val="$2"
    if $SUDO grep -qE "^${key}\s*=" "$GRAFANA_INI"; then
        $SUDO sed -i -E "s|^${key}\s*=.*|${key} = ${val}|" "$GRAFANA_INI"
    elif $SUDO grep -qE "^;${key}\s*=" "$GRAFANA_INI"; then
        $SUDO sed -i -E "s|^;${key}\s*=.*|${key} = ${val}|" "$GRAFANA_INI"
    fi
}

if [[ "$GRAFANA_PORT" != "3000" ]]; then
    step "Configuration du port → $GRAFANA_PORT"
    if ! $SUDO grep -q "^\[server\]" "$GRAFANA_INI"; then
        printf "\n[server]\nhttp_port = %s\n" "$GRAFANA_PORT" | $SUDO tee -a "$GRAFANA_INI" >/dev/null
    else
        patch_ini_key "http_port" "$GRAFANA_PORT"
    fi
    info "Port défini à $GRAFANA_PORT"
fi

if [[ -n "$ADMIN_PWD" ]]; then
    DB_PATH="${GRAFANA_DATA_PATH:+${GRAFANA_DATA_PATH}/data}/grafana.db"
    DB_PATH="${DB_PATH:-/var/lib/grafana/grafana.db}"
    if [[ ! -f "$DB_PATH" ]]; then
        step "Définition admin_password initial…"
        patch_ini_key "admin_password" "$ADMIN_PWD"
        info "Mot de passe admin initial défini"
    else
        warn "DB Grafana déjà présente — mot de passe non modifié."
        warn "Pour le changer:  sudo grafana-cli admin reset-admin-password '<nouveau>'"
    fi
fi

# ── 8. Image renderer (optionnel, lourd) ─────────────────────────────────────
if $INSTALL_RENDERER; then
    step "Installation grafana-image-renderer + Chromium (~300 Mo)…"
    $SUDO apt-get install -y -qq chromium chromium-sandbox || \
        warn "Chromium non disponible via apt"
    $SUDO grafana-cli plugins install grafana-image-renderer || \
        warn "Échec installation grafana-image-renderer"
    info "Image renderer installé"
fi

# ── 9. Pare-feu UFW (optionnel) ──────────────────────────────────────────────
if ! $SKIP_FIREWALL && command -v ufw >/dev/null 2>&1; then
    if $SUDO ufw status 2>/dev/null | grep -q "Status: active"; then
        step "Ouverture port UFW ${GRAFANA_PORT}/tcp…"
        $SUDO ufw allow "${GRAFANA_PORT}/tcp" >/dev/null
        info "UFW: port $GRAFANA_PORT autorisé"
    fi
fi

# ── 10. Activation et démarrage du service ───────────────────────────────────
step "Activation et démarrage grafana-server…"
$SUDO systemctl daemon-reload
$SUDO systemctl enable grafana-server >/dev/null 2>&1
$SUDO systemctl restart grafana-server

sleep 3
if systemctl is-active --quiet grafana-server; then
    info "grafana-server actif"
else
    error "grafana-server ne démarre pas — voir: journalctl -u grafana-server -n 50"
fi

# ── 11. Healthcheck final ────────────────────────────────────────────────────
step "Vérification HTTP Grafana…"
for i in {1..10}; do
    if curl -sf "http://127.0.0.1:${GRAFANA_PORT}/api/health" >/dev/null; then
        info "Grafana répond sur http://127.0.0.1:${GRAFANA_PORT}"
        break
    fi
    [[ $i -eq 10 ]] && warn "Grafana ne répond pas encore — peut prendre quelques secondes de plus"
    sleep 2
done

# ── Résumé ───────────────────────────────────────────────────────────────────
IP="$(hostname -I 2>/dev/null | awk '{print $1}')"
NVME_NOTE=""
[[ -n "$GRAFANA_DATA_PATH" ]] && NVME_NOTE="  Données NVMe : ${GRAFANA_DATA_PATH}/
"
cat <<EOF

${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}
${GREEN}Installation Grafana terminée${NC}
${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}

  URL          : http://${IP:-localhost}:${GRAFANA_PORT}
  Login        : admin / admin  (changer à la 1ʳᵉ connexion)
  Datasource   : VictoriaMetrics → ${VM_URL}  (provisionnée)
  Dashboard    : "PV Solaire - Monitoring & Comparaison 5 Ans" (dossier PV Solaire)
${NVME_NOTE}
  Logs         : journalctl -u grafana-server -f
  Config       : /etc/grafana/grafana.ini
  Provisioning : /etc/grafana/provisioning/

  Désinstaller : sudo bash scripts/setup-grafana.sh --uninstall

EOF
