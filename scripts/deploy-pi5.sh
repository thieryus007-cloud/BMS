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
# - VictoriaMetrics retiré (Phase 5 cleanup). Le script désactive le
#   service victoriametrics.service s'il tourne encore et archive le
#   data dir.
# - Le dashboard custom `/dashboard/history` lit aussi metrics-store
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

# ── 3. Déploiement Config.toml (uniquement si /etc/daly-bms/config.toml absent) ──
#
# IMPORTANT — le Config.toml du repo est un TEMPLATE par défaut destiné à
# l'initialisation. Sur un Pi5 déjà en prod, la config a été calibrée par
# l'opérateur (cache_mb, enabled flags, etc.) ; la copier aveuglément
# ÉCRASERAIT ces réglages. On NE copie donc que si le fichier de
# destination n'existe pas (bootstrap initial).
#
# Pour appliquer des changements de config en cours d'exploitation, l'opérateur
# édite /etc/daly-bms/config.toml manuellement OU passe explicitement par :
#   sudo cp Config.toml /etc/daly-bms/config.toml   # ÉCRASE
CONF=/etc/daly-bms/config.toml
if [[ ! -f "$CONF" ]]; then
    step "Première installation : copie Config.toml → $CONF…"
    sudo mkdir -p /etc/daly-bms
    sudo cp Config.toml "$CONF"
    info "Config.toml initialisée"
else
    info "$CONF existe déjà — préservé (pas d'écrasement)"
fi

# ── 3.bis Auto-réparation des flags critiques (post-Phase 5.1) ───────────────
# metrics-store est maintenant la SEULE TSDB de lecture — si elle est
# désactivée par accident (cp Config.toml, sed mal ciblé, etc.), le service
# n'a plus de backend et le dashboard custom + Grafana retournent vide.
# Cette étape détecte ces cas et applique la réparation automatiquement
# (avec backup horodaté).
step "Vérification config critique (auto-répare si besoin)…"

# Check 1 : section [metrics_store] présente
if ! grep -q '^\[metrics_store\]' "$CONF"; then
    error "Section [metrics_store] absente de $CONF — éditer manuellement avant de relancer"
fi

# Check 2 : metrics_store.enabled = true
METRICS_ENABLED=$(awk '/^\[metrics_store\]/,/^\[/' "$CONF" \
    | grep -E '^enabled' | sed -E 's/.*=\s*(true|false).*/\1/' | head -1)
if [[ "$METRICS_ENABLED" != "true" ]]; then
    BACKUP="$CONF.before-autorepair-$(date +%Y%m%d_%H%M%S)"
    warn "[metrics_store].enabled = '$METRICS_ENABLED' — réparation en cours (backup → $BACKUP)"
    sudo cp "$CONF" "$BACKUP"
    sudo sed -i '/^\[metrics_store\]/,/^\[/ {s/^enabled = false$/enabled = true/}' "$CONF"
    METRICS_ENABLED=$(awk '/^\[metrics_store\]/,/^\[/' "$CONF" \
        | grep -E '^enabled' | sed -E 's/.*=\s*(true|false).*/\1/' | head -1)
    if [[ "$METRICS_ENABLED" != "true" ]]; then
        error "Échec de la réparation [metrics_store].enabled — éditer manuellement $CONF"
    fi
    info "[metrics_store].enabled forcé à true ✓"
else
    info "[metrics_store].enabled = true ✓"
fi

# Check 3 : db_path résolvable, parent writable par dalybms
DB_PATH=$(grep -E '^db_path' "$CONF" | sed -E 's/.*=\s*"([^"]+)".*/\1/' | head -1)
DB_PATH="${DB_PATH:-/mnt/nvme/daly-bms/metrics.redb}"
DB_DIR=$(dirname "$DB_PATH")
if [[ ! -d "$DB_DIR" ]]; then
    warn "Répertoire $DB_DIR absent — création"
fi
DALY_USER=$(systemctl show daly-bms --property=User --value 2>/dev/null || echo dalybms)
DALY_USER="${DALY_USER:-dalybms}"
sudo mkdir -p "$DB_DIR"
sudo chown "$DALY_USER:$DALY_USER" "$DB_DIR"
info "db_path = $DB_PATH (parent owner=$DALY_USER) ✓"

# ── 4. NVMe monté ? ──────────────────────────────────────────────────────────
mountpoint -q /mnt/nvme || warn "/mnt/nvme non monté — vérifier /etc/fstab"

# Phase 5 cleanup : VictoriaMetrics retiré du code (commit 5aa9b89+).
# Si le service victoriametrics tourne encore sur ce Pi5, le désactiver
# explicitement pour libérer ~135 Mo de RSS et le port :8428.
if systemctl is-active --quiet victoriametrics 2>/dev/null; then
    warn "victoriametrics.service est encore actif — désactivation (Phase 5 retrait)"
    sudo systemctl stop victoriametrics
    sudo systemctl disable victoriametrics
    info "victoriametrics.service stoppé + disabled"
    # /mnt/nvme/victoria-metrics conservé en lecture seule pour rollback
    if [[ -d /mnt/nvme/victoria-metrics ]]; then
        sudo mv /mnt/nvme/victoria-metrics /mnt/nvme/victoria-metrics.archive-$(date +%Y%m%d) 2>/dev/null || true
        info "Data dir VM archivée (rollback possible pendant 30 j)"
    fi
else
    info "VictoriaMetrics non installé / déjà retiré"
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
echo "$BOOT_LOG" | grep -q 'metrics-store ouvert' && info "metrics-store ouvert ✓" || warn "metrics-store : init non détectée"

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
