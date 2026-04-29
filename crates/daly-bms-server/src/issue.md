La timeserie database est maintenant Tsink: https://github.com/h2337/tsink

L'erreur est claire : Tsink n'a pas les droits d'écriture dans le répertoire /var/lib/daly-bms/tsink/. Votre serveur tourne sous l'utilisateur dalybms, mais ce dossier appartient probablement à root ou à un autre utilisateur.

🔧 Solutions possibles (choisissez la plus adaptée)

1. Corriger les permissions du dossier existant (recommandé)

Exécutez ces commandes pour donner la propriété à dalybms :

```bash
sudo chown -R dalybms:dalybms /var/lib/daly-bms/tsink
sudo chmod 755 /var/lib/daly-bms/tsink
```

Ensuite, redémarrez votre serveur.

2. Changer le chemin de stockage dans la configuration

Si vous préférez utiliser un autre répertoire où dalybms a déjà les droits (par exemple son home), modifiez votre config.toml :

```toml
[tsink]
enabled = true
data_path = "/home/dalybms/tsink_data"   # ← chemin accessible
retention_days = 30
memory_limit_mb = 512
cardinality_limit = 10000
```

Créez le dossier et donnez les droits :

```bash
sudo -u dalybms mkdir -p /home/dalybms/tsink_data
```

3. Utiliser un répertoire temporaire (pour test uniquement)

```toml
data_path = "/tmp/dalybms_tsink"
```

Mais ce répertoire sera vidé au redémarrage. À éviter en production.

✅ Vérification après correction

Relancez le service et surveillez les logs :

```bash
sudo systemctl restart daly-bms   # ou votre commande de démarrage
journalctl -u daly-bms -f
```

Vous ne devriez plus voir l'erreur Permission denied. Tsink s'initialisera correctement et la charge CPU devrait se stabiliser.

Si le problème persiste, vérifiez que l'utilisateur dalybms a bien les droits sur le dossier parent (/var/lib/daly-bms) :

```bash
ls -ld /var/lib/daly-bms
```

Si nécessaire, ajustez avec sudo chown dalybms:dalybms /var/lib/daly-bms.


## I	Actions:
	-a) Retirer completement InfluxDB.
	-b) Retirer completement GRAFANA. 
	-c) Nettoyage de docker.
	-d) Fournir le process de redemarage de docker sans Grafana et sans InfluxDB. SEUL Process: mosquitto.
	-e) Fournir un script et process pour le nettoyage local (Pi5 compute) pour retirer les base de influxb.

## II	Dashboard natif Axum/Askama 
	Le frontend DOIT recevoir les flux en temps réel depuis une memoire allouée.( à definir, actuellement nous avons 20% occupée).
	Le frontend doit pouvoir acceder a Tsink pour l'historique. 
	L onglet historique du sahboard Doit avoir les graphes suivant avec la possibilité de choisir la periode de l'historique: 
	- jour
	- semaine
	- mois
	-année
	1) today energy kWh:
		importée
		solaire
		consommation

	2) today W:
		importée
		solaire
		consommation

	3) today Ah:
		importée
		solaire
		consommation

	4) today charge Ah vs today discharge Ah

