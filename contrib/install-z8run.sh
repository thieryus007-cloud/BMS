#!/usr/bin/env bash
# =============================================================================
# install-z8run.sh — Installation de z8run sur Pi5
#
# Architecture :
#   nginx :7700 (public)  → sert frontend/dist/ + proxifie vers z8run :7701
#   z8run :7701 (interne) → API REST/WS uniquement
#
# Prérequis :
#   - Rust 1.91+ (rustup)
#   - Node.js 22+ (installé automatiquement si absent)
#   - nginx (sudo apt install nginx)
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
NGINX_CONF_SRC="$(dirname "$0")/nginx-z8run.conf"
NGINX_CONF_DEST="/etc/nginx/sites-available/z8run"
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

# ── nginx ─────────────────────────────────────────────────────────────────────

if ! command -v nginx &>/dev/null; then
    echo "→ Installation de nginx…"
    apt-get install -y nginx
fi

# ── Build frontend React ──────────────────────────────────────────────────────

echo "→ Build frontend React (npm install + npm run build)…"
sudo -u "$REAL_USER" bash -l -c "cd '$Z8RUN_SRC/frontend' && npm install && npm run build"

# ── Build Rust ────────────────────────────────────────────────────────────────

echo "→ Build z8run backend en release…"
sudo -u "$REAL_USER" bash -l -c "touch '$Z8RUN_SRC/build.rs' 2>/dev/null; cd '$Z8RUN_SRC' && cargo build --release"

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
# z8run écoute sur 7701 (interne) — nginx proxifie sur 7700 (public)
Z8_PORT=7701
Z8_BIND=127.0.0.1
Z8_DATA_DIR=${DATA_DIR}
Z8_JWT_SECRET=${JWT_SECRET}
EOF
    chmod 600 "$ENV_FILE"
    echo "→ .env généré"
else
    # Mettre à jour le port si nécessaire
    sed -i 's/^Z8_PORT=7700/Z8_PORT=7701/' "$ENV_FILE"
    sed -i 's/^Z8_BIND=0\.0\.0\.0/Z8_BIND=127.0.0.1/' "$ENV_FILE"
    echo "→ .env existant mis à jour (port 7701, bind 127.0.0.1)"
fi

# ── Service systemd z8run ─────────────────────────────────────────────────────

echo "→ Installation du service systemd z8run"
cp "$SERVICE_SRC" "$SERVICE_DEST"
chmod 644 "$SERVICE_DEST"
systemctl daemon-reload
systemctl enable z8run
systemctl restart z8run

# ── Config nginx ──────────────────────────────────────────────────────────────

echo "→ Installation de la config nginx"
cp "$NGINX_CONF_SRC" "$NGINX_CONF_DEST"
ln -sf "$NGINX_CONF_DEST" /etc/nginx/sites-enabled/z8run

# Désactiver le site default si présent (évite conflit de port)
if [[ -L /etc/nginx/sites-enabled/default ]]; then
    rm -f /etc/nginx/sites-enabled/default
    echo "→ Site nginx 'default' désactivé"
fi

nginx -t
systemctl enable nginx
systemctl restart nginx

# ── Résultat ─────────────────────────────────────────────────────────────────

sleep 2
echo ""
echo "=== z8run backend ==="
systemctl status z8run --no-pager -l | head -20
echo ""
echo "=== nginx ==="
systemctl status nginx --no-pager -l | head -10
echo ""
echo "Installation terminée."
echo "  UI z8run  : http://192.168.1.141:7700"
echo "  API santé : curl http://localhost:7701/api/v1/health"
echo "  Logs z8run: journalctl -u z8run -f"
echo "  Logs nginx: tail -f /var/log/nginx/z8run_error.log"
echo "  Config env: sudo nano $ENV_FILE"
