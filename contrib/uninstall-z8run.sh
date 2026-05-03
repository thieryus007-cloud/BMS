#!/usr/bin/env bash
# =============================================================================
# uninstall-z8run.sh — Suppression complète de z8run sur Pi5
#
# Supprime : service systemd, binaire, config /etc/z8run, config nginx,
#            données /var/lib/z8run (optionnel).
#
# Usage : sudo bash ~/Daly-BMS-Rust/contrib/uninstall-z8run.sh
# =============================================================================

set -euo pipefail

if [[ $EUID -ne 0 ]]; then
    echo "Ce script doit être exécuté avec sudo."
    exit 1
fi

echo "=== Désinstallation de z8run ==="

# ── Service systemd ───────────────────────────────────────────────────────────

if systemctl is-active --quiet z8run 2>/dev/null; then
    systemctl stop z8run
    echo "→ Service z8run arrêté"
fi

if systemctl is-enabled --quiet z8run 2>/dev/null; then
    systemctl disable z8run
    echo "→ Service z8run désactivé"
fi

if [[ -f /etc/systemd/system/z8run.service ]]; then
    rm -f /etc/systemd/system/z8run.service
    echo "→ /etc/systemd/system/z8run.service supprimé"
fi

systemctl daemon-reload

# ── Binaire ───────────────────────────────────────────────────────────────────

if [[ -f /usr/local/bin/z8run ]]; then
    rm -f /usr/local/bin/z8run
    echo "→ /usr/local/bin/z8run supprimé"
fi

# ── Config /etc/z8run ─────────────────────────────────────────────────────────

if [[ -d /etc/z8run ]]; then
    rm -rf /etc/z8run
    echo "→ /etc/z8run supprimé"
fi

# ── Config nginx ──────────────────────────────────────────────────────────────

if [[ -L /etc/nginx/sites-enabled/z8run ]]; then
    rm -f /etc/nginx/sites-enabled/z8run
    echo "→ nginx sites-enabled/z8run supprimé"
fi

if [[ -f /etc/nginx/sites-available/z8run ]]; then
    rm -f /etc/nginx/sites-available/z8run
    echo "→ nginx sites-available/z8run supprimé"
fi

# Rétablir le site nginx default si absent
if [[ ! -L /etc/nginx/sites-enabled/default ]] && [[ -f /etc/nginx/sites-available/default ]]; then
    ln -sf /etc/nginx/sites-available/default /etc/nginx/sites-enabled/default
    echo "→ Site nginx 'default' rétabli"
fi

if command -v nginx &>/dev/null; then
    nginx -t 2>/dev/null && systemctl reload nginx 2>/dev/null || true
    echo "→ nginx rechargé"
fi

# ── Données /var/lib/z8run ────────────────────────────────────────────────────

if [[ -d /var/lib/z8run ]]; then
    read -r -p "Supprimer les données /var/lib/z8run (flows, DB) ? [o/N] " rep
    if [[ "${rep,,}" == "o" ]]; then
        rm -rf /var/lib/z8run
        echo "→ /var/lib/z8run supprimé"
    else
        echo "→ /var/lib/z8run conservé"
    fi
fi

echo ""
echo "=== z8run complètement désinstallé ==="
echo "  Sources ~/z8run conservées (supprimer manuellement si souhaité : rm -rf ~/z8run)"
