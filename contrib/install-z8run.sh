#!/usr/bin/env bash
# =============================================================================
# install-z8run.sh — Installation de z8run sur Pi5
#
# Prérequis :
#   - Rust 1.91+ (rustup)
#   - Node.js 22+ (nvm ou apt)
#   - Source clonée dans ~/z8run
#
# Usage (sur le Pi5) :
#   git clone https://github.com/z8run/z8run.git ~/z8run
#   sudo bash ~/Daly-BMS-Rust/contrib/install-z8run.sh
# =============================================================================

set -euo pipefail

# Résoudre le home de l'utilisateur réel même sous sudo
REAL_USER="${SUDO_USER:-${USER}}"
REAL_HOME=$(getent passwd "$REAL_USER" | cut -d: -f6)

Z8RUN_SRC="${REAL_HOME}/z8run"
BINARY_DEST="/usr/local/bin/z8run"
SERVICE_SRC="$(dirname "$0")/z8run.service"
SERVICE_DEST="/etc/systemd/system/z8run.service"
ENV_DIR="/etc/z8run"
ENV_FILE="${ENV_DIR}/.env"
DATA_DIR="/var/lib/z8run"

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

# ── Build ─────────────────────────────────────────────────────────────────────

echo "→ Build frontend React (npm install + npm run build)…"
sudo -u "$REAL_USER" bash -l -c "cd '$Z8RUN_SRC/frontend' && npm install && npm run build"

echo "→ Build z8run en release (Rust embarque le frontend)…"
# Shell login pour avoir ~/.cargo/bin dans le PATH (rustup)
sudo -u "$REAL_USER" bash -l -c "cd '$Z8RUN_SRC' && cargo build --release"

BINARY_SRC="${Z8RUN_SRC}/target/release/z8run"
if [[ ! -f "$BINARY_SRC" ]]; then
    echo "Erreur : binaire non trouvé après build : $BINARY_SRC"
    exit 1
fi

# ── Installation binaire ──────────────────────────────────────────────────────

echo "→ Installation du binaire → $BINARY_DEST"
install -m 755 -o root -g root "$BINARY_SRC" "$BINARY_DEST"

# ── Répertoires et env ────────────────────────────────────────────────────────

echo "→ Création des répertoires"
mkdir -p "$DATA_DIR" "$ENV_DIR"
chown "$REAL_USER:$REAL_USER" "$DATA_DIR"
chmod 750 "$DATA_DIR"

if [[ ! -f "$ENV_FILE" ]]; then
    echo "→ Génération du fichier .env"
    JWT_SECRET=$(openssl rand -hex 32)
    cat > "$ENV_FILE" <<EOF
Z8_PORT=7700
Z8_BIND=0.0.0.0
Z8_DATA_DIR=${DATA_DIR}
# SQLite embarqué par défaut — remplacer par postgres:// si besoin
# Z8_DB_URL=sqlite://${DATA_DIR}/z8run.db
Z8_JWT_SECRET=${JWT_SECRET}
EOF
    chmod 600 "$ENV_FILE"
    echo "→ .env généré avec JWT_SECRET aléatoire"
else
    echo "→ .env existant conservé : $ENV_FILE"
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
systemctl status z8run --no-pager -l
echo ""
echo "Installation terminée."
echo "  UI z8run  : http://192.168.1.141:7700"
echo "  Logs      : journalctl -u z8run -f"
echo "  Config env: sudo nano $ENV_FILE"
