
Problemes à Resoudre page visualisation

1) Nous avions pour le node Easysolar II, Onduleur, au depart du edge pour aller vers le node ATS Onduleur, l'affichage de la frequence en Hz avec deux digit apres la virgule. Il faut retablir cette information. a jouter aussi vers influxDB pour que ce soit enregistré.

2) Actuellement pour le node sankey production, il n'affiche pas de flux de production quand on se troiuve en mode decharge. Il faut aussi avoir un graphe sankey  pour le mode decharge.'

3) Actuellment; nous avons pour le node SmartSolar MPPT, uniquement la ligne MPPT-1, alors que nous devrions avoir MPPT-273 et MPPT-289, meme quand le courant total est nul. le MPPT doivent afficher aussi leur status. (c'etait le cas avant).

4) Pour le node Smartshunt, les information chargée et dechargée ne sont jamais presente. elle doivent correspondre au total Ah chargée sur les 24 dernieres heures et dechargée sur les 24 derniere heures. a jouter aussi vers influxDB pour que ce soit enregistré.

5) 	pour le node custom spirale en haut a gauche, cercle kWh devrait etre un arc de 270°, comme les deux autres, avec une plage de 0 à 40kWh correspondant au total cumulé d'energie solaire de la journée. ces informations doivent etre rajoutées vers influxDB pour etre exploité dans la page.

6) actuellement dans la page monitoring le service energy-manager est noté inactive et unreacheable alors que ce n'est pas le cas il me semble. a Investiguer et corriger.

7) pour les pages suivantes, Comteurs ET112 Tasmota Monitoring,les bandeaux de chaque elements doivent etre coherent par rapport au style deja mis en place dans la page Batterie BMS. 

