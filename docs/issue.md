# Problemes à Resoudre:

# 1/
```
192.168.1.141:8080/api/v1/venus/inverter
{"connected":true,"inverter":{"ac_in_ignore":true,"ac_out_frequency_hz":50.07692337036133,"ac_output_current_a":-4.809999942779541,"ac_output_power_w":null,"ac_output_voltage_v":230.25999450683594,"current_a":15.5,"mode":"inverter","power_w":903.0,"state":"Inverting","timestamp":"2026-05-04T09:04:58.260337030Z","voltage_v":53.58000183105469}}
comme "ac_output_power_w":null nous n'avons pas de graphe et pas d'affichage de la valeur w dans le node Onduleur de la page visualization.html.

Dans l'interface victoriametrics, "venus_inverter_power_w" est présent pour la puissance DC, mais ac_output_power_w absent pas pour AC OUT.
Cela doit etre ajouté a victoriametrics.

Il faut investiguer pour savoir pourquoi /api/v1/venus/inverter renvois cette valeur "null". 
```


# 2/
```
> On ne voit pas d'animation concernant la circulation du courant sur les edges entre.
>
>> 1- Onduleur et ATS-Onduleur.
>> 2- ATS-Onduleur et ATS-main.
```

# 3/
```
Revoir l'implementation de la logique de changement de mode du waterheater. 
**Probleme actuel**: Meme si les trois regles sont réunies pour que la commande "HEAT_PUMP" soit envoyée vers LG Think API, la commande n'est pas envoyée'.
Trois regles: SOC>90% et Irradiance >300 et ACIN ingnored =1 donc offgrid Alors envoyer à LG ThinQ mode HEAT_PUMP.
Le passage manuellement en mode HEAT_PUMP fonctionne.
Par ailleur, il arrive que la valeur d'irridiance au niveau du Victron VRM soit bloquée a une ancienne valeure, il serait preferable que la valeure utilisée pour la logic waterheater soit obtenue directement du capteur d 'irridiance. (actuellement, victoriametrics recoit directement cette valeur).
>> **Nous devrions avoir le process suivant**
>>>au demarage, evaluation des trois conditions, envois de la commande correspondante vers LG ThinQ.
>>>A interval regulier, 5 minutes, lire les informations (mode, temperature actuelle, temperature target)depuis LG ThinQ suivie d'evaluation des trois conditions et envois de la commande correspondante si necessaire.
>>>les trois informations,Doivent etre enregistrées dans victoriametrics.
```

# 4/ 
```
Dans la page monitoring: card regles, TROIS regles suivante OBLIGATOIRE:
	a)Gestion Courant Victron.
	b)Gestion du Chauffe-eau.
	c)Gestion des relais DEYE.
Pour chaque regles, nous devons avoir les VRAIS valeur retournées aprés interrogations de chaque elment "LG ThinQ/DEYE/Victron" et/ou les valeurs des elements stockés dans victoriametrics, avec le timestamp du dernier update. 
```
