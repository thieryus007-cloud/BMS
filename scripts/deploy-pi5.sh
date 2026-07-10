#!/usr/bin/env bash
# Déploiement Pi5 — SCRIPT UNIQUE (daly-bms-server + energy-manager + infra).
#
# Voie unique de déploiement Pi5 : sync → build → config → unités systemd →
# mosquitto/Grafana → déploiement CIBLÉ (ne redémarre que ce qui a changé) →
# validation. Remplace l'ancien duo deploy.sh / deploy-pi5.sh (fusionnés ici
# pour éviter les divergences — cf. audit : deploy.sh oubliait les unités
# systemd, d'où des limites mémoire non appliquées).
#
# Usage (depuis ~/Daly-BMS-Rust sur le Pi5) :
#   bash scripts/deploy-pi5.sh                  # sync + build + déploiement ciblé + infra + validation
#   bash scripts/deploy-pi5.sh --dry-run        # montre le plan (code/config/binaires/unités), n'écrit RIEN
#   bash scripts/deploy-pi5.sh --no-sync        # garder le code local (pas de make sync)
#   bash scripts/deploy-pi5.sh --no-build       # utiliser les binaires déjà compilés dans target/
#   bash scripts/deploy-pi5.sh --apply-config   # appliquer Config.toml du repo → prod (diff + backup)
#   bash scripts/deploy-pi5.sh --no-validate    # sauter scripts/test-api.sh
# (options combinables ; --dry-run l'emporte sur toute écriture.)
#
# Config.toml : PRÉSERVÉE par défaut (la prod fait foi). Le script ne fait que
# bootstrap (si absente) + auto-réparations ciblées (port série, metrics_store).
# Pour pousser une vraie modif de config du repo → --apply-config (avec backup).
#
# Architecture post-migration redb (cf. docs/metriques-redb-architecture.md) :
# metrics-store (redb /mnt/nvme/daly-bms/metrics.redb) est la TSDB primaire ;
# VictoriaMetrics retiré (Phase 5) — le service est désactivé s'il traîne encore.

set -euo pipefail

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; CYAN='\033[0;36m'; NC='\033[0m'
info()  { echo -e "${GREEN}[OK]${NC} $*"; }
step()  { echo -e "${CYAN}[>>]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!!]${NC} $*"; }
error() { echo -e "${RED}[XX]${NC} $*" >&2; exit 1; }

DO_SYNC=true
DO_BUILD=true
DO_VALIDATE=true
APPLY_CONFIG=false
DRY_RUN=false
for arg in "$@"; do
    case "$arg" in
        --no-sync)      DO_SYNC=false ;;
        --no-build)     DO_BUILD=false ;;
        --no-validate)  DO_VALIDATE=false ;;
        --apply-config) APPLY_CONFIG=true ;;
        --dry-run)      DRY_RUN=true ;;
        -h|--help)
            sed -n '/^# Usage/,/^$/p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *) error "Argument inconnu : $arg (voir --help)" ;;
    esac
done
$DRY_RUN && warn "DRY-RUN : aucune écriture (config / binaires / unités / restart) ne sera appliquée."

# Compilation et `git` tournent sous l'utilisateur appelant (pas root) :
# `rustup`/`cargo` sont dans son `~/.cargo`, le dépôt lui appartient. Sous `sudo`,
# le PATH de root n'inclut pas `~/.cargo/bin` → « rustup: not found ». `as_user`
# ré-exécute dans un shell de login de cet utilisateur. L'install (cp, systemctl)
# reste en root. `$PWD` + commande passés en paramètres positionnels pour éviter
# toute double expansion / injection (revue Gemini).
RUN_USER="${SUDO_USER:-$(id -un)}"
as_user() {
    if [ "$(id -un)" = "root" ] && [ "$RUN_USER" != "root" ]; then
        sudo -u "$RUN_USER" -H bash -lc 'cd "$1" && eval "$2"' _ "$PWD" "$*"
    else
        bash -lc 'cd "$1" && eval "$2"' _ "$PWD" "$*"
    fi
}

ARM_DIR=target/aarch64-unknown-linux-gnu/release
CONF=/etc/daly-bms/config.toml
# Suivi des changements pour le déploiement ciblé (§6).
CONFIG_CHANGED=false          # config (appliquée ou auto-réparée) → restart des 2 services
declare -A UNIT_CHANGED       # par service → restart même si binaire identique
bin_for() { case "$1" in daly-bms) echo daly-bms-server ;; energy-manager) echo energy-manager ;; esac; }

# ── 1. Récupération du code ───────────────────────────────────────────────────
if $DO_SYNC; then
    step "Synchronisation du dépôt (branche courante)…"
    if $DRY_RUN; then
        as_user "git fetch origin \$(git rev-parse --abbrev-ref HEAD)" || true
        warn "[dry-run] commits entrants :"
        as_user "git --no-pager log --oneline HEAD..FETCH_HEAD" 2>/dev/null || true
    else
        as_user "make sync" || error "make sync a échoué"
        info "Code à jour"
    fi
else
    warn "Sync skippé (--no-sync) — code local conservé"
fi
BRANCH=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo '?')
COMMIT=$(git rev-parse --short HEAD 2>/dev/null || echo '?')
info "Code : branche '$BRANCH' @ $COMMIT"

# ── 2. Compilation croisée aarch64 (sous l'utilisateur appelant) ──────────────
# Build inconditionnel (le cache Cargo rend un build sans changement quasi
# instantané) ; c'est le diff binaire en §6 qui décide du redémarrage.
if $DRY_RUN; then
    warn "[dry-run] build sauté — comparaison sur les binaires déjà présents dans $ARM_DIR"
elif $DO_BUILD; then
    step "Compilation daly-bms-server (aarch64) — user: $RUN_USER…"
    as_user "make build-arm" || error "make build-arm a échoué"
    info "daly-bms-server compilé"
    step "Compilation energy-manager (aarch64) — user: $RUN_USER…"
    as_user "make build-energy-arm" || error "make build-energy-arm a échoué"
    info "energy-manager compilé"
else
    warn "Build skippé (--no-build) — binaires existants dans $ARM_DIR"
fi

# ── 3. Config.toml ────────────────────────────────────────────────────────────
# Défaut : PRÉSERVER (la prod a été calibrée par l'opérateur). --apply-config
# pour pousser le repo (diff + backup). Bootstrap si la cible est absente.
if [[ ! -f "$CONF" ]]; then
    step "Première installation : copie Config.toml → $CONF…"
    if $DRY_RUN; then warn "[dry-run] $CONF absent → serait créé depuis le repo"
    else
        sudo mkdir -p /etc/daly-bms
        sudo cp Config.toml "$CONF"
        CONFIG_CHANGED=true
        info "Config.toml initialisée"
    fi
elif $APPLY_CONFIG; then
    if sudo cmp -s Config.toml "$CONF"; then
        info "Config identique à la prod — rien à appliquer"
    else
        warn "Différences config (prod ◀ repo) :"
        sudo diff "$CONF" Config.toml || true
        if $DRY_RUN; then warn "[dry-run] la config ci-dessus serait appliquée (backup auto)"
        else
            BACKUP="$CONF.bak-$(date +%Y%m%d_%H%M%S)"
            sudo cp "$CONF" "$BACKUP"
            sudo cp Config.toml "$CONF"
            CONFIG_CHANGED=true
            info "Config appliquée — backup : $BACKUP"
        fi
    fi
else
    info "$CONF préservé (pas d'écrasement ; --apply-config pour pousser le repo)"
fi

# ── 3.bis Auto-réparations ciblées (ne réécrivent QUE la clé fautive) ─────────
# Ces correctifs évitent un service mort au boot ; ils s'appliquent même sans
# --apply-config car ils ne touchent pas aux réglages calibrés par l'opérateur.
if [[ -f "$CONF" ]]; then
    # Port série vide → auto-détection qui échoue si le BMS tarde. Port fixe ici.
    step "Vérification port série…"
    SERIAL_PORT=$(awk -F'"' '/^\[serial\]/{s=1;next} /^\[/{s=0} s&&/^[[:space:]]*port[[:space:]]*=/{print $2;exit}' "$CONF" || true)
    if [[ -z "$SERIAL_PORT" ]]; then
        if $DRY_RUN; then warn "[dry-run] [serial].port vide → serait forcé à /dev/ttyUSB0"
        else
            warn "[serial].port vide — réparation : /dev/ttyUSB0"
            sudo sed -i '/^\[serial\]/,/^\[/{s|^port = ""|port = "/dev/ttyUSB0"|}' "$CONF"
            CONFIG_CHANGED=true
            info "[serial].port forcé à /dev/ttyUSB0 ✓"
        fi
    else
        info "[serial].port = $SERIAL_PORT ✓"
    fi

    # metrics-store = seule TSDB de lecture ; désactivée par accident = dashboards vides.
    step "Vérification config critique (metrics_store)…"
    grep -q '^\[metrics_store\]' "$CONF" || error "Section [metrics_store] absente de $CONF — éditer manuellement"
    METRICS_ENABLED=$(awk '/^\[metrics_store\]/{s=1;next} /^\[/{s=0} s&&/^[[:space:]]*enabled[[:space:]]*=/{print $3;exit}' "$CONF" || true)
    if [[ "$METRICS_ENABLED" != "true" ]]; then
        if $DRY_RUN; then warn "[dry-run] [metrics_store].enabled='$METRICS_ENABLED' → serait forcé à true"
        else
            BACKUP="$CONF.before-autorepair-$(date +%Y%m%d_%H%M%S)"
            warn "[metrics_store].enabled = '$METRICS_ENABLED' — réparation (backup → $BACKUP)"
            sudo cp "$CONF" "$BACKUP"
            sudo sed -i '/^\[metrics_store\]/,/^\[/ {s/^enabled = false$/enabled = true/}' "$CONF"
            CONFIG_CHANGED=true
            info "[metrics_store].enabled forcé à true ✓"
        fi
    else
        info "[metrics_store].enabled = true ✓"
    fi

    # db_path : parent doit exister et appartenir à l'utilisateur du service.
    DB_PATH=$(awk -F'"' '/^[[:space:]]*db_path[[:space:]]*=/{print $2;exit}' "$CONF" || true)
    DB_PATH="${DB_PATH:-/mnt/nvme/daly-bms/metrics.redb}"
    DB_DIR=$(dirname "$DB_PATH")
    DALY_USER=$(systemctl show daly-bms --property=User --value 2>/dev/null || echo dalybms)
    DALY_USER="${DALY_USER:-dalybms}"
    if $DRY_RUN; then info "[dry-run] db_path = $DB_PATH (parent owner attendu : $DALY_USER)"
    else
        sudo mkdir -p "$DB_DIR"
        sudo chown "$DALY_USER:$DALY_USER" "$DB_DIR"
        info "db_path = $DB_PATH (parent owner=$DALY_USER) ✓"
    fi

    # Config DEYE : alerte non destructive si les nouvelles clés manquent.
    step "Vérification config DEYE…"
    DEYE_STALE=false
    grep -q '^[[:space:]]*shelly_deye_channels' "$CONF" || DEYE_STALE=true
    grep -q '^[[:space:]]*mppt_cut_enabled'     "$CONF" || DEYE_STALE=true
    if $DEYE_STALE; then
        warn "Config DEYE incomplète (shelly_deye_channels / mppt_cut_enabled absents) →"
        warn "  corrections DEYE inactives. Appliquer : bash scripts/deploy-pi5.sh --apply-config"
    else
        info "Config DEYE à jour ✓"
    fi
fi

# ── 4. NVMe monté + retrait VictoriaMetrics (Phase 5) ─────────────────────────
mountpoint -q /mnt/nvme || warn "/mnt/nvme non monté — vérifier /etc/fstab"
if systemctl is-active --quiet victoriametrics 2>/dev/null; then
    if $DRY_RUN; then warn "[dry-run] victoriametrics actif → serait stoppé + disabled (Phase 5)"
    else
        warn "victoriametrics.service encore actif — désactivation (Phase 5 retrait)"
        sudo systemctl stop victoriametrics
        sudo systemctl disable victoriametrics
        info "victoriametrics.service stoppé + disabled"
        if [[ -d /mnt/nvme/victoria-metrics ]]; then
            sudo mv /mnt/nvme/victoria-metrics /mnt/nvme/victoria-metrics.archive-$(date +%Y%m%d) 2>/dev/null || true
            info "Data dir VM archivée (rollback possible 30 j)"
        fi
    fi
else
    info "VictoriaMetrics non installé / déjà retiré"
fi

# ── 5. Unités systemd (redéployer si modifiées dans le repo) ──────────────────
# Un changement d'unité impose un restart du service concerné MÊME si le binaire
# est identique (cas typique : ajout de MemoryHigh/Max, RequiresMountsFor…).
for svc in daly-bms energy-manager; do
    src="contrib/${svc}.service"
    dst="/etc/systemd/system/${svc}.service"
    [[ -f "$src" ]] || continue
    if sudo diff -q "$src" "$dst" >/dev/null 2>&1; then
        info "Unité $svc à jour"
    else
        UNIT_CHANGED[$svc]=1
        if $DRY_RUN; then
            warn "[dry-run] unité $svc modifiée → serait copiée + daemon-reload + restart $svc"
            sudo diff "$dst" "$src" 2>/dev/null | sed 's/^/    /' || true
        else
            step "Mise à jour $dst…"
            sudo cp "$src" "$dst"
            sudo systemctl daemon-reload
            info "Unité systemd $svc mise à jour (restart en §6)"
        fi
    fi
done

# ── 5.quater Sauvegarde redb : script + service oneshot + timer quotidien ─────
# Copie périodique crash-consistent de metrics.redb (#3, incident 2026-07-09).
# Protège contre une corruption AVÉRÉE (quarantaine auto → base recréée vide) :
# ni TimeoutStartSec=infinity ni l'ouverture redb en tâche de fond ne couvrent
# ce cas. Le timer tourne quotidiennement (cf. contrib/daly-bms-backup.timer).
BK_SRC=scripts/backup-redb.sh
BK_DST=/usr/local/bin/backup-redb.sh
if [[ -f "$BK_SRC" ]]; then
    if $DRY_RUN; then
        sudo cmp -s "$BK_SRC" "$BK_DST" 2>/dev/null || warn "[dry-run] $BK_DST absent/différent → serait (ré)installé (0755)"
    else
        sudo install -m 0755 "$BK_SRC" "$BK_DST"
        info "Script de sauvegarde installé → $BK_DST"
    fi
fi
BK_UNIT_CHANGED=0
for unit in daly-bms-backup.service daly-bms-backup.timer; do
    usrc="contrib/${unit}"
    udst="/etc/systemd/system/${unit}"
    [[ -f "$usrc" ]] || continue
    if sudo diff -q "$usrc" "$udst" >/dev/null 2>&1; then
        info "Unité $unit à jour"
    else
        BK_UNIT_CHANGED=1
        if $DRY_RUN; then
            warn "[dry-run] unité $unit modifiée → serait copiée"
        else
            sudo cp "$usrc" "$udst"
            info "Unité systemd $unit mise à jour"
        fi
    fi
done
if ! $DRY_RUN; then
    [[ $BK_UNIT_CHANGED -eq 1 ]] && sudo systemctl daemon-reload
    if [[ -f /etc/systemd/system/daly-bms-backup.timer ]]; then
        sudo systemctl enable --now daly-bms-backup.timer >/dev/null 2>&1 || true
        info "Timer daly-bms-backup activé (sauvegarde redb quotidienne)"
    fi
fi

# ── 5.bis Mosquitto bridge (si modifié) — backup + verify-no-loop + restart ────
MOSQ_SRC=contrib/mosquitto/mosquitto.conf
MOSQ_DST=/etc/mosquitto/mosquitto.conf
if [[ -f "$MOSQ_SRC" ]] && ! sudo diff -q "$MOSQ_SRC" "$MOSQ_DST" >/dev/null 2>&1; then
    if $DRY_RUN; then warn "[dry-run] mosquitto.conf modifié → serait déployé (après verify-no-loop) + restart broker"
    else
        step "Mise à jour mosquitto.conf (bridge topics)…"
        MOSQ_BAK="${MOSQ_DST}.bak-$(date +%Y%m%d_%H%M%S)"
        sudo cp "$MOSQ_DST" "$MOSQ_BAK" 2>/dev/null || true
        sudo cp "$MOSQ_SRC" "$MOSQ_DST"
        if [[ -x /usr/local/bin/verify-no-loop.sh ]]; then
            if ! sudo /usr/local/bin/verify-no-loop.sh; then
                warn "verify-no-loop a échoué — restauration depuis $MOSQ_BAK"
                [[ -f "$MOSQ_BAK" ]] && sudo cp "$MOSQ_BAK" "$MOSQ_DST"
                error "Bridge mosquitto invalide (risque de boucle) — config restaurée, déploiement interrompu"
            fi
            info "Bridge validé (aucune boucle)"
        else
            warn "verify-no-loop.sh absent — bridge NON validé (vérifier les règles 'out')"
        fi
        sudo systemctl restart mosquitto-broker
        info "mosquitto.conf déployé + broker redémarré"
    fi
else
    info "mosquitto.conf déjà à jour (ou absent du repo)"
fi

# ── 5.ter Journald : plafond de taille (si modifié) ───────────────────────────
# Borne le journal systemd (cf. contrib/journald/daly-bms.conf). Purement
# défensif : journald purge ses plus vieux enregistrements au-delà du plafond,
# aucun service applicatif n'est touché → on peut redémarrer journald sans
# risque pour daly-bms/energy-manager/mosquitto.
JRNL_SRC=contrib/journald/daly-bms.conf
JRNL_DST=/etc/systemd/journal.conf.d/daly-bms.conf
if [[ -f "$JRNL_SRC" ]] && ! sudo diff -q "$JRNL_SRC" "$JRNL_DST" >/dev/null 2>&1; then
    if $DRY_RUN; then
        warn "[dry-run] $JRNL_DST absent/différent → serait déployé + restart systemd-journald"
        sudo diff "$JRNL_DST" "$JRNL_SRC" 2>/dev/null | sed 's/^/    /' || true
    else
        step "Mise à jour du plafond journald ($JRNL_DST)…"
        sudo mkdir -p /etc/systemd/journal.conf.d
        sudo install -m 644 -o root -g root "$JRNL_SRC" "$JRNL_DST"
        sudo systemctl restart systemd-journald
        info "Plafond journald appliqué (SystemMaxUse=200M) + journald redémarré"
        # Purge immédiate à la nouvelle borne (sinon le plafond n'est appliqué
        # qu'à la prochaine rotation). Sans effet si déjà sous la borne.
        sudo journalctl --vacuum-size=200M >/dev/null 2>&1 || true
    fi
else
    info "Plafond journald déjà à jour (ou absent du repo)"
fi

# ── 6. Déploiement CIBLÉ des binaires ─────────────────────────────────────────
# Redémarre un service ssi : binaire différent, OU unité changée (§5), OU config
# changée (§3/§3.bis). Évite les interruptions de service inutiles.
step "Déploiement des binaires (diff vs installé)…"
RESTARTED=()
for svc in daly-bms energy-manager; do
    bin="$(bin_for "$svc")"
    built="$ARM_DIR/$bin"
    installed="/usr/local/bin/$bin"
    [[ -f "$built" ]] || { warn "$built absent (build skippé ?) — $svc ignoré"; continue; }

    reason=""
    sudo cmp -s "$built" "$installed" 2>/dev/null || reason="binaire"
    [[ "${UNIT_CHANGED[$svc]:-}" == "1" ]] && reason="${reason:+$reason+}unité"
    $CONFIG_CHANGED && reason="${reason:+$reason+}config"

    if [[ -z "$reason" ]]; then
        info "$svc inchangé — aucun redéploiement"
        continue
    fi
    if $DRY_RUN; then
        warn "[dry-run] $svc serait redéployé ($reason)"
        continue
    fi
    step "Déploiement $svc ($reason)…"
    sudo systemctl stop "$svc"
    sudo cp "$built" "$installed"
    sudo systemctl start "$svc"
    RESTARTED+=("$svc")
    # Healthcheck : on POLL jusqu'à 300 s. READY (Type=notify) n'est envoyé
    # qu'après l'ouverture de metrics-store (redb) ; sur une grosse base
    # (plusieurs Go) ça peut prendre 1-2 min. Un `sleep 4` fixe faisait
    # échouer le déploiement à tort sur un démarrage lent mais légitime.
    svc_up=0
    for _ in $(seq 1 60); do
        if systemctl is-active --quiet "$svc"; then svc_up=1; break; fi
        if systemctl is-failed --quiet "$svc"; then break; fi
        sleep 5
    done
    if [[ "$svc_up" -eq 1 ]]; then
        info "$svc actif"
    else
        error "$svc n'a pas démarré (≤ 300 s) — journalctl -u $svc -n 50"
    fi
    if [[ "$svc" == "daly-bms" ]]; then
        journalctl -u daly-bms --since "30 seconds ago" --no-pager 2>/dev/null \
            | grep -q 'metrics-store ouvert' && info "metrics-store ouvert ✓" \
            || warn "metrics-store : init non détectée (vérifier les logs)"
    fi
done

# ── 7. Grafana (datasource + dashboards) ──────────────────────────────────────
if $DRY_RUN; then
    warn "[dry-run] Grafana : datasource + import dashboards non exécutés"
elif systemctl list-unit-files grafana-server.service &>/dev/null; then
    step "Déploiement Grafana (datasource + dashboards)…"
    sudo rm -f /etc/grafana/provisioning/datasources/victoriametrics.yaml
    sudo rm -f /etc/grafana/provisioning/datasources/redb.yaml
    sudo install -d -m 755 -o root -g grafana /etc/grafana/provisioning/datasources
    sudo install -m 644 -o root -g grafana \
        contrib/grafana/provisioning/datasources/daly-metrics.yaml \
        /etc/grafana/provisioning/datasources/
    sudo rm -f /etc/grafana/provisioning/dashboards/daly-bms.yaml
    sudo rm -f /etc/grafana/provisioning/dashboards/dashboards.yaml
    sudo rm -rf /etc/grafana/dashboards
    info "Datasource provisionnée, providers dashboards nettoyés"
    sudo systemctl restart grafana-server
    sleep 4
    if systemctl is-active --quiet grafana-server; then
        info "grafana-server actif — import des dashboards via API…"
        sudo bash scripts/fix-grafana.sh 2>&1 | tail -25
    else
        warn "grafana-server ne démarre pas — journalctl -u grafana-server -n 30"
    fi
else
    info "Grafana non installé — bash scripts/setup-grafana.sh --nvme"
fi

# ── 8. Validation API ──────────────────────────────────────────────────────────
if $DRY_RUN; then
    warn "[dry-run] validation API (test-api.sh) non exécutée"
elif $DO_VALIDATE; then
    echo ""
    step "Validation des endpoints API (test-api.sh)…"
    if [[ -x scripts/test-api.sh ]]; then
        if bash scripts/test-api.sh; then info "Validation API : tous les tests passent ✓"
        else warn "Validation API : 1+ test(s) en échec — voir ci-dessus"; fi
    else
        warn "scripts/test-api.sh manquant/non exécutable — skip"
    fi
fi

# ── 9. Résumé ─────────────────────────────────────────────────────────────────
echo ""
if $DRY_RUN; then
    echo -e "${YELLOW}═══════════════════════════════════════${NC}"
    echo -e "${YELLOW}  DRY-RUN terminé — rien n'a été appliqué${NC}"
    echo -e "${YELLOW}═══════════════════════════════════════${NC}"
    exit 0
fi
echo -e "${GREEN}═══════════════════════════════════════${NC}"
echo -e "${GREEN}  Déploiement terminé ✓${NC}"
echo -e "${GREEN}═══════════════════════════════════════${NC}"
echo ""
if [[ ${#RESTARTED[@]} -eq 0 ]]; then
    info "Aucun service redémarré (rien n'avait changé)."
else
    info "Services redémarrés : ${RESTARTED[*]}"
fi
info "Code déployé : branche '$BRANCH' @ $COMMIT"

# Comptage des séries + présence des métriques clés (8 s pour les 1ers snapshots).
echo ""
step "Patientons 8 s avant comptage des séries…"
sleep 8
N_SERIES=$(curl -s http://localhost:8080/api/v1/redb/series 2>/dev/null | jq '.data | length' 2>/dev/null || echo "?")
info "Nb séries en base : $N_SERIES"

NEW_METRICS=(
    bms_capacity_ah bms_temp_min bms_charge_mos bms_cell_voltage bms_power
    et112_apparent_power_va et112_frequency_hz et112_power_factor
    venus_mppt_power_w venus_mppt_yield_today_kwh venus_mppt_dc_current_a
    venus_mppt_max_power_today_w
    venus_shunt_soc_percent venus_shunt_energy_in_kwh venus_shunt_energy_out_kwh
    venus_inverter_ac_output_power_w venus_inverter_ac_output_voltage_v
    venus_inverter_ac_output_current_a venus_inverter_ac_freq_hz
    venus_heatpump_power_w venus_heatpump_temp_c venus_heatpump_energy_kwh
    shelly_current_a shelly_output shelly_energy_wh
    pi5_cpu_percent pi5_memory_percent pi5_cpu_temp_c pi5_uptime_secs
    em_cpu_percent em_memory_percent
    dc_pv_power_w pvinv_power_w solar_yield_kwh solar_total_wh
    ats_voltage_v ats_freq_hz total_solar_power
    source_last_update_age_seconds
)
SERIES_JSON=$(curl -s http://localhost:8080/api/v1/redb/series 2>/dev/null || echo '{}')
PRESENT=0; MISSING=()
for m in "${NEW_METRICS[@]}"; do
    if echo "$SERIES_JSON" | jq -e --arg m "$m" '.data[] | select(.__name__ == $m)' >/dev/null 2>&1; then
        PRESENT=$((PRESENT+1))
    else
        MISSING+=("$m")
    fi
done
info "Métriques présentes : $PRESENT / ${#NEW_METRICS[@]}"
if (( ${#MISSING[@]} > 0 )); then
    warn "Manquantes : ${MISSING[*]}"
    warn "→ vérifier les payloads MQTT amont :"
    warn "    timeout 5 mosquitto_sub -h 127.0.0.1 -t 'santuario/#' -v"
fi

echo ""
step "État des services :"
for svc in daly-bms energy-manager grafana-server mosquitto-broker; do
    if systemctl list-unit-files "${svc}.service" &>/dev/null; then
        STATUS=$(systemctl is-active "$svc" 2>/dev/null || echo "absent")
        [[ "$STATUS" == "active" ]] && info "$svc : actif" || warn "$svc : $STATUS"
    fi
done
if systemctl is-active --quiet grafana-server 2>/dev/null; then
    curl -sf http://localhost:3000/api/health -o /dev/null --max-time 3 \
        && info "Grafana healthcheck OK" \
        || warn "Grafana healthcheck échoué — journalctl -u grafana-server -n 30"
fi
