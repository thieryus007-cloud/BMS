#!/usr/bin/env bash
# =============================================================================
# backup-redb.sh — sauvegarde périodique de la base metrics-store (redb)
# =============================================================================
#
# POURQUOI (#3, incident 2026-07-09) : `TimeoutStartSec=infinity` +
# l'ouverture redb en tâche de fond garantissent que le service REMONTE après
# une coupure, mais ne protègent PAS contre une corruption AVÉRÉE (bitflip,
# panne NVMe) : dans ce cas la quarantaine automatique
# (`open_metrics_store_with_quarantine`) met la base de côté et en recrée une
# VIDE → l'historique est perdu. Ce script en garde des copies pour pouvoir
# restaurer.
#
# COHÉRENCE : redb est transactionnel (COW + fsync par commit). Une copie « à
# chaud » (service en marche) capture donc un instantané CRASH-CONSISTENT :
# au pire, restaurer déclenchera une brève récupération redb au démarrage
# (tolérée depuis `TimeoutStartSec=infinity`). Pas besoin d'arrêter le service.
#
# USAGE :
#   backup-redb.sh                 # valeurs par défaut
#   DB_PATH=... BACKUP_DIR=... KEEP=5 backup-redb.sh
#
# Variables d'environnement (surchargables) :
#   DB_PATH     chemin de la base redb      (déf. /mnt/nvme/daly-bms/metrics.redb)
#   BACKUP_DIR  dossier des sauvegardes     (déf. /mnt/nvme/daly-bms/backups)
#   KEEP        nombre de copies à garder   (déf. 3)
#
# ⚠ Par défaut la sauvegarde est sur le MÊME NVMe que la base : cela protège
# de la corruption logique / quarantaine / suppression accidentelle, PAS d'une
# panne matérielle du disque. Pour une vraie protection, viser un autre support
# (BACKUP_DIR sur un second disque, ou rsync vers une autre machine — voir la
# doc en fin de fichier).
# -----------------------------------------------------------------------------
set -euo pipefail

DB_PATH="${DB_PATH:-/mnt/nvme/daly-bms/metrics.redb}"
BACKUP_DIR="${BACKUP_DIR:-/mnt/nvme/daly-bms/backups}"
KEEP="${KEEP:-3}"

log() { echo "[backup-redb] $*"; }
die() { echo "[backup-redb] ERREUR : $*" >&2; exit 1; }

[ -f "$DB_PATH" ] || die "base introuvable : $DB_PATH"

base_name="$(basename "$DB_PATH")"
mkdir -p "$BACKUP_DIR"

# ── Garde-fou espace disque : refuser si l'espace libre < 1.2× la taille base ─
db_bytes=$(stat -c '%s' "$DB_PATH")
avail_bytes=$(df -PB1 "$BACKUP_DIR" | awk 'NR==2 {print $4}')
need_bytes=$(( db_bytes + db_bytes / 5 ))   # taille + 20 % de marge
if [ "$avail_bytes" -lt "$need_bytes" ]; then
    die "espace insuffisant dans $BACKUP_DIR (libre=$(( avail_bytes/1048576 )) Mo, requis≈$(( need_bytes/1048576 )) Mo). Baisser KEEP ou changer BACKUP_DIR."
fi

# ── Copie horodatée (crash-consistent) ───────────────────────────────────────
ts="$(date +%Y%m%d_%H%M%S)"
dest="$BACKUP_DIR/${base_name}.${ts}"
tmp="${dest}.partial"

log "copie $DB_PATH ($(( db_bytes/1048576 )) Mo) → $dest"
t0=$(date +%s)
# --reflink=auto : copie CoW instantanée si le FS le supporte (btrfs/xfs),
# sinon copie complète (ext4). Le fichier n'est renommé qu'une fois complet
# (évite qu'une copie interrompue passe pour une sauvegarde valide).
cp --reflink=auto "$DB_PATH" "$tmp"
sync "$tmp"
mv "$tmp" "$dest"
log "copie terminée en $(( $(date +%s) - t0 )) s"

# ── Rotation : ne garder que les KEEP plus récentes ──────────────────────────
mapfile -t backups < <(ls -1t "$BACKUP_DIR/${base_name}."* 2>/dev/null | grep -v '\.partial$' || true)
if [ "${#backups[@]}" -gt "$KEEP" ]; then
    for old in "${backups[@]:$KEEP}"; do
        log "purge ancienne sauvegarde : $old"
        rm -f "$old"
    done
fi

log "OK — $(ls -1 "$BACKUP_DIR/${base_name}."* 2>/dev/null | grep -vc '\.partial$') sauvegarde(s) conservée(s) dans $BACKUP_DIR"

# =============================================================================
# RESTAURATION (manuelle, service arrêté) :
#   sudo systemctl stop daly-bms
#   sudo mv /mnt/nvme/daly-bms/metrics.redb /mnt/nvme/daly-bms/metrics.redb.avant-restore
#   sudo cp /mnt/nvme/daly-bms/backups/metrics.redb.<TS> /mnt/nvme/daly-bms/metrics.redb
#   sudo chown dalybms:dalybms /mnt/nvme/daly-bms/metrics.redb
#   sudo systemctl start daly-bms
#
# SAUVEGARDE HORS-MACHINE (recommandé en complément) — exemple vers une autre
# machine, à mettre dans un cron/timer séparé :
#   rsync -a --delete /mnt/nvme/daly-bms/backups/ user@autre-hote:/backups/daly-bms/
# =============================================================================
