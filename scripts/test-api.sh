#!/usr/bin/env bash
# Tests d'intégration API daly-bms-server (post-déploiement).
#
# Couvre tous les endpoints critiques utilisés par :
# - Grafana (dispatcher /api/v1/query, /api/v1/query_range, /api/v1/labels)
# - Dashboard custom interne (/api/v1/chart/*, /api/v1/history/*, /api/v1/dashboards/*)
# - Endpoints redb directs (/api/v1/redb/*, /-/healthy)
#
# Exit code 0 = tous les tests passent. Non-zero = au moins un échec.
#
# Usage :
#   bash scripts/test-api.sh              # tests prod (127.0.0.1:8080)
#   bash scripts/test-api.sh HOST:PORT    # cible alternative
#   bash scripts/test-api.sh -v           # verbose (affiche payloads)

set -uo pipefail   # PAS -e : on veut continuer même si un test fail
                   # pour compter tous les échecs

HOST="${1:-127.0.0.1:8080}"
case "$HOST" in
    -v|--verbose) VERBOSE=true; HOST="127.0.0.1:8080" ;;
    *) VERBOSE=false ;;
esac
[[ "${2:-}" == "-v" || "${2:-}" == "--verbose" ]] && VERBOSE=true

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; CYAN='\033[0;36m'; NC='\033[0m'

PASS=0
FAIL=0
SKIP=0
FAIL_NAMES=()

ok()   { echo -e "  ${GREEN}✓${NC} $*"; PASS=$((PASS + 1)); }
ko()   { echo -e "  ${RED}✗${NC} $*"; FAIL=$((FAIL + 1)); FAIL_NAMES+=("$1"); }
skip() { echo -e "  ${YELLOW}↷${NC} $* (skip)"; SKIP=$((SKIP + 1)); }
section() { echo ""; echo -e "${CYAN}── $* ──${NC}"; }

# Helper : curl avec timeout + capture status + body
http() {
    local path="$1"; shift
    local out
    out=$(curl -s -o /tmp/test-api.body -w '%{http_code}' --max-time 10 \
        "http://${HOST}${path}" "$@" 2>/dev/null || echo "000")
    echo "$out"
}

# Helper : assert status code + JSON path non-vide / égal
assert_200() {
    local name="$1" path="$2"
    local code; code=$(http "$path")
    if [[ "$code" == "200" ]]; then
        ok "$name → 200"
        $VERBOSE && head -c 200 /tmp/test-api.body && echo
    else
        ko "$name" "$path → HTTP $code"
    fi
}

assert_json_field() {
    local name="$1" path="$2" jq_expr="$3" expected="$4"
    local code; code=$(http "$path")
    if [[ "$code" != "200" ]]; then
        ko "$name" "$path → HTTP $code (attendu 200)"
        return
    fi
    local value; value=$(jq -r "$jq_expr" /tmp/test-api.body 2>/dev/null || echo "PARSE_ERROR")
    if [[ "$value" == "$expected" ]]; then
        ok "$name → $jq_expr = $expected"
    else
        ko "$name" "$path : $jq_expr = '$value' (attendu '$expected')"
        $VERBOSE && head -c 300 /tmp/test-api.body && echo
    fi
}

assert_json_nonempty() {
    local name="$1" path="$2" jq_expr="$3"
    local code; code=$(http "$path")
    if [[ "$code" != "200" ]]; then
        ko "$name" "$path → HTTP $code"
        return
    fi
    local count; count=$(jq -r "$jq_expr" /tmp/test-api.body 2>/dev/null || echo 0)
    if [[ "$count" =~ ^[0-9]+$ ]] && [[ "$count" -gt 0 ]]; then
        ok "$name → $jq_expr = $count (non-vide)"
    else
        ko "$name" "$path : $jq_expr = $count (attendu >0)"
        $VERBOSE && head -c 300 /tmp/test-api.body && echo
    fi
}

# ── Pré-requis ────────────────────────────────────────────────────────────────
section "Pré-requis"
command -v jq >/dev/null 2>&1 || { echo "jq requis (sudo apt install jq)"; exit 2; }
ok "jq disponible"

# ── 1. Healthchecks ──────────────────────────────────────────────────────────
section "1. Healthchecks"
assert_200 "GET /health"                  "/health"
assert_200 "GET /-/healthy"               "/-/healthy"
assert_200 "GET /api/v1/redb/healthy"     "/api/v1/redb/healthy"

# ── 2. Catalogue redb ────────────────────────────────────────────────────────
section "2. Catalogue redb (series + labels)"
assert_json_nonempty "GET /api/v1/redb/series"               "/api/v1/redb/series"             '.data | length'
assert_json_nonempty "GET /api/v1/redb/labels"               "/api/v1/redb/labels"             '.data | length'
assert_json_nonempty "GET /api/v1/redb/label/__name__/values" "/api/v1/redb/label/__name__/values" '.data | length'
assert_json_nonempty "GET /api/v1/series"                    "/api/v1/series"                  '.data | length'
assert_json_nonempty "GET /api/v1/labels"                    "/api/v1/labels"                  '.data | length'

# ── 3. Dispatcher PromQL — GET ───────────────────────────────────────────────
section "3. Dispatcher PromQL (GET — Grafana-facing)"
QUERY_BMS="bms_voltage"
QUERY_SOLAR="solar_total_w"

assert_json_field "GET /api/v1/query?query=$QUERY_BMS"   \
    "/api/v1/query?query=${QUERY_BMS}" '.status' 'success'

assert_json_nonempty "GET /api/v1/query (résultat non-vide)" \
    "/api/v1/query?query=${QUERY_BMS}" '.data.result | length'

NOW=$(date +%s)
FROM=$((NOW - 3600))
assert_json_field "GET /api/v1/query_range?query=$QUERY_BMS (1h)" \
    "/api/v1/query_range?query=${QUERY_BMS}&start=${FROM}&end=${NOW}&step=60" '.status' 'success'

assert_json_nonempty "GET /api/v1/query_range (résultat non-vide, 1h)" \
    "/api/v1/query_range?query=${QUERY_BMS}&start=${FROM}&end=${NOW}&step=60" '.data.result | length'

# ── 4. Dispatcher PromQL — POST (Grafana httpMethod=POST par défaut) ────────
section "4. Dispatcher PromQL (POST)"
code=$(curl -s -o /tmp/test-api.body -w '%{http_code}' --max-time 10 \
    -X POST "http://${HOST}/api/v1/query" \
    --data-urlencode "query=${QUERY_BMS}" 2>/dev/null || echo "000")
if [[ "$code" == "200" ]]; then
    status=$(jq -r '.status' /tmp/test-api.body 2>/dev/null)
    if [[ "$status" == "success" ]]; then
        ok "POST /api/v1/query → status=success"
    else
        ko "POST /api/v1/query" "status='$status' au lieu de 'success'"
    fi
else
    ko "POST /api/v1/query" "HTTP $code"
fi

code=$(curl -s -o /tmp/test-api.body -w '%{http_code}' --max-time 10 \
    -X POST "http://${HOST}/api/v1/query_range" \
    --data-urlencode "query=${QUERY_SOLAR}" \
    --data-urlencode "start=${FROM}" \
    --data-urlencode "end=${NOW}" \
    --data-urlencode "step=60" 2>/dev/null || echo "000")
if [[ "$code" == "200" ]]; then
    status=$(jq -r '.status' /tmp/test-api.body 2>/dev/null)
    [[ "$status" == "success" ]] \
        && ok "POST /api/v1/query_range → status=success" \
        || ko "POST /api/v1/query_range" "status='$status'"
else
    ko "POST /api/v1/query_range" "HTTP $code"
fi

# ── 5. Chart history (graphes internes) ─────────────────────────────────────
# NB 2026-06 : /dashboard/history et /api/v1/history/energy ont été retirés
# (Grafana = unique outil d'historique).
section "5. Routes chart history"
assert_json_nonempty "GET /api/v1/chart/history?minutes=60" \
    "/api/v1/chart/history?minutes=60" '.solar | length'
assert_json_field "GET /api/v1/chart/history?minutes=60 (.ok)" \
    "/api/v1/chart/history?minutes=60" '.ok // true' 'true'

# ── 6. PromQL avec label matcher (Grafana panels) ───────────────────────────
section "6. PromQL avec label matchers"
QM=$(printf '%s' 'bms_voltage{bms_id="0x01"}' | jq -sRr @uri 2>/dev/null)
assert_json_nonempty "GET /api/v1/query?query=bms_voltage{bms_id=\"0x01\"}" \
    "/api/v1/query?query=${QM}" '.data.result | length'

QM=$(printf '%s' 'increase(et112_energy_export_wh{address="0x09"}[1h])' | jq -sRr @uri 2>/dev/null)
NOW=$(date +%s); FROM=$((NOW - 7200))
assert_json_field "GET /api/v1/query_range (increase 1h, 2h window)" \
    "/api/v1/query_range?query=${QM}&start=${FROM}&end=${NOW}&step=300" '.status' 'success'

# ── 7. PromQL erreur attendue (vérifie que le shim rejette correctement) ────
section "7. Rejet de constructions non supportées"
QM=$(printf '%s' 'histogram_quantile(0.95, foo)' | jq -sRr @uri 2>/dev/null)
code=$(http "/api/v1/query?query=${QM}")
status=$(jq -r '.status' /tmp/test-api.body 2>/dev/null || echo none)
errtype=$(jq -r '.errorType // .error_type // "?"' /tmp/test-api.body 2>/dev/null)
if [[ "$status" == "error" && "$errtype" == "bad_data" ]]; then
    ok "histogram_quantile() correctement rejeté ($status / $errtype)"
else
    ko "rejet histogram_quantile" "HTTP $code status='$status' errorType='$errtype'"
fi

# ── 8. Health checks redb backend ────────────────────────────────────────────
section "8. Sanity backend redb"
DB_PATH=$(grep -E '^db_path' /etc/daly-bms/config.toml 2>/dev/null \
    | sed -E 's/.*=\s*"([^"]+)".*/\1/' | head -1)
DB_PATH="${DB_PATH:-/mnt/nvme/daly-bms/metrics.redb}"
if [[ -f "$DB_PATH" ]]; then
    SIZE=$(du -sh "$DB_PATH" | cut -f1)
    AGE=$(stat -c '%Y' "$DB_PATH")
    NOW=$(date +%s)
    DELTA=$((NOW - AGE))
    ok "Base redb $DB_PATH existe ($SIZE)"
    if [[ "$DELTA" -lt 300 ]]; then
        ok "Base redb mtime récent (${DELTA}s) — writer actif"
    else
        ko "redb mtime" "fichier pas écrit depuis ${DELTA}s — writer mort ?"
    fi
else
    skip "Base redb $DB_PATH introuvable"
fi

# ── Résumé ────────────────────────────────────────────────────────────────────
echo ""
echo "═══════════════════════════════════════"
TOTAL=$((PASS + FAIL + SKIP))
if [[ $FAIL -eq 0 ]]; then
    echo -e "${GREEN}  Résultat : $PASS/$TOTAL OK ✓${NC}  (skip: $SKIP)"
    exit 0
else
    echo -e "${RED}  Résultat : $FAIL/$TOTAL échec(s) ✗${NC}  (ok: $PASS, skip: $SKIP)"
    echo ""
    echo -e "${RED}Tests en échec :${NC}"
    for n in "${FAIL_NAMES[@]}"; do
        echo "  - $n"
    done
    exit 1
fi
