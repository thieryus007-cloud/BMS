La timeserie database est maintenant Tsink: https://github.com/h2337/tsink

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

