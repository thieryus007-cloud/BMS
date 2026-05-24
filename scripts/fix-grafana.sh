#!/usr/bin/env bash
# =============================================================================
# scripts/fix-grafana.sh
#
# Réparation et déploiement définitif des 15 dashboards Grafana.
#
# Stratégie : import via l'API HTTP Grafana (contourne le bug
# "restricted database access" du provisioning fichier Grafana 11+).
# La datasource reste provisionnée par fichier (pas affectée par ce bug).
#
# Usage :
#   sudo bash scripts/fix-grafana.sh
#   sudo bash scripts/fix-grafana.sh --password MyPass123
# =============================================================================
set -euo pipefail

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; CYAN='\033[0;36m'; NC='\033[0m'
info()  { echo -e "${GREEN}[OK]${NC} $*"; }
step()  { echo -e "${CYAN}[>>]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!!]${NC} $*"; }
error() { echo -e "${RED}[XX]${NC} $*" >&2; exit 1; }

[[ $EUID -eq 0 ]] || error "Ce script doit être exécuté en root : sudo $0"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

ADMIN_PWD="admin"
for arg in "$@"; do
    case "$arg" in
        --password=*) ADMIN_PWD="${arg#*=}" ;;
    esac
done

GRAFANA_URL="http://localhost:3000"
GRAFANA_AUTH="admin:${ADMIN_PWD}"
DASHBOARD_DIR="$REPO_ROOT/contrib/grafana/dashboards"

# ── 1. S'assurer que Grafana tourne ─────────────────────────────────────────
step "Vérification que Grafana est démarré…"
if ! systemctl is-active --quiet grafana-server; then
    systemctl start grafana-server
    sleep 5
fi
if ! systemctl is-active --quiet grafana-server; then
    error "grafana-server ne démarre pas — voir : journalctl -u grafana-server -n 50"
fi
info "grafana-server actif"

# ── 2. Réinitialiser le mot de passe admin ──────────────────────────────────
step "Réinitialisation du mot de passe admin…"
grafana-cli admin reset-admin-password "$ADMIN_PWD" 2>/dev/null || true
sleep 2
info "Mot de passe admin = $ADMIN_PWD"

# ── 3. Attendre que l'API soit prête ────────────────────────────────────────
step "Attente de l'API Grafana…"
for i in $(seq 1 15); do
    HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
        -u "$GRAFANA_AUTH" "$GRAFANA_URL/api/health" 2>/dev/null || echo "000")
    if [[ "$HTTP_CODE" == "200" ]]; then
        break
    fi
    if [[ $i -eq 15 ]]; then
        error "API Grafana injoignable après 15 tentatives (HTTP $HTTP_CODE)"
    fi
    sleep 2
done
info "API Grafana opérationnelle"

# ── 4. Déployer la datasource (par fichier — ça fonctionne) ────────────────
step "Déploiement datasource daly-metrics.yaml…"
rm -f /etc/grafana/provisioning/datasources/victoriametrics.yaml
rm -f /etc/grafana/provisioning/datasources/redb.yaml
install -d -m 755 -o root -g grafana /etc/grafana/provisioning/datasources
install -m 644 -o root -g grafana \
    "$REPO_ROOT/contrib/grafana/provisioning/datasources/daly-metrics.yaml" \
    /etc/grafana/provisioning/datasources/
info "Datasource provisionnée"

# ── 5. Supprimer le provisioning fichier des dashboards ─────────────────────
# On supprime le provider YAML pour éviter le conflit entre provisioning
# fichier (qui échoue) et API import (qui fonctionne).
step "Suppression du provisioning fichier des dashboards…"
rm -f /etc/grafana/provisioning/dashboards/daly-bms.yaml
rm -f /etc/grafana/provisioning/dashboards/dashboards.yaml
rm -rf /etc/grafana/dashboards
info "Provisioning fichier désactivé"

# ── 6. Créer/trouver le dossier "Daly-BMS" via API ─────────────────────────
step "Création du dossier Daly-BMS…"
FOLDER_UID="daly-bms-folder"
FOLDER_RESPONSE=$(curl -s -u "$GRAFANA_AUTH" \
    "$GRAFANA_URL/api/folders/$FOLDER_UID" 2>/dev/null)
FOLDER_ID=$(echo "$FOLDER_RESPONSE" | python3 -c "
import json,sys
try:
    d = json.load(sys.stdin)
    if 'id' in d and d.get('uid') == '$FOLDER_UID':
        print(d['id'])
    else:
        print('')
except: print('')
" 2>/dev/null)

if [[ -z "$FOLDER_ID" ]]; then
    FOLDER_RESPONSE=$(curl -s -u "$GRAFANA_AUTH" \
        -H "Content-Type: application/json" \
        -X POST "$GRAFANA_URL/api/folders" \
        -d "{\"uid\":\"$FOLDER_UID\",\"title\":\"Daly-BMS\"}" 2>/dev/null)
    FOLDER_ID=$(echo "$FOLDER_RESPONSE" | python3 -c "
import json,sys
try: print(json.load(sys.stdin).get('id',''))
except: print('')
" 2>/dev/null)
    if [[ -z "$FOLDER_ID" ]]; then
        error "Impossible de créer le dossier Daly-BMS : $FOLDER_RESPONSE"
    fi
    info "Dossier Daly-BMS créé (id=$FOLDER_ID)"
else
    info "Dossier Daly-BMS existe (id=$FOLDER_ID)"
fi

# ── 7. Importer les 15 dashboards via API ───────────────────────────────────
step "Import des 15 dashboards via API…"
IMPORTED=0
FAILED=0
for json_file in "$DASHBOARD_DIR"/*.json; do
    BASENAME=$(basename "$json_file")

    # Construire le payload d'import API
    PAYLOAD=$(python3 -c "
import json, sys
with open('$json_file') as f:
    dash = json.load(f)
# Forcer id=null pour que Grafana le crée/mette à jour par uid
dash['id'] = None
payload = {
    'dashboard': dash,
    'folderUid': '$FOLDER_UID',
    'overwrite': True,
    'message': 'Deployed by fix-grafana.sh'
}
print(json.dumps(payload))
" 2>/dev/null)

    if [[ -z "$PAYLOAD" ]]; then
        echo -e "  ${RED}FAIL${NC} $BASENAME (erreur lecture JSON)"
        FAILED=$((FAILED+1))
        continue
    fi

    RESPONSE=$(curl -s -u "$GRAFANA_AUTH" \
        -H "Content-Type: application/json" \
        -X POST "$GRAFANA_URL/api/dashboards/db" \
        -d "$PAYLOAD" 2>/dev/null)

    STATUS=$(echo "$RESPONSE" | python3 -c "
import json,sys
try:
    d = json.load(sys.stdin)
    print(d.get('status','') or d.get('slug','ok'))
except: print('error')
" 2>/dev/null)

    if [[ "$STATUS" == "success" || "$STATUS" != "error" ]]; then
        TITLE=$(echo "$RESPONSE" | python3 -c "
import json,sys
try: print(json.load(sys.stdin).get('slug','?'))
except: print('?')
" 2>/dev/null)
        echo -e "  ${GREEN}OK${NC}   $BASENAME → $TITLE"
        IMPORTED=$((IMPORTED+1))
    else
        MSG=$(echo "$RESPONSE" | python3 -c "
import json,sys
try: print(json.load(sys.stdin).get('message','?'))
except: print('?')
" 2>/dev/null)
        echo -e "  ${RED}FAIL${NC} $BASENAME : $MSG"
        FAILED=$((FAILED+1))
    fi
done

echo ""
if [[ $FAILED -eq 0 ]]; then
    info "Tous les $IMPORTED dashboards importés avec succès"
else
    warn "$IMPORTED importés, $FAILED en échec"
fi

# ── 8. Vérification finale ──────────────────────────────────────────────────
step "Vérification finale…"
VISIBLE=$(curl -s -u "$GRAFANA_AUTH" \
    "$GRAFANA_URL/api/search?type=dash-db&folderIds=$FOLDER_ID" 2>/dev/null \
    | python3 -c "import json,sys; print(len(json.load(sys.stdin)))" 2>/dev/null || echo "?")
info "Dashboards visibles dans le dossier Daly-BMS : $VISIBLE"

# Recharger Grafana pour prendre en compte la datasource
systemctl reload grafana-server 2>/dev/null || systemctl restart grafana-server

# ── Résumé ──────────────────────────────────────────────────────────────────
IP="$(hostname -I 2>/dev/null | awk '{print $1}')"
echo ""
echo -e "${GREEN}══════════════════════════════════════════════════${NC}"
echo -e "${GREEN}  Grafana — 15 dashboards déployés${NC}"
echo -e "${GREEN}══════════════════════════════════════════════════${NC}"
echo ""
echo "  URL        : http://${IP:-localhost}:3000"
echo "  Login      : admin / $ADMIN_PWD"
echo "  Dossier    : Daly-BMS ($VISIBLE dashboards)"
echo "  Datasource : Daly Metrics (redb) → http://127.0.0.1:8080"
echo ""
echo "  Re-déployer les dashboards :"
echo "    sudo bash scripts/fix-grafana.sh"
echo ""
