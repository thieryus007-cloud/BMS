# 1. Stopper le service
sudo systemctl stop daly-bms

# 2. Sauvegarder l'existant (au cas où)
sudo -u dalybms cp -r /var/lib/daly-bms/tsink /var/lib/daly-bms/tsink.backup.$(date +%Y%m%d%H%M)

# 3. Recréer un dossier tsink VIDE avec les bons droits
sudo rm -rf /var/lib/daly-bms/tsink
sudo -u dalybms mkdir -p /var/lib/daly-bms/tsink
sudo chmod 755 /var/lib/daly-bms/tsink

# 4. Redémarrer
sudo systemctl start daly-bms
journalctl -u daly-bms -f --no-pager
