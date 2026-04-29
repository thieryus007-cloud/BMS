#!/usr/bin/env bash
# cleanup-influxdb.sh — Supprime InfluxDB et ses données du Pi5
# À exécuter SUR le Pi5 (pi5compute@192.168.1.141)
#
# Usage : bash scripts/cleanup-influxdb.sh

set -euo pipefail

echo "=== Nettoyage InfluxDB Pi5 ==="

# 1. Stopper et désactiver le service InfluxDB s'il tourne
if systemctl is-active --quiet influxdb 2>/dev/null; then
  echo "→ Arrêt service influxdb…"
  sudo systemctl stop influxdb
fi
if systemctl is-enabled --quiet influxdb 2>/dev/null; then
  echo "→ Désactivation service influxdb…"
  sudo systemctl disable influxdb
fi

# 2. Supprimer les volumes Docker InfluxDB s'ils existent
for vol in dalybms-influxdb-data dalybms-influxdb-config influxdb-data influxdb-config; do
  if docker volume inspect "$vol" &>/dev/null 2>&1; then
    echo "→ Suppression volume Docker $vol…"
    docker volume rm "$vol" || true
  fi
done

# 3. Supprimer les données InfluxDB locales
PATHS=(
  /var/lib/influxdb
  /var/lib/influxdb2
  /etc/influxdb
  /etc/influxdb2
  /root/.influxdbv2
  /home/pi5compute/.influxdbv2
)
for p in "${PATHS[@]}"; do
  if [ -d "$p" ] || [ -f "$p" ]; then
    echo "→ Suppression $p…"
    sudo rm -rf "$p"
  fi
done

# 4. Désinstaller le paquet InfluxDB s'il est installé
if dpkg -l influxdb2 &>/dev/null 2>&1; then
  echo "→ Désinstallation paquet influxdb2…"
  sudo apt-get remove --purge -y influxdb2 || true
fi
if dpkg -l influxdb &>/dev/null 2>&1; then
  echo "→ Désinstallation paquet influxdb…"
  sudo apt-get remove --purge -y influxdb || true
fi

echo ""
echo "✓ Nettoyage terminé."
echo "  Les données Tsink sont dans /var/lib/daly-bms/tsink — intactes."
