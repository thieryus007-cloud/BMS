#!/usr/bin/env bash
# =============================================================================
# backup-sdcard-pi5.sh — image disque complète de la carte SD du Pi5 vers le NVMe
# =============================================================================
#
# POURQUOI : la racine (/, carte microSD /dev/mmcblk0) porte l'OS, les paquets,
# /etc, les checkouts de code (~/Daly-BMS-Rust, ~/ot-br-posix) et tout ce qui a
# été identifié comme « local uniquement, pas dans Git » (cf. CLAUDE.md § État
# Git). Une carte microSD est le composant le plus fragile d'un Raspberry Pi
# (usure d'écriture, corruption) — ce script en garde une image bootable
# complète sur le NVMe (disque physique séparé), pour restaurer en flashant
# une carte de remplacement plutôt qu'en reconstruisant tout à la main.
#
# COHÉRENCE : image « à chaud » (services en marche), PAS un snapshot atomique
# par défaut. Les données critiques à écriture fréquente (metrics.redb,
# Grafana) vivent sur le NVMe, PAS sur cette carte — la racine ne reçoit quasi
# aucune écriture pendant la copie (journald, apt). Risque résiduel : légère
# incohérence si un fichier système change pendant la copie (~10-20 min) ; un
# `fsck` au premier boot de la carte restaurée suffit à corriger, comme après
# une coupure secteur. Pour une garantie plus forte (gèle TOUTES les écritures
# sur / pendant toute la copie — n'affecte pas metrics.redb/Grafana, sur NVMe) :
# --freeze. Non utilisé par défaut ni par le timer (trop intrusif pour un run
# automatique non surveillé).
#
# USAGE :
#   sudo backup-sdcard-pi5.sh
#   sudo KEEP=3 backup-sdcard-pi5.sh
#   sudo backup-sdcard-pi5.sh --freeze
#
# Variables d'environnement (surchargables) :
#   SRC_DEVICE  périphérique bloc source     (déf. /dev/mmcblk0)
#   BACKUP_DIR  dossier des images           (déf. /mnt/nvme/daly-bms/backups/sdcard)
#   KEEP        nombre d'images à garder     (déf. 2 — images volumineuses, ~Go)
# -----------------------------------------------------------------------------
set -euo pipefail

log() { echo "[backup-sdcard] $*"; }
die() { echo "[backup-sdcard] ERREUR : $*" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] || die "lancer avec sudo (accès bloc nécessaire sur \$SRC_DEVICE)"

SRC_DEVICE="${SRC_DEVICE:-/dev/mmcblk0}"
BACKUP_DIR="${BACKUP_DIR:-/mnt/nvme/daly-bms/backups/sdcard}"
KEEP="${KEEP:-2}"
FREEZE=false
for arg in "$@"; do
    case "$arg" in
        --freeze) FREEZE=true ;;
        *) die "option inconnue : $arg" ;;
    esac
done

command -v zstd >/dev/null 2>&1 || die "zstd introuvable (apt install zstd)"
[ -b "$SRC_DEVICE" ] || die "périphérique bloc introuvable : $SRC_DEVICE"
mkdir -p "$BACKUP_DIR"

# ── Verrou anti-concurrence (best-effort, même pattern que backup-redb.sh) ───
if exec 9<"$BACKUP_DIR" 2>/dev/null && command -v flock >/dev/null 2>&1; then
    if ! flock -n 9; then
        log "une sauvegarde est déjà en cours (verrou tenu) — abandon"
        exit 0
    fi
fi

# ── Nettoyage des .partial orphelins ─────────────────────────────────────────
for orphan in "$BACKUP_DIR"/*.img.zst.partial; do
    [ -e "$orphan" ] || continue
    log "purge d'un .partial orphelin (run précédent interrompu) : $orphan"
    rm -f "$orphan"
done

# ── Garde-fou espace disque : taille brute du périphérique + 10 % de marge ───
# (majorant volontairement pessimiste : la compression zstd réduit en pratique
# la taille réelle, mais on garde de la marge pour un run non compressible)
src_bytes=$(blockdev --getsize64 "$SRC_DEVICE")
avail_bytes=$(df -PB1 "$BACKUP_DIR" | awk 'NR==2 {print $4}')
need_bytes=$(( src_bytes + src_bytes / 10 ))
if [ "$avail_bytes" -lt "$need_bytes" ]; then
    die "espace insuffisant dans $BACKUP_DIR (libre=$(( avail_bytes/1073741824 )) Go, requis≈$(( need_bytes/1073741824 )) Go). Baisser KEEP ou libérer de l'espace."
fi

# ── Image compressée (zstd multi-thread) ──────────────────────────────────────
ts="$(date +%Y%m%d_%H%M%S)"
dest="$BACKUP_DIR/pi5-sdcard-${ts}.img.zst"
tmp="${dest}.partial"

if [ "$FREEZE" = true ]; then
    log "gel des écritures sur / pendant la copie (--freeze)..."
    fsfreeze -f /
    trap 'fsfreeze -u / 2>/dev/null || true' EXIT
fi

sync
log "image de $SRC_DEVICE ($(( src_bytes/1073741824 )) Go) → $dest"
t0=$(date +%s)
dd if="$SRC_DEVICE" bs=4M conv=noerror,sync status=progress 2>&1 | zstd -T0 -q > "$tmp"
sync "$tmp"
mv "$tmp" "$dest"

if [ "$FREEZE" = true ]; then
    fsfreeze -u /
    trap - EXIT
fi

log "image terminée en $(( ($(date +%s) - t0) / 60 )) min ($(du -h "$dest" | cut -f1))"

# ── Rotation : ne garder que les KEEP plus récentes ───────────────────────────
mapfile -t backups < <(ls -1t "$BACKUP_DIR"/pi5-sdcard-*.img.zst 2>/dev/null || true)
if [ "${#backups[@]}" -gt "$KEEP" ]; then
    for old in "${backups[@]:$KEEP}"; do
        log "purge ancienne image : $old"
        rm -f "$old"
    done
fi

log "OK — $(ls -1 "$BACKUP_DIR"/pi5-sdcard-*.img.zst 2>/dev/null | wc -l) image(s) conservée(s) dans $BACKUP_DIR"

# =============================================================================
# RESTAURATION (depuis une AUTRE machine avec lecteur de carte SD/USB — ne PAS
# écrire sur la carte qui fait actuellement démarrer le Pi5) :
#
#   1. Récupérer l'image depuis le Pi5 :
#        scp pi5compute@192.168.1.141:/mnt/nvme/daly-bms/backups/sdcard/pi5-sdcard-<TS>.img.zst .
#
#   2. Vérifier le bon périphérique AVANT de flasher (dd sur le mauvais disque
#      est irréversible) :
#        lsblk
#
#   3. Décompresser et flasher sur une carte SD de remplacement (>= 29 Go) :
#        zstd -d pi5-sdcard-<TS>.img.zst -o pi5-sdcard-<TS>.img
#        sudo dd if=pi5-sdcard-<TS>.img of=/dev/sdX bs=4M status=progress conv=fsync
#
#   4. Insérer la carte dans le Pi5, démarrer normalement.
#
#   5. Un `fsck` automatique peut se déclencher au premier boot (normal si
#      l'image a été prise sans --freeze) — laisser faire, ne pas interrompre.
#
#   6. Reconstruire ensuite ce qui est volontairement absent de l'image récente
#      si l'image restaurée est ancienne (cf. CLAUDE.md § État Git pour la
#      liste à jour) : `sudo bash scripts/setup-otbr-pi5.sh`, venv du pont FP2, etc.
# =============================================================================
