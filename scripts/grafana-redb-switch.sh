#!/usr/bin/env bash
# =============================================================================
# scripts/grafana-redb-switch.sh
#
# Bascule Grafana vers le backend metrics-store (redb) — Phase 3 de la
# migration documentée dans docs/plan_migration_vm_redb.md.
#
# Stratégie : on modifie l'URL de la datasource Prometheus existante
# (uid `victoriametrics`) pour pointer sur daly-bms-server (127.0.0.1:8080)
# au lieu de VictoriaMetrics direct (:8428). L'UID est conservé →
# AUCUN dashboard ni alerte à modifier — ils continuent de référencer le
# même UID. Le dispatcher daly-bms-server (`/api/v1/query[_range]`) route
# vers redb car on positionne aussi `default_backend = "redb"`.
#
# Sous-commandes :
#   switch     Bascule vers redb (backup auto, idempotent)
#   rollback   Restaure l'état pré-bascule depuis le backup le plus récent
#   status     Affiche l'état courant (backend serveur, datasource Grafana)
#
# Pré-requis :
#   - daly-bms-server avec `[metrics_store].enabled = true` et dual-write
#     actif depuis assez longtemps pour que la base contienne toutes les
#     séries que Grafana va requêter.
#   - Grafana installé via systemd (testé sur grafana-server.service).
#   - Exécution avec sudo (ou en root) — modifie /etc/grafana/* et
#     /etc/daly-bms/*.
# =============================================================================
set -euo pipefail

GRAFANA_DS="/etc/grafana/provisioning/datasources/victoriametrics.yaml"
DALYBMS_CONF="/etc/daly-bms/config.toml"
BACKUP_ROOT="/var/backups/migration-redb"
DALYBMS_URL_HEALTHY="http://127.0.0.1:8080/-/healthy"
DALYBMS_URL_QUERY="http://127.0.0.1:8080/api/v1/query?query=up"
GRAFANA_URL="http://127.0.0.1:3000/api/health"

# ── helpers ──────────────────────────────────────────────────────────────────

c_red()   { printf '\033[31m%s\033[0m' "$*"; }
c_green() { printf '\033[32m%s\033[0m' "$*"; }
c_yel()   { printf '\033[33m%s\033[0m' "$*"; }
c_bold()  { printf '\033[1m%s\033[0m' "$*"; }

log()  { echo "[$(date -u +%H:%M:%SZ)] $*"; }
die()  { echo "$(c_red FATAL): $*" >&2; exit 1; }
ok()   { echo "  $(c_green ✓) $*"; }
warn() { echo "  $(c_yel ⚠) $*"; }

require_root() {
    if [[ $EUID -ne 0 ]]; then
        die "Doit être exécuté en root (sudo $0 $*)"
    fi
}

require_files() {
    [[ -f "$GRAFANA_DS"   ]] || die "Datasource Grafana introuvable : $GRAFANA_DS"
    [[ -f "$DALYBMS_CONF" ]] || die "Config daly-bms introuvable : $DALYBMS_CONF"
}

# Retourne la valeur courante de `default_backend` côté daly-bms-server.
current_backend() {
    grep -E '^default_backend[[:space:]]*=' "$DALYBMS_CONF" 2>/dev/null \
        | sed -E 's/.*=\s*"?([^"]+)"?\s*/\1/' \
        | head -1
}

# Retourne l'URL courante côté datasource Grafana.
current_grafana_url() {
    grep -E '^[[:space:]]+url:' "$GRAFANA_DS" 2>/dev/null \
        | head -1 \
        | sed -E 's/.*url:[[:space:]]+//'
}

health_check() {
    local label="$1" url="$2" expected="${3:-200}"
    local code
    code=$(curl -s -o /dev/null -w "%{http_code}" --max-time 5 "$url" || true)
    if [[ "$code" == "$expected" ]]; then
        ok "$label OK ($code)"
        return 0
    fi
    warn "$label retourne $code (attendu $expected) — url=$url"
    return 1
}

latest_backup_dir() {
    ls -1dt "$BACKUP_ROOT"/* 2>/dev/null | head -1
}

# ── sous-commandes ───────────────────────────────────────────────────────────

cmd_status() {
    require_files
    echo "$(c_bold "État de la migration redb")"
    echo
    echo "  daly-bms-server :"
    local backend; backend=$(current_backend)
    if [[ "$backend" == "redb" ]]; then
        echo "    default_backend = $(c_green redb)"
    elif [[ "$backend" == "vm" ]]; then
        echo "    default_backend = $(c_yel vm) (comportement historique)"
    else
        echo "    default_backend = $(c_red "${backend:-absent}") — config inattendue"
    fi
    echo
    echo "  Grafana datasource (uid=victoriametrics) :"
    echo "    url = $(current_grafana_url)"
    echo
    echo "  Services :"
    systemctl is-active daly-bms.service     | sed 's/^/    daly-bms       : /'
    systemctl is-active grafana-server.service | sed 's/^/    grafana-server : /'
    systemctl is-active victoriametrics.service 2>/dev/null | sed 's/^/    victoriametrics: /'
    echo
    echo "  Health-checks :"
    health_check "/-/healthy        " "$DALYBMS_URL_HEALTHY"   || true
    health_check "/api/v1/query?up  " "$DALYBMS_URL_QUERY"     || true
    health_check "Grafana /api/health" "$GRAFANA_URL"          || true
    echo
    local last; last=$(latest_backup_dir || true)
    if [[ -n "${last:-}" ]]; then
        echo "  Dernier backup : $last"
    else
        echo "  Aucun backup trouvé sous $BACKUP_ROOT"
    fi
}

cmd_switch() {
    require_root "$@"
    require_files

    log "Pré-vérification du backend redb..."
    health_check "daly-bms /-/healthy" "$DALYBMS_URL_HEALTHY" || \
        die "daly-bms-server ne répond pas — corriger AVANT de basculer Grafana"

    local cur_backend cur_url
    cur_backend=$(current_backend)
    cur_url=$(current_grafana_url)

    if [[ "$cur_backend" == "redb" ]] && [[ "$cur_url" == "http://127.0.0.1:8080" ]]; then
        log "Déjà en mode redb — rien à faire."
        cmd_status
        return 0
    fi

    local ts
    ts=$(date -u +%Y%m%dT%H%M%SZ)
    local backup_dir="$BACKUP_ROOT/$ts"
    log "Création du backup → $backup_dir"
    mkdir -p "$backup_dir"
    cp -a "$GRAFANA_DS"   "$backup_dir/victoriametrics.yaml"
    cp -a "$DALYBMS_CONF" "$backup_dir/daly-bms-config.toml"
    ok "Backup créé"

    log "Pivot serveur : default_backend → \"redb\""
    sed -i 's/^default_backend = "vm"$/default_backend = "redb"/' "$DALYBMS_CONF"
    grep -q '^default_backend = "redb"' "$DALYBMS_CONF" || \
        die "Modification de $DALYBMS_CONF a échoué"
    ok "/etc/daly-bms/config.toml mis à jour"

    log "Pivot Grafana : datasource url → http://127.0.0.1:8080"
    sed -i 's|url: http://127.0.0.1:8428|url: http://127.0.0.1:8080|' "$GRAFANA_DS"
    grep -q 'url: http://127.0.0.1:8080' "$GRAFANA_DS" || \
        die "Modification de $GRAFANA_DS a échoué (URL inchangée ?)"
    ok "$GRAFANA_DS mis à jour"

    log "Restart daly-bms-server..."
    systemctl restart daly-bms.service
    sleep 4
    health_check "daly-bms /-/healthy" "$DALYBMS_URL_HEALTHY" || \
        die "daly-bms ne redémarre pas — rollback manuel : $0 rollback"

    log "Restart grafana-server (provisioning reload requis)..."
    systemctl restart grafana-server.service
    sleep 8
    health_check "Grafana /api/health" "$GRAFANA_URL" || \
        warn "Grafana n'a pas encore répondu — peut prendre 15-30 s au boot"

    echo
    echo "$(c_green "✓ Bascule effectuée — Grafana interroge maintenant redb")"
    echo
    echo "  Prochaines étapes :"
    echo "    1. Ouvrir Grafana et vérifier qu'au moins 1 dashboard affiche les courbes attendues."
    echo "    2. Laisser tourner 12-24h, surveiller :"
    echo "         journalctl -u daly-bms -f | grep -iE 'error|warn|PromQl'"
    echo "         du -sh /mnt/nvme/daly-bms/metrics.redb"
    echo "    3. Si KO à tout moment : $0 rollback"
    echo
    echo "  Backup conservé sous $backup_dir"
}

cmd_rollback() {
    require_root "$@"
    require_files

    local backup_dir="${1:-$(latest_backup_dir || true)}"
    [[ -n "${backup_dir:-}" ]] || \
        die "Aucun backup trouvé (cherché sous $BACKUP_ROOT). Spécifier un chemin : $0 rollback /path/to/backup"
    [[ -d "$backup_dir" ]] || die "Backup directory introuvable : $backup_dir"

    log "Restauration depuis $backup_dir"
    [[ -f "$backup_dir/victoriametrics.yaml"  ]] || die "Backup incomplet : victoriametrics.yaml manquant"
    [[ -f "$backup_dir/daly-bms-config.toml"  ]] || die "Backup incomplet : daly-bms-config.toml manquant"

    cp -a "$backup_dir/victoriametrics.yaml"  "$GRAFANA_DS"
    cp -a "$backup_dir/daly-bms-config.toml"  "$DALYBMS_CONF"
    ok "Fichiers de config restaurés"

    log "Restart daly-bms-server..."
    systemctl restart daly-bms.service
    sleep 4
    health_check "daly-bms /-/healthy" "$DALYBMS_URL_HEALTHY" || \
        warn "daly-bms ne répond pas — vérifier journalctl -u daly-bms -n 50"

    log "Restart grafana-server..."
    systemctl restart grafana-server.service
    sleep 8
    health_check "Grafana /api/health" "$GRAFANA_URL" || \
        warn "Grafana n'a pas encore répondu"

    echo
    echo "$(c_green "✓ Rollback effectué — état pré-bascule restauré")"
    echo
    cmd_status
}

# ── dispatch ─────────────────────────────────────────────────────────────────

usage() {
    cat >&2 <<EOF
Usage : $0 <subcommand>

Sous-commandes :
  status     Affiche l'état courant (backend, URL Grafana, services)
  switch     Bascule Grafana + serveur vers redb (backup auto)
  rollback   Restaure l'état pré-bascule (depuis backup le plus récent)
EOF
    exit 1
}

cmd="${1:-}"
shift || true
case "$cmd" in
    status)   cmd_status   "$@" ;;
    switch)   cmd_switch   "$@" ;;
    rollback) cmd_rollback "$@" ;;
    -h|--help|"") usage ;;
    *) echo "Commande inconnue : $cmd" >&2; usage ;;
esac
