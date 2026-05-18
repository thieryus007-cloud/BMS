#!/usr/bin/env bash
# Déploiement Pi5 — daly-bms-server + energy-manager + validation
#
# Usage (depuis ~/Daly-BMS-Rust sur le Pi5) :
#   bash scripts/deploy-pi5.sh                  # full deploy + validation
#   bash scripts/deploy-pi5.sh --no-build       # skip make build-arm
#   bash scripts/deploy-pi5.sh --no-validate    # skip script test-api.sh
#
# Architecture post-migration redb (cf. docs/plan_migration_vm_redb.md) :
# - metrics-store (redb à /mnt/nvme/daly-bms/metrics.redb) est la TSDB
#   primaire, sert toutes les lectures via le dispatcher PromQL local
#   (`/api/v1/query[_range]`, `/api/v1/chart/*`, `/api/v1/history/*`,
#    `/api/v1/dashboards/*`, `/api/v1/redb/*`).
# - VictoriaMetrics est optionnel (dual-write si encore activé dans
#   Config.toml `[victoriametrics] enabled = true`).
# - Le dashboard custom `/dashboard/history` passe par le dispatcher
#   donc suit le toggle `[metrics_store].default_backend`.

set -euo pipefail

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; CYAN='\033[0;36m'; NC='\033[0m'
info()  { echo -e "${GREEN}[OK]${NC} $*"; }
step()  { echo -e "${CYAN}[>>]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!!]${NC} $*"; }
error() { echo -e "${RED}[XX]${NC} $*" >&2; exit 1; }

DO_BUILD=true
DO_VALIDATE=true
for arg in "$@"; do
    case "$arg" in
        --no-build)    DO_BUILD=false ;;
        --no-validate) DO_VALIDATE=false ;;
        -h|--help)
            sed -n '/^# Usage/,/^$/p' "$0" | sed 's/^# //'
            exit 0
            ;;
    esac
done

# ── 1. Récupération du code ───────────────────────────────────────────────────
step "Synchronisation du dépôt…"
make sync || error "make sync a échoué"
info "Code à jour"

# ── 2. Compilation croisée aarch64 ───────────────────────────────────────────
if $DO_BUILD; then
    step "Compilation daly-bms-server (aarch64)…"
    make build-arm || error "make build-arm a échoué"
    info "daly-bms-server compilé"

    step "Compilation energy-manager (aarch64)…"
    make build-energy-arm || error "make build-energy-arm a échoué"
    info "energy-manager compilé"
else
    warn "Build skippé (--no-build)"
fi

# ── 3. Déploiement Config.toml (préserve default_backend de l'existant) ─────
step "Déploiement Config.toml → /etc/daly-bms/config.toml…"
EXISTING_BACKEND=""
if [[ -f /etc/daly-bms/config.toml ]]; then
    EXISTING_BACKEND=$(grep -E '^default_backend' /etc/daly-bms/config.toml 2>/dev/null | head -1 || true)
fi
sudo cp Config.toml /etc/daly-bms/config.toml
if [[ -n "$EXISTING_BACKEND" ]]; then
    # Restaure la valeur déployée précédemment (évite de reverter à "vm"
    # si l'opérateur avait basculé en redb)
    sudo sed -i "s|^default_backend = .*|$EXISTING_BACKEND|" /etc/daly-bms/config.toml
    info "Config.toml déployée (default_backend préservé : $EXISTING_BACKEND)"
else
    info "Config.toml déployée"
fi

# ── 4. Répertoires runtime ───────────────────────────────────────────────────
step "Vérification répertoires runtime…"
mountpoint -q /mnt/nvme || warn "/mnt/nvme non monté — vérifier /etc/fstab"

# metrics-store redb
REDB_PATH=$(grep -E '^db_path' /etc/daly-bms/config.toml 2>/dev/null \
    | sed -E 's/.*=\s*"([^"]+)".*/\1/' | head -1)
REDB_PATH="${REDB_PATH:-/mnt/nvme/daly-bms/metrics.redb}"
REDB_DIR=$(dirname "$REDB_PATH")
DALY_USER=$(systemctl show daly-bms --property=User --value 2>/dev/null || echo dalybms)
DALY_USER="${DALY_USER:-dalybms}"
sudo mkdir -p "$REDB_DIR"
sudo chown "$DALY_USER:$DALY_USER" "$REDB_DIR"
info "Répertoire redb : $REDB_DIR (owner=$DALY_USER)"

# VictoriaMetrics (optionnel) — initialisé seulement si activé dans Config.toml
VM_ENABLED=$(awk '/^\[victoriametrics\]/,/^\[/' /etc/daly-bms/config.toml \
    | grep -E '^enabled' | sed -E 's/.*=\s*(true|false).*/\1/' | head -1)
if [[ "$VM_ENABLED" == "true" ]]; then
    VM_DIR="/mnt/nvme/victoria-metrics"
    VM_USER=$(systemctl show victoriametrics --property=User --value 2>/dev/null || echo victoriametrics)
    VM_USER="${VM_USER:-victoriametrics}"
    sudo mkdir -p "$VM_DIR"
    sudo chown "$VM_USER:$VM_USER" "$VM_DIR" 2>/dev/null || true
    info "Répertoire VictoriaMetrics : $VM_DIR (owner=$VM_USER) [dual-write activé]"
else
    info "VictoriaMetrics désactivé dans Config.toml (mode redb seul)"
fi

# ── 5. systemd unit (si modifié dans le repo, redéployer) ───────────────────
if ! sudo diff -q contrib/daly-bms.service /etc/systemd/system/daly-bms.service >/dev/null 2>&1; then
    step "Mise à jour /etc/systemd/system/daly-bms.service…"
    sudo cp contrib/daly-bms.service /etc/systemd/system/
    sudo systemctl daemon-reload
    info "Unit systemd mis à jour"
fi

# ── 6. Déploiement daly-bms-server ───────────────────────────────────────────
step "Déploiement daly-bms-server…"
sudo systemctl stop daly-bms
sudo cp target/aarch64-unknown-linux-gnu/release/daly-bms-server /usr/local/bin/
sudo systemctl start daly-bms
sleep 4

if ! systemctl is-active --quiet daly-bms; then
    error "daly-bms n'a pas démarré — vérifier : journalctl -u daly-bms -n 50"
fi
info "daly-bms actif"

# Inspection des logs de boot
BOOT_LOG=$(journalctl -u daly-bms --since "30 seconds ago" --no-pager 2>/dev/null)
echo "$BOOT_LOG" | grep -q 'metrics-store ouvert'     && info "metrics-store ouvert ✓"      || warn "metrics-store : init non détectée"
echo "$BOOT_LOG" | grep -q 'VictoriaMetrics activé'  && info "VictoriaMetrics activé ✓"   || true
echo "$BOOT_LOG" | grep -q 'dual-write metrics-store' && info "dual-write activé ✓"        || true

# ── 7. Déploiement energy-manager ────────────────────────────────────────────
sleep 2
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

# ── 8. Validation API ────────────────────────────────────────────────────────
if $DO_VALIDATE; then
    echo ""
    step "Validation des endpoints API (test-api.sh)…"
    if [[ -x scripts/test-api.sh ]]; then
        if bash scripts/test-api.sh; then
            info "Validation API : tous les tests passent ✓"
        else
            warn "Validation API : 1+ test(s) en échec — voir sortie ci-dessus"
        fi
    else
        warn "scripts/test-api.sh manquant ou non exécutable — skip validation"
    fi
fi

# ── 9. Résumé ─────────────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}═══════════════════════════════════════${NC}"
echo -e "${GREEN}  Déploiement terminé ✓${NC}"
echo -e "${GREEN}═══════════════════════════════════════${NC}"
echo ""
systemctl status daly-bms energy-manager --no-pager 2>/dev/null \
    | grep -E "^●|Active:" || true
echo ""
if [[ -x scripts/grafana-redb-switch.sh ]]; then
    step "État backends de lecture :"
    sudo scripts/grafana-redb-switch.sh status 2>/dev/null \
        | grep -E "default_backend|url =|Services|Health" || true
fi
