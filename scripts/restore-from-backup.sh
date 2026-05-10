#!/usr/bin/env bash
# =============================================================================
# restore-from-backup.sh
# =============================================================================
# Restaure ce dépôt depuis un clone/backup local au cas où le nettoyage
# (commit "chore: audit cleanup …") aurait supprimé un fichier nécessaire.
#
# Usage :
#   bash scripts/restore-from-backup.sh /chemin/vers/clone-backup
#   bash scripts/restore-from-backup.sh /chemin/vers/clone-backup --dry-run
#
# Comportement :
#   - Vérifie que le clone backup existe et contient bien un dépôt git.
#   - rsync -a --delete (sauf .git, target/, *.lock runtime) du backup
#     vers le dépôt courant.
#   - Le dépôt courant garde son .git (historique conservé) — on rejoue
#     simplement les fichiers du backup par-dessus.
#   - Après restauration, lance `git status` pour visualiser ce qui a été
#     restauré (à committer manuellement si voulu).
#
# Alternatives natives git si tu n'as pas de clone séparé :
#   git revert <hash-du-commit-de-cleanup>          # revert via nouvel commit
#   git reset --hard <hash-avant-cleanup>           # destructif, attention
# =============================================================================

set -euo pipefail

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; NC='\033[0m'
info()  { echo -e "${GREEN}[OK]${NC} $*"; }
step()  { echo -e "${YELLOW}[>>]${NC} $*"; }
error() { echo -e "${RED}[!!]${NC} $*" >&2; exit 1; }

[ $# -ge 1 ] || error "Usage : $0 <chemin-clone-backup> [--dry-run]"
BACKUP="$1"
DRY_RUN=""
[ "${2:-}" = "--dry-run" ] && DRY_RUN="--dry-run"

CURRENT="$(git rev-parse --show-toplevel 2>/dev/null)" \
    || error "Le répertoire courant n'est pas un dépôt git."

[ -d "$BACKUP" ] || error "Backup introuvable : $BACKUP"
[ -d "$BACKUP/.git" ] || error "Le backup ne semble pas être un dépôt git : $BACKUP"

step "Dépôt courant : $CURRENT"
step "Backup source : $BACKUP"
[ -n "$DRY_RUN" ] && step "Mode dry-run (aucune modification)"

step "Restauration via rsync (préserve .git, ignore target/)…"
rsync -a $DRY_RUN \
    --delete \
    --exclude='.git/' \
    --exclude='target/' \
    --exclude='*.swp' \
    --exclude='.DS_Store' \
    "$BACKUP"/ "$CURRENT"/

info "Restauration terminée."

if [ -z "$DRY_RUN" ]; then
    step "État git après restauration :"
    cd "$CURRENT"
    git status --short
    echo ""
    info "Pour committer la restauration :"
    echo "  git add -A && git commit -m 'revert: restore from backup clone'"
    echo "  git push -u origin \$(git branch --show-current)"
    echo ""
    info "Pour annuler (revenir à l'état post-cleanup) :"
    echo "  git checkout -- ."
    echo "  git clean -fd"
fi
