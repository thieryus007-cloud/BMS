#!/usr/bin/env bash
# Déploiement complet Pi5 — daly-bms-server + energy-manager
# Usage (depuis ~/Daly-BMS-Rust sur le Pi5) : bash scripts/deploy-pi5.sh

set -euo pipefail

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; NC='\033[0m'
info()  { echo -e "${GREEN}[OK]${NC} $*"; }
step()  { echo -e "${YELLOW}[>>]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!!]${NC} $*"; }
error() { echo -e "${RED}[!!]${NC} $*" >&2; exit 1; }

# ── 1. Récupération du code ───────────────────────────────────────────────────
step "Synchronisation du dépôt…"
make sync || error "make sync a échoué"
info "Code à jour"

# ── 2. Compilation croisée aarch64 ───────────────────────────────────────────
step "Compilation daly-bms-server (aarch64)…"
make build-arm || error "make build-arm a échoué"
info "daly-bms-server compilé"

step "Compilation energy-manager (aarch64)…"
make build-energy-arm || error "make build-energy-arm a échoué"
info "energy-manager compilé"

# ── 3. Mise à jour de la configuration ──────────────────────────────────────
step "Déploiement Config.toml → /etc/daly-bms/config.toml…"
sudo cp Config.toml /etc/daly-bms/config.toml
info "Config.toml déployée"

# ── 4. Répertoire Tsink ──────────────────────────────────────────────────────
TSINK_DIR="/var/lib/daly-bms/tsink"
# Récupère l'utilisateur qui exécute le service (ExecStart user, pas root)
SERVICE_USER=$(systemctl show daly-bms --property=User --value 2>/dev/null)
SERVICE_USER="${SERVICE_USER:-dalybms}"
step "Vérification répertoire Tsink (${TSINK_DIR}) → owner=${SERVICE_USER}…"
sudo mkdir -p "${TSINK_DIR}"
sudo chown "${SERVICE_USER}:${SERVICE_USER}" "${TSINK_DIR}"
info "Répertoire Tsink OK (owner=${SERVICE_USER})"

# ── 5. Déploiement daly-bms-server ───────────────────────────────────────────
step "Déploiement daly-bms-server…"
sudo systemctl stop daly-bms
sudo cp target/aarch64-unknown-linux-gnu/release/daly-bms-server /usr/local/bin/
sudo systemctl start daly-bms
sleep 3

if ! systemctl is-active --quiet daly-bms; then
    error "daly-bms n'a pas démarré — vérifier : journalctl -u daly-bms -n 50"
fi
info "daly-bms actif"

# Vérification VictoriaMetrics
sleep 2
TSINK_LOG=$(journalctl -u daly-bms --since "30 seconds ago" 2>/dev/null | grep -i tsink | head -3)
if echo "${TSINK_LOG}" | grep -q "activé"; then
    info "Tsink initialisé : ${TSINK_DIR}"
elif echo "${TSINK_LOG}" | grep -q "échoué\|error\|Error"; then
    warn "Tsink init a échoué — voir : journalctl -u daly-bms -n 30 | grep -i tsink"
else
    warn "Tsink : aucun message de démarrage détecté (vérifier la config)"
fi

# ── 6. Déploiement energy-manager ────────────────────────────────────────────
sleep 3
step "Déploiement energy-manager…"
sudo systemctl stop energy-manager
sudo cp target/aarch64-unknown-linux-gnu/release/energy-manager /usr/local/bin/
sudo systemctl start energy-manager
sleep 2
if systemctl is-active --quiet energy-manager; then
    info "energy-manager actif"
else
    error "energy-manager n'a pas démarré — vérifier : journalctl -u energy-manager -n 50"
fi

# ── 7. Résumé ─────────────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}═══════════════════════════════════════${NC}"
echo -e "${GREEN}  Déploiement terminé avec succès ✓${NC}"
echo -e "${GREEN}═══════════════════════════════════════${NC}"
echo ""
systemctl status daly-bms energy-manager --no-pager -l | grep -E "Active:|Loaded:" || true
echo ""
step "Vérification Tsink via API…"
sleep 1
curl -sf 'http://localhost:8080/health' 2>/dev/null | python3 -m json.tool 2>/dev/null || \
    warn "Endpoint /health non accessible"
