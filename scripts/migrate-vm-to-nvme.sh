#!/usr/bin/env bash
# Migration VictoriaMetrics → NVMe (/mnt/nvme/victoria-metrics)
# Exécuter UNE SEULE FOIS sur le Pi5 en tant que root ou avec sudo
# Usage : sudo bash scripts/migrate-vm-to-nvme.sh

set -euo pipefail

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; NC='\033[0m'
info()  { echo -e "${GREEN}[OK]${NC} $*"; }
step()  { echo -e "${YELLOW}[>>]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!!]${NC} $*"; }
error() { echo -e "${RED}[!!]${NC} $*" >&2; exit 1; }

OLD_DIR="/var/lib/victoria-metrics"
NEW_DIR="/mnt/nvme/victoria-metrics"
SERVICE="victoriametrics"
SERVICE_FILE="/etc/systemd/system/victoriametrics.service"

# ── Vérifications préalables ──────────────────────────────────────────────────
[[ $EUID -eq 0 ]] || error "Ce script doit être exécuté en root (sudo)"

step "Vérification du montage NVMe…"
mountpoint -q /mnt/nvme || error "/mnt/nvme n'est pas monté — vérifier /etc/fstab"
NVME_FREE=$(df -BG /mnt/nvme | awk 'NR==2{print $4}' | tr -d 'G')
[[ "${NVME_FREE}" -ge 10 ]] || warn "Espace NVMe < 10 Go disponibles (${NVME_FREE} Go)"
info "NVMe monté, ${NVME_FREE} Go libres"

# ── 1. Arrêt du service ───────────────────────────────────────────────────────
step "Arrêt de VictoriaMetrics…"
systemctl stop "${SERVICE}" 2>/dev/null || warn "Service déjà arrêté"
sleep 2
info "Service arrêté"

# ── 2. Création du répertoire cible ──────────────────────────────────────────
step "Création de ${NEW_DIR}…"
mkdir -p "${NEW_DIR}"
VM_USER=$(stat -c '%U' "${OLD_DIR}" 2>/dev/null || echo "victoriametrics")
chown "${VM_USER}:${VM_USER}" "${NEW_DIR}"
info "Répertoire créé (owner=${VM_USER})"

# ── 3. Migration des données ──────────────────────────────────────────────────
if [[ -d "${OLD_DIR}" ]] && [[ -n "$(ls -A "${OLD_DIR}" 2>/dev/null)" ]]; then
    OLD_SIZE=$(du -sh "${OLD_DIR}" 2>/dev/null | cut -f1)
    step "Copie des données ${OLD_DIR} → ${NEW_DIR} (taille: ${OLD_SIZE})…"
    rsync -aH --info=progress2 "${OLD_DIR}/" "${NEW_DIR}/"
    info "Données copiées"

    step "Vérification de l'intégrité (comptage fichiers)…"
    OLD_COUNT=$(find "${OLD_DIR}" -type f | wc -l)
    NEW_COUNT=$(find "${NEW_DIR}" -type f | wc -l)
    [[ "${OLD_COUNT}" -eq "${NEW_COUNT}" ]] || \
        error "Comptage différent : ${OLD_COUNT} (source) vs ${NEW_COUNT} (cible) — NE PAS supprimer l'ancienne"
    info "Intégrité OK : ${NEW_COUNT} fichiers copiés"
else
    warn "Aucune donnée dans ${OLD_DIR} — démarrage à vide sur NVMe"
fi

# ── 4. Mise à jour du service systemd ────────────────────────────────────────
step "Déploiement nouveau fichier service…"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_SERVICE="${SCRIPT_DIR}/../contrib/victoriametrics.service"

if [[ -f "${REPO_SERVICE}" ]]; then
    cp "${REPO_SERVICE}" "${SERVICE_FILE}"
    info "Service déployé depuis contrib/victoriametrics.service"
else
    # Mise à jour directe si fichier contrib absent
    warn "contrib/victoriametrics.service introuvable — mise à jour directe du service"
    sed -i \
        -e "s|-storageDataPath=.*|-storageDataPath=${NEW_DIR} \\\\|" \
        -e "s|-retentionPeriod=.*|-retentionPeriod=2y \\\\|" \
        "${SERVICE_FILE}"
fi

systemctl daemon-reload
info "Systemd rechargé"

# ── 5. Démarrage et vérification ─────────────────────────────────────────────
step "Démarrage de VictoriaMetrics sur NVMe…"
systemctl start "${SERVICE}"
sleep 5

if systemctl is-active --quiet "${SERVICE}"; then
    info "VictoriaMetrics actif sur ${NEW_DIR}"
else
    error "VictoriaMetrics n'a pas démarré — journalctl -u victoriametrics -n 30"
fi

# Test de l'API
if curl -sf 'http://localhost:8428/-/healthy' >/dev/null 2>&1; then
    info "API VictoriaMetrics accessible (:8428)"
else
    warn "API non accessible — attendre quelques secondes et vérifier"
fi

# ── 6. Sauvegarde de l'ancienne données (optionnel) ──────────────────────────
echo ""
echo -e "${YELLOW}═══════════════════════════════════════════════════════${NC}"
echo -e "${YELLOW}  Ancien répertoire conservé : ${OLD_DIR}${NC}"
echo -e "${YELLOW}  Supprimer manuellement si tout est OK :${NC}"
echo -e "${YELLOW}  sudo rm -rf ${OLD_DIR}${NC}"
echo -e "${YELLOW}═══════════════════════════════════════════════════════${NC}"
echo ""

step "Vérification finale…"
echo "Service    : $(systemctl is-active ${SERVICE})"
echo "Données    : ${NEW_DIR}"
echo "Retention  : 2 ans"
USED=$(du -sh "${NEW_DIR}" 2>/dev/null | cut -f1 || echo "N/A")
echo "Taille     : ${USED}"
echo ""
info "Migration terminée avec succès"
