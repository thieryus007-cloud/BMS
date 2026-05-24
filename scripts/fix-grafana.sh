#!/usr/bin/env bash
# =============================================================================
# scripts/fix-grafana.sh
#
# Réparation complète de Grafana : nettoie le provisioning résiduel,
# réinitialise la base SQLite, et redéploie les 15 dashboards.
#
# Usage :
#   sudo bash scripts/fix-grafana.sh
# =============================================================================
set -euo pipefail

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; CYAN='\033[0;36m'; NC='\033[0m'
info()  { echo -e "${GREEN}[OK]${NC} $*"; }
step()  { echo -e "${CYAN}[>>]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!!]${NC} $*"; }
error() { echo -e "${RED}[XX]${NC} $*" >&2; exit 1; }

[[ $EUID -eq 0 ]] || error "Ce script doit être exécuté en root : sudo bash $0"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# ── 1. Arrêter Grafana ──────────────────────────────────────────────────────
step "Arrêt de grafana-server…"
systemctl stop grafana-server 2>/dev/null || true
sleep 1
info "grafana-server arrêté"

# ── 2. Identifier le data dir Grafana ────────────────────────────────────────
GRAFANA_DATA=""
for candidate in /mnt/nvme/grafana/data /var/lib/grafana; do
    if [[ -d "$candidate" ]]; then
        GRAFANA_DATA="$candidate"
        break
    fi
done
[[ -n "$GRAFANA_DATA" ]] || error "Data dir Grafana introuvable"
info "Data dir Grafana : $GRAFANA_DATA"

# ── 3. Sauvegarder puis supprimer la base SQLite corrompue ──────────────────
step "Réinitialisation de la base Grafana…"
if [[ -f "$GRAFANA_DATA/grafana.db" ]]; then
    BACKUP="$GRAFANA_DATA/grafana.db.backup-$(date +%Y%m%d_%H%M%S)"
    cp "$GRAFANA_DATA/grafana.db" "$BACKUP"
    info "Backup → $BACKUP"
    rm -f "$GRAFANA_DATA/grafana.db"
    rm -f "$GRAFANA_DATA/grafana.db-wal"
    rm -f "$GRAFANA_DATA/grafana.db-shm"
    info "Base SQLite supprimée (sera recréée au démarrage)"
else
    info "Pas de base SQLite existante"
fi

# ── 4. Nettoyer TOUT le provisioning résiduel ───────────────────────────────
step "Nettoyage du provisioning…"

# Datasources — supprimer TOUT puis remettre uniquement daly-metrics
rm -f /etc/grafana/provisioning/datasources/*.yaml
rm -f /etc/grafana/provisioning/datasources/*.yml
install -d -m 755 -o root -g grafana /etc/grafana/provisioning/datasources
install -m 644 -o root -g grafana \
    "$REPO_ROOT/contrib/grafana/provisioning/datasources/daly-metrics.yaml" \
    /etc/grafana/provisioning/datasources/
info "Datasource : daly-metrics.yaml (unique)"

# Dashboards provider — supprimer TOUT puis remettre uniquement daly-bms
rm -f /etc/grafana/provisioning/dashboards/*.yaml
rm -f /etc/grafana/provisioning/dashboards/*.yml
install -d -m 755 -o root -g grafana /etc/grafana/provisioning/dashboards
install -m 644 -o root -g grafana \
    "$REPO_ROOT/contrib/grafana/provisioning/dashboards/daly-bms.yaml" \
    /etc/grafana/provisioning/dashboards/
info "Provider : daly-bms.yaml (unique)"

# Supprimer le faux répertoire /etc/grafana/dashboards (ancien résidu)
rm -rf /etc/grafana/dashboards

# ── 5. Déployer les 15 dashboards JSON ──────────────────────────────────────
step "Déploiement des 15 dashboards…"
install -d -m 755 -o grafana -g grafana /var/lib/grafana/dashboards
rm -f /var/lib/grafana/dashboards/*.json
install -m 644 -o grafana -g grafana \
    "$REPO_ROOT/contrib/grafana/dashboards/"*.json \
    /var/lib/grafana/dashboards/
N_DASH=$(ls /var/lib/grafana/dashboards/*.json 2>/dev/null | wc -l)
info "$N_DASH dashboards copiés dans /var/lib/grafana/dashboards/"

# ── 6. Réparer les permissions ──────────────────────────────────────────────
step "Réparation des permissions…"
chown -R grafana:grafana "$GRAFANA_DATA"
chown -R grafana:grafana /var/lib/grafana/dashboards
info "Permissions OK"

# ── 7. Vérification du provisioning ─────────────────────────────────────────
step "Vérification pré-démarrage…"
echo "  Datasources :"
ls -la /etc/grafana/provisioning/datasources/
echo "  Providers :"
ls -la /etc/grafana/provisioning/dashboards/
echo "  Dashboards :"
ls /var/lib/grafana/dashboards/ | head -20
echo ""

# ── 8. Démarrer Grafana ─────────────────────────────────────────────────────
step "Démarrage de grafana-server…"
systemctl start grafana-server
sleep 5

if ! systemctl is-active --quiet grafana-server; then
    error "grafana-server ne démarre pas — voir : journalctl -u grafana-server -n 50"
fi
info "grafana-server actif"

# ── 9. Vérifier le provisioning dans les logs ───────────────────────────────
step "Vérification des logs de démarrage…"
LOGS=$(journalctl -u grafana-server --since "15 seconds ago" --no-pager 2>/dev/null)

# Compter les dashboards provisionnés
PROV_OK=$(echo "$LOGS" | grep -c "provisioning.*dashboard" || echo "0")
PROV_ERR=$(echo "$LOGS" | grep -c "restricted database access" || echo "0")

if [[ "$PROV_ERR" -gt 0 ]]; then
    warn "$PROV_ERR erreurs 'restricted database access' détectées"
    warn "Logs :"
    echo "$LOGS" | grep -i "restricted\|error\|warn" | tail -10
else
    info "Aucune erreur de provisioning"
fi

# ── 10. Test API (credentials par défaut après reset) ───────────────────────
step "Test API Grafana (admin/admin)…"
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -u admin:admin http://localhost:3000/api/search 2>/dev/null || echo "000")
if [[ "$HTTP_CODE" == "200" ]]; then
    VISIBLE=$(curl -s -u admin:admin 'http://localhost:3000/api/search?type=dash-db' 2>/dev/null \
        | python3 -c "import json,sys; print(len(json.load(sys.stdin)))" 2>/dev/null || echo "?")
    info "Dashboards visibles via API : $VISIBLE"
elif [[ "$HTTP_CODE" == "401" ]]; then
    warn "API retourne 401 — mot de passe admin modifié"
    warn "Pour le réinitialiser : sudo grafana-cli admin reset-admin-password admin"
else
    warn "API retourne HTTP $HTTP_CODE"
fi

# ── Résumé ──────────────────────────────────────────────────────────────────
IP="$(hostname -I 2>/dev/null | awk '{print $1}')"
echo ""
echo -e "${GREEN}══════════════════════════════════════════════════${NC}"
echo -e "${GREEN}  Grafana réparé${NC}"
echo -e "${GREEN}══════════════════════════════════════════════════${NC}"
echo ""
echo "  URL       : http://${IP:-localhost}:3000"
echo "  Login     : admin / admin (mot de passe par défaut après reset)"
echo "  Dossier   : Daly-BMS (15 dashboards)"
echo "  Datasource: Daly Metrics (redb) → http://127.0.0.1:8080"
echo ""
echo "  Si le mot de passe a été changé :"
echo "    sudo grafana-cli admin reset-admin-password admin"
echo ""
