# Problemes à Resoudre:

# 1/
```
Constat:
la valeur du SOC dans la regle waterheater vient de soc_pct definit dans crates/daly-bms-server/src/state.rs comme venus_shunt_soc_percent.
La valeur du SOC qui est affichée dans la card rules chauffe-eau est la valeur calcullée des deux SOC BMS DALY.
Conclusions: 
Pour les deux pages suivantes, ON DOIT UTILISER le SOC donné par le SmartShunt Victron: venus_shunt_soc_percent.
	-1- page visualization card batterie SOC.
	-2- page Monitoring, id="card-rules": Gestion Chauffe-eau.

```


# 2/
```


```

# 3/
```
Revoir l'implementation de la logique de changement de mode du waterheater. 
** Probleme actuel ** : Meme si les trois regles sont réunies pour que la commande "HEAT_PUMP" soit envoyée vers LG Think API, la commande n'est pas envoyée'.
Trois regles: SOC>90% et Irradiance >300 et ACIN ingnored =1 donc offgrid Alors envoyer à LG ThinQ mode HEAT_PUMP.
Le passage manuellement en mode HEAT_PUMP fonctionne.
Par ailleur, il arrive que la valeur d'irridiance au niveau du Victron VRM soit bloquée a une ancienne valeure, il serait preferable que la valeure utilisée pour la logic waterheater soit obtenue directement du capteur d 'irridiance. (actuellement, victoriametrics recoit directement cette valeur).

>> ** Nous devrions avoir le process suivant **
>>>	- Au demarage, evaluation des trois conditions, envois de la commande correspondante vers LG ThinQ.
>>>	- A interval regulier, 5 minutes, lire les informations (mode, temperature actuelle, temperature target)depuis LG ThinQ suivie d'evaluation des trois conditions et envois de la commande correspondante si necessaire.
>>> - mettre en place une relecture depuis LG ThinQ et resend si necessaire. alerter en cas de resend trop frequent.
>>>	- les trois informations,Doivent etre enregistrées dans victoriametrics.

```

# 4/ 
```




```
