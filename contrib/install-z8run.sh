#!/usr/bin/env bash
# =============================================================================
# install-z8run.sh — Installation de z8run sur Pi5
#
# Le binaire z8run sert le frontend React depuis frontend/dist/ au runtime
# (lecture depuis le WorkingDirectory du service). Pas de nginx nécessaire.
#
# Prérequis :
#   - Rust 1.91+ (rustup)
#   - Node.js 22+ (installé automatiquement si absent)
#   - Source clonée dans ~/z8run
#
# Usage (sur le Pi5) :
#   git clone https://github.com/z8run/z8run.git ~/z8run
#   sudo bash ~/Daly-BMS-Rust/contrib/install-z8run.sh
# =============================================================================

set -euo pipefail

REAL_USER="${SUDO_USER:-${USER}}"
REAL_HOME=$(getent passwd "$REAL_USER" | cut -d: -f6)

Z8RUN_SRC="${REAL_HOME}/z8run"
BINARY_DEST="/usr/local/bin/z8run"
SERVICE_SRC="$(dirname "$0")/z8run.service"
SERVICE_DEST="/etc/systemd/system/z8run.service"
ENV_DIR="/etc/z8run"
ENV_FILE="${ENV_DIR}/.env"

# ── Vérifications ─────────────────────────────────────────────────────────────

if [[ $EUID -ne 0 ]]; then
    echo "Ce script doit être exécuté avec sudo."
    exit 1
fi

if [[ ! -d "$Z8RUN_SRC" ]]; then
    echo "Erreur : sources z8run non trouvées dans $Z8RUN_SRC"
    echo "  git clone https://github.com/z8run/z8run.git ~/z8run"
    exit 1
fi

# ── Désactiver nginx pour z8run si configuré (nettoyage ancienne install) ─────

if [[ -L /etc/nginx/sites-enabled/z8run ]]; then
    rm -f /etc/nginx/sites-enabled/z8run
    # Rétablir le site default si absent
    if [[ ! -L /etc/nginx/sites-enabled/default ]] && [[ -f /etc/nginx/sites-available/default ]]; then
        ln -sf /etc/nginx/sites-available/default /etc/nginx/sites-enabled/default
    fi
    systemctl reload nginx 2>/dev/null || true
    echo "→ Config nginx z8run désactivée (plus nécessaire)"
fi

# ── Node.js 22+ ───────────────────────────────────────────────────────────────

NODE_MAJOR=$(node --version 2>/dev/null | sed 's/v\([0-9]*\).*/\1/' || echo 0)
if [[ "$NODE_MAJOR" -lt 22 ]]; then
    echo "→ Node.js ${NODE_MAJOR} détecté, installation de Node.js 22 via NodeSource…"
    curl -fsSL https://deb.nodesource.com/setup_22.x | bash -
    apt-get install -y nodejs
    echo "→ Node.js $(node --version) installé"
else
    echo "→ Node.js $(node --version) OK"
fi

# ── Build frontend React ──────────────────────────────────────────────────────

echo "→ Build frontend React (npm install + npm run build)…"
sudo -u "$REAL_USER" bash -l -c "cd '$Z8RUN_SRC/frontend' && npm install && npm run build"

if [[ ! -d "$Z8RUN_SRC/frontend/dist" ]]; then
    echo "Erreur : frontend/dist/ absent après npm run build"
    exit 1
fi
echo "→ frontend/dist/ OK ($(find "$Z8RUN_SRC/frontend/dist" -type f | wc -l) fichiers)"

# ── Build Rust ────────────────────────────────────────────────────────────────

echo "→ Build z8run en release…"
sudo -u "$REAL_USER" bash -l -c "cd '$Z8RUN_SRC' && cargo build --release"

BINARY_SRC="${Z8RUN_SRC}/target/release/z8run"
if [[ ! -f "$BINARY_SRC" ]]; then
    echo "Erreur : binaire non trouvé après build"
    exit 1
fi

# ── Installation binaire ──────────────────────────────────────────────────────

echo "→ Installation du binaire → $BINARY_DEST"
install -m 755 -o root -g root "$BINARY_SRC" "$BINARY_DEST"

# ── Env ───────────────────────────────────────────────────────────────────────

mkdir -p "$ENV_DIR"

if [[ ! -f "$ENV_FILE" ]]; then
    JWT_SECRET=$(openssl rand -hex 32)
    cat > "$ENV_FILE" <<EOF
Z8_PORT=7700
Z8_BIND=0.0.0.0
Z8_JWT_SECRET=${JWT_SECRET}
EOF
    chmod 600 "$ENV_FILE"
    echo "→ .env généré"
else
    # Corriger port/bind si ancienne config
    sed -i 's/^Z8_PORT=7701/Z8_PORT=7700/' "$ENV_FILE"
    sed -i 's/^Z8_BIND=127\.0\.0\.1/Z8_BIND=0.0.0.0/' "$ENV_FILE"
    echo "→ .env mis à jour"
fi

# ── Service systemd ───────────────────────────────────────────────────────────

echo "→ Installation du service systemd"
cp "$SERVICE_SRC" "$SERVICE_DEST"
chmod 644 "$SERVICE_DEST"
systemctl daemon-reload
systemctl enable z8run
systemctl restart z8run

sleep 2

# ── Résultat ─────────────────────────────────────────────────────────────────

echo ""
systemctl status z8run --no-pager -l | head -20
echo ""
echo "Installation terminée."
echo "  UI z8run  : http://192.168.1.141:7700"
echo "  Santé API : curl http://localhost:7700/api/v1/health"
echo "  Logs      : journalctl -u z8run -f"
echo "  Config    : sudo nano $ENV_FILE"
