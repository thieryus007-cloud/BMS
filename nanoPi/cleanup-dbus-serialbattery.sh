#!/bin/bash
# =============================================================================
# cleanup-dbus-serialbattery.sh
# Diagnostic et nettoyage complet de dbus-serialbattery + overlay-fs
# sur Venus OS (NanoPI)
#
# Usage: bash cleanup-dbus-serialbattery.sh
# Copier la sortie complète avant de rebooter.
# =============================================================================

SEP="─────────────────────────────────────────────────────────"
FOUND_ITEMS=()

log()   { echo "[INFO]  $*"; }
found() { echo "[FOUND] $*"; FOUND_ITEMS+=("$*"); }
warn()  { echo "[WARN]  $*"; }
ok()    { echo "[OK]    $*"; }
step()  { echo; echo "$SEP"; echo ">>> $*"; echo "$SEP"; }

# =============================================================================
step "1. PROCESSUS EN COURS"
# =============================================================================

log "Recherche processus dbus-serialbattery..."
PROCS=$(ps aux 2>/dev/null | grep -i "serialbattery\|dbus-batt" | grep -v grep)
if [ -n "$PROCS" ]; then
    found "Processus actif :"
    echo "$PROCS"
else
    ok "Aucun processus dbus-serialbattery actif"
fi

log "Recherche processus overlay-fs..."
OFS_PROCS=$(ps aux 2>/dev/null | grep -i "overlay-fs\|overlay_fs" | grep -v grep)
if [ -n "$OFS_PROCS" ]; then
    found "Processus overlay-fs actif :"
    echo "$OFS_PROCS"
else
    ok "Aucun processus overlay-fs actif"
fi

# =============================================================================
step "2. SERVICES VENUS OS (svctl / /service)"
# =============================================================================

log "Listing /service/ ..."
if [ -d /service ]; then
    SVC=$(ls /service/ | grep -i "serialbattery\|can\|batt")
    if [ -n "$SVC" ]; then
        found "Services trouvés dans /service/ : $SVC"
        for s in $SVC; do
            echo "  → /service/$s"
            ls -la /service/$s 2>/dev/null
        done
    else
        ok "Aucun service serialbattery/can dans /service/"
    fi
fi

log "Listing /opt/victronenergy/service/ ..."
if [ -d /opt/victronenergy/service ]; then
    SVC2=$(ls /opt/victronenergy/service/ | grep -i "serialbattery\|batt")
    if [ -n "$SVC2" ]; then
        found "Services dans /opt/victronenergy/service/ : $SVC2"
    else
        ok "Aucun service serialbattery dans /opt/victronenergy/service/"
    fi
fi

# =============================================================================
step "3. RÉPERTOIRES PRINCIPAUX"
# =============================================================================

DIRS_TO_CHECK=(
    "/data/etc/dbus-serialbattery"
    "/data/apps/dbus-serialbattery"
    "/opt/victronenergy/dbus-serialbattery"
    "/data/apps/overlay-fs"
    "/opt/victronenergy/overlay-fs"
    "/data/conf/serial-starter.d"
)

for d in "${DIRS_TO_CHECK[@]}"; do
    if [ -d "$d" ]; then
        SIZE=$(du -sh "$d" 2>/dev/null | cut -f1)
        found "Répertoire présent : $d ($SIZE)"
        ls -la "$d" 2>/dev/null | head -30
        echo "..."
    else
        ok "Absent : $d"
    fi
done

# =============================================================================
step "4. FICHIERS OVERLAY-FS (données GUI)"
# =============================================================================

log "Recherche données overlay dans /data/apps/overlay-fs/data/ ..."
if [ -d /data/apps/overlay-fs/data ]; then
    found "Données overlay présentes :"
    find /data/apps/overlay-fs/data -type f 2>/dev/null | while read f; do
        echo "  $f"
    done
else
    ok "Pas de données overlay-fs/data"
fi

# =============================================================================
step "5. INTERFACE CAN0"
# =============================================================================

log "État interface CAN0 ..."
ip link show can0 2>/dev/null && found "Interface can0 PRÉSENTE" || ok "Interface can0 absente"

log "Recherche scripts de configuration CAN ..."
CAN_SCRIPTS=$(find /data /etc /opt -name "*.sh" -o -name "rc.local" 2>/dev/null | \
    xargs grep -l "can0\|socketcan\|mcp251" 2>/dev/null)
if [ -n "$CAN_SCRIPTS" ]; then
    found "Scripts mentionnant can0 :"
    echo "$CAN_SCRIPTS"
    for f in $CAN_SCRIPTS; do
        echo "  --- $f ---"
        grep -n "can0\|socketcan\|mcp251" "$f"
    done
else
    ok "Aucun script de config CAN trouvé"
fi

log "Vérification /etc/network/interfaces ..."
if grep -q "can0" /etc/network/interfaces 2>/dev/null; then
    found "can0 mentionné dans /etc/network/interfaces :"
    grep -n "can0" /etc/network/interfaces
fi

log "Vérification /data/rc.local ..."
if [ -f /data/rc.local ] && grep -q "can0\|serialbattery" /data/rc.local 2>/dev/null; then
    found "Entrées can0/serialbattery dans /data/rc.local :"
    grep -n "can0\|serialbattery" /data/rc.local
fi

# =============================================================================
step "6. FICHIERS DE CONFIGURATION VENUS"
# =============================================================================

log "Recherche dans /data/conf/ ..."
CONF_FILES=$(find /data/conf -type f 2>/dev/null | xargs grep -l "serialbattery\|can0" 2>/dev/null)
if [ -n "$CONF_FILES" ]; then
    found "Fichiers conf mentionnant serialbattery/can0 :"
    for f in $CONF_FILES; do
        echo "  --- $f ---"
        grep -n "serialbattery\|can0" "$f"
    done
else
    ok "Rien dans /data/conf/"
fi

log "Recherche serial-starter config ..."
if [ -d /data/conf/serial-starter.d ]; then
    found "serial-starter.d présent :"
    ls -la /data/conf/serial-starter.d/
fi
if [ -f /etc/serial-starter.d/dbus-serialbattery ]; then
    found "/etc/serial-starter.d/dbus-serialbattery présent"
fi

# =============================================================================
step "7. EXTENSIONS GUI (overlay)"
# =============================================================================

log "Recherche fichiers GUI overlay de serialbattery ..."
GUI_FILES=$(find /opt/victronenergy/gui /www 2>/dev/null | grep -i "serialbattery\|dbus-batt" 2>/dev/null)
if [ -n "$GUI_FILES" ]; then
    found "Fichiers GUI overlay :"
    echo "$GUI_FILES"
else
    ok "Aucun fichier GUI overlay serialbattery"
fi

log "Vérification /data/themes/ ..."
if [ -d /data/themes ]; then
    THEME_FILES=$(find /data/themes -name "*serialbattery*" 2>/dev/null)
    [ -n "$THEME_FILES" ] && found "Fichiers themes : $THEME_FILES" || ok "Rien dans /data/themes"
fi

# =============================================================================
step "8. LOGS"
# =============================================================================

log "Dernières lignes de log serialbattery ..."
for logf in /var/log/dbus-serialbattery* /data/log/dbus-serialbattery*; do
    if [ -f "$logf" ]; then
        found "Log trouvé : $logf"
        tail -5 "$logf"
    fi
done

# =============================================================================
step "9. RÉSUMÉ DE CE QUI A ÉTÉ TROUVÉ"
# =============================================================================

echo
if [ ${#FOUND_ITEMS[@]} -eq 0 ]; then
    echo "✓ Rien trouvé — système déjà propre."
else
    echo "⚠ Éléments à supprimer (${#FOUND_ITEMS[@]}) :"
    for item in "${FOUND_ITEMS[@]}"; do
        echo "  • $item"
    done
fi

# =============================================================================
step "10. NETTOYAGE"
# =============================================================================

echo
read -r -p "Lancer le nettoyage ? [y/N] " CONFIRM
CONFIRM=${CONFIRM,,}

if [[ ! "$CONFIRM" =~ ^(y|yes|oui|o)$ ]]; then
    echo "Nettoyage annulé. Copiez la sortie ci-dessus pour analyse."
    exit 0
fi

echo
log "Démarrage du nettoyage..."

# Arrêter les services s'ils tournent
if [ -d /service/dbus-serialbattery ]; then
    log "Arrêt service dbus-serialbattery..."
    svc -d /service/dbus-serialbattery 2>/dev/null && ok "Service arrêté" || warn "Impossible d'arrêter"
    rm -f /service/dbus-serialbattery
    log "Symlink /service/dbus-serialbattery supprimé"
fi

# Désactiver + supprimer overlay-fs
if [ -f /data/apps/overlay-fs/uninstall.sh ]; then
    log "Désactivation overlay-fs..."
    bash /data/apps/overlay-fs/disable.sh 2>/dev/null
    log "Suppression /data/apps/overlay-fs/ ..."
    rm -rf /data/apps/overlay-fs
    ok "overlay-fs supprimé"
elif [ -d /data/apps/overlay-fs ]; then
    log "Suppression /data/apps/overlay-fs/ (pas de uninstall.sh)..."
    rm -rf /data/apps/overlay-fs
    ok "overlay-fs supprimé"
fi

# Supprimer le driver principal
for d in \
    /data/etc/dbus-serialbattery \
    /data/apps/dbus-serialbattery \
    /opt/victronenergy/dbus-serialbattery; do
    if [ -d "$d" ]; then
        rm -rf "$d"
        ok "Supprimé : $d"
    fi
done

# Supprimer config serial-starter
if [ -f /data/conf/serial-starter.d/dbus-serialbattery ]; then
    rm -f /data/conf/serial-starter.d/dbus-serialbattery
    ok "serial-starter config supprimé"
fi
if [ -f /etc/serial-starter.d/dbus-serialbattery ]; then
    rm -f /etc/serial-starter.d/dbus-serialbattery
    ok "serial-starter /etc config supprimé"
fi

# Supprimer les données overlay GUI
for d in /www/dbus-serialbattery /opt/victronenergy/gui/dbus-serialbattery; do
    if [ -d "$d" ]; then
        rm -rf "$d"
        ok "GUI overlay supprimé : $d"
    fi
done

echo
echo "$SEP"
echo ">>> NETTOYAGE TERMINÉ"
echo "$SEP"
echo
echo "Veuillez copier cette sortie complète, puis rebooter :"
echo "  reboot"
echo
