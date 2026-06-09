#!/usr/bin/env bash
# Déploiement COURANT Pi5 — le script du quotidien (≈ 90 % des changements).
#
# Principe : faire le strict nécessaire, RIEN de superflu, et ne redémarrer
# QUE ce qui a réellement changé (binaire et/ou config).
#
# Étapes :
#   1. Sync du dépôt (branche courante).
#   2. Build incrémental des 2 binaires aarch64 (cargo cache → rapide si inchangé).
#   3. Config : le Config.toml du repo est la SOURCE DE VÉRITÉ. Diff vs prod,
#      backup horodaté, puis application. (--no-config pour ne pas y toucher.)
#   4. Déploiement ciblé : compare chaque binaire à l'installé ; ne copie/redémarre
#      que s'il a changé. Un changement de config redémarre les 2 services.
#   5. Vérification santé (services actifs, /health, champ deye.state présent).
#
# Pour le déploiement COMPLET (Grafana, bridge mosquitto, validation métriques,
# auto-réparations de config) → scripts/deploy-pi5.sh.
#
# Usage (sur le Pi5, depuis ~/Daly-BMS-Rust) :
#   sudo bash scripts/deploy.sh                # tout (sync + build + config + deploy)
#   sudo bash scripts/deploy.sh --no-sync      # garder le code local (pas de make sync)
#   sudo bash scripts/deploy.sh --no-build     # config + redéploiement seuls
#   sudo bash scripts/deploy.sh --no-config    # ne pas toucher /etc/daly-bms/config.toml
#   sudo bash scripts/deploy.sh --dry-run      # montrer ce qui changerait, sans rien appliquer

set -euo pipefail

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; CYAN='\033[0;36m'; NC='\033[0m'
info()  { echo -e "${GREEN}[OK]${NC} $*"; }
step()  { echo -e "${CYAN}[>>]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!!]${NC} $*"; }
error() { echo -e "${RED}[XX]${NC} $*" >&2; exit 1; }

DO_SYNC=true; DO_BUILD=true; DO_CONFIG=true; DRY_RUN=false
for arg in "$@"; do
    case "$arg" in
        --no-sync)   DO_SYNC=false ;;
        --no-build)  DO_BUILD=false ;;
        --no-config) DO_CONFIG=false ;;
        --dry-run)   DRY_RUN=true ;;
        -h|--help)   sed -n '/^# Usage/,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *)           error "Argument inconnu : $arg (voir --help)" ;;
    esac
done
$DRY_RUN && warn "DRY-RUN : aucune écriture (config/binaires/restart) ne sera appliquée."

# Builds et git sous l'utilisateur appelant (cargo/rustup dans son ~/.cargo, dépôt
# lui appartient). L'install (cp, systemctl) reste en root. PWD + commande passés
# en paramètres positionnels pour éviter toute double expansion.
RUN_USER="${SUDO_USER:-$(id -un)}"
as_user() {
    if [ "$(id -un)" = "root" ] && [ "$RUN_USER" != "root" ]; then
        sudo -u "$RUN_USER" -H bash -lc 'cd "$1" && eval "$2"' _ "$PWD" "$*"
    else
        bash -lc 'cd "$1" && eval "$2"' _ "$PWD" "$*"
    fi
}

CONF=/etc/daly-bms/config.toml
ARM_DIR=target/aarch64-unknown-linux-gnu/release
# service systemd → nom du binaire
SERVICES=(daly-bms energy-manager)
bin_for() { case "$1" in daly-bms) echo daly-bms-server ;; energy-manager) echo energy-manager ;; esac; }

# ── 1. Sync ───────────────────────────────────────────────────────────────────
if $DO_SYNC; then
    step "Synchronisation du dépôt (branche courante)…"
    if $DRY_RUN; then as_user "git fetch origin \$(git rev-parse --abbrev-ref HEAD)" || true
    else as_user "make sync" || error "make sync a échoué"; fi
fi
BRANCH=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo '?')
COMMIT=$(git rev-parse --short HEAD 2>/dev/null || echo '?')
info "Code : branche '$BRANCH' @ $COMMIT"

# ── 2. Build incrémental ──────────────────────────────────────────────────────
if $DO_BUILD; then
    step "Compilation daly-bms-server (aarch64) — user: $RUN_USER…"
    as_user "make build-arm"        || error "make build-arm a échoué"
    step "Compilation energy-manager (aarch64) — user: $RUN_USER…"
    as_user "make build-energy-arm" || error "make build-energy-arm a échoué"
    info "Binaires compilés"
else
    warn "Build skippé (--no-build) — utilisation des binaires déjà présents dans $ARM_DIR"
fi

# ── 3. Config (repo = source de vérité) ───────────────────────────────────────
CONFIG_CHANGED=false
if $DO_CONFIG; then
    step "Vérification Config.toml (repo) vs prod ($CONF)…"
    if [[ ! -f "$CONF" ]]; then
        CONFIG_CHANGED=true
        if $DRY_RUN; then warn "[dry-run] $CONF absent → serait créé depuis le repo"
        else
            sudo mkdir -p "$(dirname "$CONF")"; sudo cp Config.toml "$CONF"
            info "Config initialisée (première install)"
        fi
    elif sudo cmp -s Config.toml "$CONF"; then
        info "Config identique à la prod — rien à faire"
    else
        warn "Différences config (prod ◀ repo) :"
        sudo diff "$CONF" Config.toml || true
        CONFIG_CHANGED=true
        if $DRY_RUN; then warn "[dry-run] la config ci-dessus serait appliquée (backup auto)"
        else
            BACKUP="$CONF.bak-$(date +%Y%m%d_%H%M%S)"
            sudo cp "$CONF" "$BACKUP"
            sudo cp Config.toml "$CONF"
            info "Config appliquée — backup : $BACKUP"
        fi
    fi
else
    warn "Config non touchée (--no-config)"
fi

# ── 4. Déploiement ciblé (ne redémarre que ce qui a changé) ───────────────────
step "Déploiement des binaires (diff vs installé)…"
RESTART=()
for svc in "${SERVICES[@]}"; do
    bin="$(bin_for "$svc")"
    built="$ARM_DIR/$bin"
    installed="/usr/local/bin/$bin"
    [[ -f "$built" ]] || { warn "$built absent (build skippé ?) — $svc ignoré"; continue; }
    if sudo cmp -s "$built" "$installed" 2>/dev/null; then
        info "$bin inchangé"
        $CONFIG_CHANGED && RESTART+=("$svc")   # config changée → restart quand même
    else
        if $DRY_RUN; then warn "[dry-run] $bin serait mis à jour → restart $svc"
        else sudo cp "$built" "$installed"; info "$bin mis à jour"; fi
        RESTART+=("$svc")
    fi
done

if [[ ${#RESTART[@]} -eq 0 ]]; then
    info "Aucun changement — aucun redémarrage nécessaire."
elif $DRY_RUN; then
    warn "[dry-run] redémarrerait : ${RESTART[*]}"
else
    # Dédoublonnage
    mapfile -t RESTART < <(printf '%s\n' "${RESTART[@]}" | sort -u)
    for svc in "${RESTART[@]}"; do
        step "Redémarrage $svc…"
        sudo systemctl restart "$svc"
    done
fi

$DRY_RUN && { echo ""; info "DRY-RUN terminé — rien n'a été appliqué."; exit 0; }

# ── 5. Vérification santé ─────────────────────────────────────────────────────
[[ ${#RESTART[@]} -gt 0 ]] && { step "Attente 4 s (démarrage)…"; sleep 4; }
echo ""
FAIL=0
for svc in "${SERVICES[@]}"; do
    if systemctl is-active --quiet "$svc"; then info "$svc : actif"
    else warn "$svc : INACTIF → journalctl -u $svc -n 50"; FAIL=1; fi
done

# Healthcheck backend + présence des champs DEYE (preuve que la nouvelle logique tourne)
if curl -sf http://localhost:8080/health -o /dev/null --max-time 3; then info "daly-bms /health OK"
else warn "daly-bms /health KO"; FAIL=1; fi

DEYE=$(curl -s --max-time 3 http://localhost:8081/api/rules-status 2>/dev/null | jq -c '.deye' 2>/dev/null || echo '')
if [[ -n "$DEYE" && "$DEYE" != "null" ]]; then
    info "energy-manager rules-status.deye : $DEYE"
    echo "$DEYE" | jq -e 'has("state") and has("mppt_full")' >/dev/null 2>&1 \
        && info "Champs DEYE à jour (state + mppt_full présents) ✓" \
        || warn "Champs DEYE anciens — binaire energy-manager pas à jour ?"
else
    warn "rules-status.deye indisponible (energy-manager up ?)"
fi

echo ""
if [[ $FAIL -eq 0 ]]; then
    info "Déploiement OK — branche '$BRANCH' @ $COMMIT"
else
    error "Déploiement terminé AVEC AVERTISSEMENTS — voir ci-dessus."
fi
