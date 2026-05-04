'''
192.168.1.141:8080/api/v1/venus/inverter.  
{"connected":true,"inverter":{"ac_in_ignore":true,"ac_out_frequency_hz":50.07692337036133,"ac_output_current_a":-4.809999942779541,"ac_output_power_w":null,"ac_output_voltage_v":230.25999450683594,"current_a":15.5,"mode":"inverter","power_w":903.0,"state":"Inverting","timestamp":"2026-05-04T09:04:58.260337030Z","voltage_v":53.58000183105469}}
comme "ac_output_power_w":null nous n'avons pas de graphe et pas d'affichage de la valeur w dans le node Onduleur de la page visualization.html.

Dans l'interface victoriametrics, "venus_inverter_power_w" est bien present avec des valeures a jours et venus_inverter_ac_output_current_a.  

Il faut investiguer pour savoir pourquoi /api/v1/venus/inverter renvois cette valeur "null". 
'''
