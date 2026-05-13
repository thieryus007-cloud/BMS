#!/usr/bin/env bash
#
# setup-grafana.sh
# ----------------------------------------------------------------------------
# Installation et configuration complète de Grafana sur Raspberry Pi 5 (aarch64)
# pour le monitoring PV solaire avec VictoriaMetrics.
#
# Cible :  Raspberry Pi 5 — Debian/Ubuntu 64 bits — user pi5compute
# Source : ~/Daly-BMS-Rust  (lance via : bash scripts/setup-grafana.sh)
#
# Fonctionnalités :
#   - Installe Grafana OSS depuis le dépôt officiel (ARM64)
#   - Provisionne automatiquement la datasource VictoriaMetrics
#   - Provisionne automatiquement le dashboard "PV Solaire 5 ans"
#   - Configure le pare-feu (si UFW actif) et le port 3000
#   - Vérifie la connectivité à VictoriaMetrics (http://127.0.0.1:8428)
#   - Optionnellement : installe le plugin grafana-image-renderer
#   - Idempotent : peut être relancé sans casser une install existante
#
# Options :
#   --renderer   : installe grafana-image-renderer + chromium (~300 Mo)
#   --port=N     : change le port Grafana (défaut 3000)
#   --admin-pwd=PASS : définit le mot de passe admin initial
#   --vm-url=URL : URL VictoriaMetrics (défaut http://127.0.0.1:8428)
#   --no-firewall : désactive l'ajout de règle UFW
#   --uninstall  : désinstalle Grafana et nettoie la configuration
#
# Exemples :
#   bash scripts/setup-grafana.sh
#   bash scripts/setup-grafana.sh --renderer --admin-pwd='ChangeMe!2026'
#   bash scripts/setup-grafana.sh --port=8081
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
INSTALL_RENDERER=false
SKIP_FIREWALL=false
UNINSTALL=false
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GRAFANA_PROVISION_SRC="${REPO_ROOT}/contrib/grafana"

# ── Parsing args ──────────────────────────────────────────────────────────────
for arg in "$@"; do
    case "$arg" in
        --renderer)       INSTALL_RENDERER=true ;;
        --no-firewall)    SKIP_FIREWALL=true ;;
        --uninstall)      UNINSTALL=true ;;
        --port=*)         GRAFANA_PORT="${arg#*=}" ;;
        --admin-pwd=*)    ADMIN_PWD="${arg#*=}" ;;
        --vm-url=*)       VM_URL="${arg#*=}" ;;
        -h|--help)
            sed -n '3,32p' "$0"
            exit 0
            ;;
        *) error "Option inconnue: $arg (essayez --help)" ;;
    esac
done

# ── Vérifications préalables ──────────────────────────────────────────────────
require_sudo() {
    if [[ $EUID -ne 0 ]]; then
        if ! sudo -n true 2>/dev/null; then
            step "Élévation sudo requise…"
        fi
        SUDO="sudo"
    else
        SUDO=""
    fi
}

require_sudo

ARCH="$(dpkg --print-architecture 2>/dev/null || uname -m)"
case "$ARCH" in
    arm64|aarch64) info "Architecture détectée: $ARCH" ;;
    *) warn "Architecture non aarch64 ($ARCH) — installation possible mais non testée" ;;
esac

# ── Désinstallation ───────────────────────────────────────────────────────────
if $UNINSTALL; then
    step "Désinstallation de Grafana…"
    $SUDO systemctl stop grafana-server 2>/dev/null || true
    $SUDO systemctl disable grafana-server 2>/dev/null || true
    $SUDO apt-get remove -y grafana 2>/dev/null || true
    $SUDO rm -rf /etc/grafana/provisioning/datasources/victoriametrics.yaml
    $SUDO rm -rf /etc/grafana/provisioning/dashboards/daly-bms.yaml
    $SUDO rm -rf /var/lib/grafana/dashboards
    info "Grafana désinstallé (les data /var/lib/grafana/grafana.db restent)"
    exit 0
fi

# ── 1. Dépendances système ────────────────────────────────────────────────────
step "Mise à jour APT et installation des dépendances…"
$SUDO apt-get update -qq
$SUDO apt-get install -y -qq \
    apt-transport-https \
    software-properties-common \
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
    info "Clé GPG installée: $KEYRING"
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

# ── 4. Vérification accès VictoriaMetrics ────────────────────────────────────
step "Vérification VictoriaMetrics: $VM_URL …"
if curl -sf "$VM_URL/health" -o /dev/null --max-time 5; then
    info "VictoriaMetrics répond sur $VM_URL"
else
    warn "VictoriaMetrics injoignable sur $VM_URL — Grafana sera installé mais la"
    warn "datasource montrera une erreur tant que VictoriaMetrics n'est pas démarré."
    warn "Vérifiez:  systemctl status victoriametrics"
fi

# ── 5. Provisioning datasource + dashboard ───────────────────────────────────
step "Déploiement du provisioning Grafana…"

[[ -d "$GRAFANA_PROVISION_SRC" ]] || error "Dossier ${GRAFANA_PROVISION_SRC} introuvable"

# Datasource
$SUDO install -m 0644 -o root -g grafana \
    "${GRAFANA_PROVISION_SRC}/provisioning/datasources/victoriametrics.yaml" \
    /etc/grafana/provisioning/datasources/victoriametrics.yaml

# Si une URL VM différente est demandée, on patche le fichier déployé
if [[ "$VM_URL" != "http://127.0.0.1:8428" ]]; then
    $SUDO sed -i "s|http://127.0.0.1:8428|${VM_URL}|g" \
        /etc/grafana/provisioning/datasources/victoriametrics.yaml
    info "Datasource patchée → $VM_URL"
fi

# Provider dashboards
$SUDO install -m 0644 -o root -g grafana \
    "${GRAFANA_PROVISION_SRC}/provisioning/dashboards/daly-bms.yaml" \
    /etc/grafana/provisioning/dashboards/daly-bms.yaml

# Dashboards JSON
$SUDO install -d -o grafana -g grafana -m 0755 /var/lib/grafana/dashboards
$SUDO install -m 0644 -o grafana -g grafana \
    "${GRAFANA_PROVISION_SRC}/dashboards/pv-solar-5y.json" \
    /var/lib/grafana/dashboards/pv-solar-5y.json

info "Provisioning déployé:"
echo "  • /etc/grafana/provisioning/datasources/victoriametrics.yaml"
echo "  • /etc/grafana/provisioning/dashboards/daly-bms.yaml"
echo "  • /var/lib/grafana/dashboards/pv-solar-5y.json"

# ── 6. Configuration grafana.ini (port, admin) ───────────────────────────────
GRAFANA_INI="/etc/grafana/grafana.ini"

if [[ "$GRAFANA_PORT" != "3000" ]]; then
    step "Configuration du port → $GRAFANA_PORT"
    if grep -qE "^http_port\s*=" "$GRAFANA_INI"; then
        $SUDO sed -i -E "s|^http_port\s*=.*|http_port = ${GRAFANA_PORT}|" "$GRAFANA_INI"
    elif grep -qE "^;http_port\s*=" "$GRAFANA_INI"; then
        $SUDO sed -i -E "s|^;http_port\s*=.*|http_port = ${GRAFANA_PORT}|" "$GRAFANA_INI"
    else
        echo -e "\n[server]\nhttp_port = ${GRAFANA_PORT}" | $SUDO tee -a "$GRAFANA_INI" >/dev/null
    fi
    info "Port défini à $GRAFANA_PORT"
fi

# Mot de passe admin (uniquement à la 1ʳᵉ install ; ensuite via API)
if [[ -n "$ADMIN_PWD" ]]; then
    if [[ ! -f /var/lib/grafana/grafana.db ]]; then
        step "Définition admin_password initial dans grafana.ini…"
        if grep -qE "^;?admin_password\s*=" "$GRAFANA_INI"; then
            $SUDO sed -i -E "s|^;?admin_password\s*=.*|admin_password = ${ADMIN_PWD}|" "$GRAFANA_INI"
        fi
        info "Mot de passe admin initial défini"
    else
        warn "DB Grafana déjà présente — mot de passe non modifié."
        warn "Pour le changer:  sudo grafana-cli admin reset-admin-password '<nouveau>'"
    fi
fi

# ── 7. Image renderer (optionnel) ────────────────────────────────────────────
if $INSTALL_RENDERER; then
    step "Installation grafana-image-renderer + Chromium…"
    $SUDO apt-get install -y -qq chromium chromium-sandbox || \
        warn "Chromium non disponible — le plugin pourrait nécessiter une install manuelle"
    $SUDO grafana-cli plugins install grafana-image-renderer || \
        warn "Échec installation grafana-image-renderer (continuer sans)"
    info "Image renderer installé"
fi

# ── 8. Pare-feu UFW (optionnel) ──────────────────────────────────────────────
if ! $SKIP_FIREWALL && command -v ufw >/dev/null 2>&1; then
    if $SUDO ufw status 2>/dev/null | grep -q "Status: active"; then
        step "Ouverture port UFW $GRAFANA_PORT/tcp…"
        $SUDO ufw allow "${GRAFANA_PORT}/tcp" >/dev/null
        info "UFW: port $GRAFANA_PORT autorisé"
    fi
fi

# ── 9. Activation et démarrage du service ────────────────────────────────────
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

# ── 10. Healthcheck final ────────────────────────────────────────────────────
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
cat <<EOF

${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}
${GREEN}Installation Grafana terminée${NC}
${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}

  URL          : http://${IP:-localhost}:${GRAFANA_PORT}
  Login        : admin / admin (à changer à la 1ʳᵉ connexion si non --admin-pwd)
  Datasource   : VictoriaMetrics → ${VM_URL}  (provisionnée)
  Dashboard    : "PV Solaire - Monitoring & Comparaison 5 Ans"
                 dossier "PV Solaire" (provisionné)

  Logs         : journalctl -u grafana-server -f
  Config       : /etc/grafana/grafana.ini
  Provisioning : /etc/grafana/provisioning/

  Désinstaller : bash scripts/setup-grafana.sh --uninstall

EOF
