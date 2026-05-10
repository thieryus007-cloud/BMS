#!/usr/bin/env bash
# Déploiement complet Pi5 — daly-bms-server + energy-manager + mqtt-broker + mqtt-bridge
# Usage (depuis ~/Daly-BMS-Rust sur le Pi5) : bash scripts/deploy-pi5.sh [--mqtt-only] [--skip-mqtt]

set -euo pipefail

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; NC='\033[0m'
info()  { echo -e "${GREEN}[OK]${NC} $*"; }
step()  { echo -e "${YELLOW}[>>]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!!]${NC} $*"; }
error() { echo -e "${RED}[!!]${NC} $*" >&2; exit 1; }

# Options
SKIP_MQTT=false
MQTT_ONLY=false
for arg in "$@"; do
  case "$arg" in
    --skip-mqtt) SKIP_MQTT=true ;;
    --mqtt-only) MQTT_ONLY=true ;;
  esac
done

# ── 1. Récupération du code ───────────────────────────────────────────────────
if [ "$MQTT_ONLY" = false ]; then
  step "Synchronisation du dépôt…"
  make sync || error "make sync a échoué"
  info "Code à jour"
fi

# ── 2. Compilation croisée aarch64 ───────────────────────────────────────────
if [ "$MQTT_ONLY" = false ]; then
  step "Compilation daly-bms-server (aarch64)…"
  make build-arm || error "make build-arm a échoué"
  info "daly-bms-server compilé"

  step "Compilation energy-manager (aarch64)…"
  make build-energy-arm || error "make build-energy-arm a échoué"
  info "energy-manager compilé"
fi

if [ "$SKIP_MQTT" = false ]; then
  step "Compilation mqtt-broker + mqtt-bridge (aarch64)…"
  make build-mqtt-arm || error "make build-mqtt-arm a échoué"
  info "mqtt-broker et mqtt-bridge compilés"
fi

# ── 3. Mise à jour de la configuration ──────────────────────────────────────
step "Déploiement Config.toml → /etc/daly-bms/config.toml…"
sudo cp Config.toml /etc/daly-bms/config.toml
info "Config.toml déployée"

# ── 4. Répertoire VictoriaMetrics ────────────────────────────────────────────
VM_DIR="/var/lib/victoria-metrics"
SERVICE_USER=$(systemctl show victoriametrics --property=User --value 2>/dev/null)
SERVICE_USER="${SERVICE_USER:-victoriametrics}"
step "Vérification répertoire VictoriaMetrics (${VM_DIR}) → owner=${SERVICE_USER}…"
sudo mkdir -p "${VM_DIR}"
sudo chown "${SERVICE_USER}:${SERVICE_USER}" "${VM_DIR}"
info "Répertoire VictoriaMetrics OK (owner=${SERVICE_USER})"

# ── 5. Déploiement mqtt-broker (avant les autres — dépendance) ───────────────
if [ "$SKIP_MQTT" = false ]; then
  ARM_DIR="target/aarch64-unknown-linux-gnu/release"

  step "Déploiement mqtt-broker…"
  # Créer l'utilisateur système si absent
  if ! id mqtt-broker &>/dev/null; then
    sudo useradd --system --no-create-home --shell /usr/sbin/nologin mqtt-broker
    info "Utilisateur système mqtt-broker créé"
  fi
  sudo mkdir -p /var/lib/mqtt-broker
  sudo chown mqtt-broker:mqtt-broker /var/lib/mqtt-broker
  sudo chmod 750 /var/lib/mqtt-broker

  # S'assurer que mqtt-broker peut lire /etc/daly-bms/
  sudo chmod o+rx /etc/daly-bms/ 2>/dev/null || true

  # Déployer/mettre à jour la config broker (toujours, pas seulement si absente)
  sudo cp crates/mqtt-broker/mqtt-broker.toml /etc/daly-bms/mqtt-broker.toml
  sudo chmod 644 /etc/daly-bms/mqtt-broker.toml
  info "mqtt-broker.toml déployé dans /etc/daly-bms/"

  # Déployer le binaire
  sudo systemctl stop mqtt-bridge 2>/dev/null || true
  sudo systemctl stop mqtt-broker 2>/dev/null || true
  sudo cp "${ARM_DIR}/mqtt-broker" /usr/local/bin/mqtt-broker
  sudo chmod 755 /usr/local/bin/mqtt-broker

  # Installer/mettre à jour l'unité systemd
  sudo cp contrib/mqtt-broker.service /etc/systemd/system/
  sudo systemctl daemon-reload
  sudo systemctl enable mqtt-broker 2>/dev/null || true
  info "Service mqtt-broker enregistré"

  sudo systemctl start mqtt-broker
  sleep 3

  if systemctl is-active --quiet mqtt-broker; then
    info "mqtt-broker actif (TCP :1883, WS :9001, metrics :8082)"
  else
    warn "mqtt-broker n'a pas démarré — logs :"
    journalctl -u mqtt-broker -n 20 --no-pager || true
    error "Arrêt — corriger mqtt-broker avant de continuer"
  fi

  # ── 6. Déploiement mqtt-bridge ─────────────────────────────────────────────
  step "Déploiement mqtt-bridge…"
  sudo cp "${ARM_DIR}/mqtt-bridge" /usr/local/bin/mqtt-bridge
  sudo chmod 755 /usr/local/bin/mqtt-bridge

  if [ ! -f /etc/systemd/system/mqtt-bridge.service ]; then
    sudo cp contrib/mqtt-bridge.service /etc/systemd/system/
    sudo systemctl daemon-reload
    sudo systemctl enable mqtt-bridge
    info "Service mqtt-bridge enregistré"
  fi

  sudo systemctl start mqtt-bridge
  sleep 3

  if systemctl is-active --quiet mqtt-bridge; then
    info "mqtt-bridge actif (Pi5 :1883 ↔ NanoPi 192.168.1.120:1883, metrics :8084)"
  else
    warn "mqtt-bridge n'a pas démarré — vérifier : journalctl -u mqtt-bridge -n 30"
    warn "  (Normal si NanoPi inaccessible — le bridge reconnectera automatiquement)"
  fi
fi

if [ "$MQTT_ONLY" = true ]; then
  echo ""
  echo -e "${GREEN}═══════════════════════════════════════${NC}"
  echo -e "${GREEN}  MQTT déployé avec succès ✓${NC}"
  echo -e "${GREEN}═══════════════════════════════════════${NC}"
  systemctl status mqtt-broker mqtt-bridge --no-pager | grep -E "Active:|Loaded:" || true
  exit 0
fi

# ── 7. Déploiement daly-bms-server ───────────────────────────────────────────
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
VM_LOG=$(journalctl -u daly-bms --since "30 seconds ago" 2>/dev/null | grep -i victoriametrics | head -3)
if echo "${VM_LOG}" | grep -q "activé"; then
    info "VictoriaMetrics initialisé : ${VM_DIR}"
elif echo "${VM_LOG}" | grep -q "échoué\|error\|Error"; then
    warn "VictoriaMetrics init a échoué — voir : journalctl -u daly-bms -n 30 | grep -i victoriametrics"
else
    warn "VictoriaMetrics : aucun message de démarrage détecté (vérifier la config)"
fi

# ── 8. Déploiement energy-manager ────────────────────────────────────────────
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

# ── 9. Résumé ─────────────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}═══════════════════════════════════════════${NC}"
echo -e "${GREEN}  Déploiement complet terminé avec succès ✓${NC}"
echo -e "${GREEN}═══════════════════════════════════════════${NC}"
echo ""
systemctl status daly-bms energy-manager mqtt-broker mqtt-bridge --no-pager -l \
  | grep -E "Active:|Loaded:" || true
echo ""
step "Vérification VictoriaMetrics via API…"
sleep 1
curl -sf 'http://localhost:8428/-/healthy' 2>/dev/null || \
    warn "Endpoint VictoriaMetrics non accessible"
echo ""
step "Vérification MQTT broker…"
curl -sf 'http://localhost:8082/health' 2>/dev/null && info "Broker MQTT actif" || \
    warn "Endpoint métriques broker non accessible (démarrage en cours ?)"
echo ""
step "Vérification MQTT bridge…"
curl -sf 'http://localhost:8084/health' 2>/dev/null && info "Bridge MQTT actif" || \
    warn "Endpoint métriques bridge non accessible"
