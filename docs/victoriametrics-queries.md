# VictoriaMetrics — Référence des métriques et requêtes PromQL

> URL VMUI : `http://192.168.1.141:8428/vmui/`
> URL API  : `http://192.168.1.141:8428/api/v1/query?query=<PROMQL>`

---
Pour convertir une courbe d’intensité (en Ampères) en une charge totale en Ampères-heures (Ah) sur une période définie, VictoriaMetrics met à disposition une fonction integrate dédiée dans son langage de requête MetricsQL.

Voici la commande PromQL complète, à exécuter via l’API REST depuis la ligne de commande avec curl. Cette requête simule une intégration sur les 6 dernières heures et convertit le résultat en Ampères-heures.

```bash
curl 'http://192.168.1.141:8428/api/v1/query_range?query=integrate(metric_current_amperes[6h])/3600&start=-6h&end=now&step=1m' --get
```

🧮 Synthèse de la requête

· Fonction integrate : Elle calcule l’aire sous la courbe de courant sur une fenêtre de 6 heures [6h]. C’est la méthode recommandée pour ce type de calcul de charge.
· Division par 3600 : L’intégrale est, par défaut, exprimée en A·s (Ampères-secondes). La division par 3600 est donc indispensable pour convertir le résultat en A·h (Ampères-heures).
· Détails de l’appel API :
  · Point d’entrée : /api/v1/query_range est l’URL de l’API de VictoriaMetrics.
  · Principaux paramètres :
    · query : Contient l’expression PromQL décrite ci-dessus.
    · start=-6h : Définit le début de la plage à il y a 6 heures.
    · end=now : Définit la fin de la plage à l’heure actuelle.
    · step=1m : Définit une résolution pour la série de données, ici une donnée par minute.

⚠️ À éviter : La fonction sum_over_time(metric_current_amperes[6h]) est parfois utilisée pour ce type de calcul, mais elle n’est pas adaptée. Elle fonctionne en sommant des points discrets sans tenir compte de la surface réelle sous la courbe de l’historique, ce qui la rend imprécise pour toutes les données variant dans le temps. Pour résumer :

· Mauvaise approche : sum_over_time(metric_current_amperes[6h]) (simple somme des points).
· Bonne approche : integrate(metric_current_amperes[6h])/3600 (intégrale correcte de la surface sous la courbe).

la documentation officielle de MetricsQL sur les fonctions de roulage.


## Récapitulatif des métriques par appareil

| Appareil | Métriques | Séries |
|----------|-----------|--------|
| BMS ×2 | bms_voltage, bms_current, bms_power, bms_soc, bms_capacity_ah, bms_cell_delta_mv, bms_temp_max, bms_temp_min, bms_charge_mos, bms_discharge_mos, bms_cell_voltage×16 | ~54 |
| ET112 ×3 | et112_voltage_v, et112_current_a, et112_power_w, et112_apparent_power_va, et112_power_factor, et112_frequency_hz, et112_energy_import_wh, et112_energy_export_wh | 24 |
| Irradiance | irradiance_wm2 | 1 |
| SmartShunt | venus_shunt_voltage_v, venus_shunt_current_a, venus_shunt_power_w, venus_shunt_soc_percent, venus_shunt_energy_in_kwh, venus_shunt_energy_out_kwh, venus_shunt_ah_charged_today, venus_shunt_ah_discharged_today | 8 |
| Solar agrégé | solar_total_w, mppt_power_w, solar_yield_kwh | 3 |
| Inverter (EasySolar II GX) | venus_inverter_voltage_v, venus_inverter_current_a, venus_inverter_power_w, venus_inverter_ac_output_voltage_v, venus_inverter_ac_output_current_a, venus_inverter_ac_output_power_w, venus_inverter_ac_freq_hz, venus_inverter_ac_in_ignore | 8 |
| MPPT ×2 | venus_mppt_power_w, venus_mppt_pv_voltage_v, venus_mppt_dc_current_a, venus_mppt_yield_today_kwh, venus_mppt_max_power_today_w | 10 |
| Température/Humidité | venus_temp_c, venus_humidity_percent | 2 |
| Heatpump ×2 (PAC/chauffe-eau) | venus_heatpump_state, venus_heatpump_power_w, venus_heatpump_energy_kwh, venus_heatpump_temp_c, venus_heatpump_target_temp_c | 10 |
| ATS CHINT | ats_sw1_closed, ats_sw2_closed, ats_active_source, ats_voltage_v×6, ats_freq_hz×2 | 11 |
| Tasmota ×6 | tasmota_power_on, tasmota_power_w, tasmota_voltage_v, tasmota_current_a, tasmota_energy_today_kwh | 30 |
| Shelly Pro 2PM | shelly_output×2, shelly_power_w×2, shelly_voltage_v×2, shelly_current_a×2, shelly_energy_wh×2 | 10 |
| **Total** | | **~171** |

---

## Labels utilisés

| Label | Valeurs exemples | Métriques concernées |
|-------|-----------------|----------------------|
| `bms_id` | `"0x01"`, `"0x02"` | bms_* |
| `cell` | `"C01"` … `"C16"` | bms_cell_voltage |
| `address` | `"0x05"`, `"0x07"`, `"0x08"`, `"0x09"` | et112_*, irradiance_wm2 |
| `name` | `"ET112-Micro-Onduleurs"`, `"Tongou Switch1"` … | et112_*, tasmota_*, shelly_* |
| `instance` | `"273"`, `"289"` | venus_mppt_* |
| `idx` | `"8"`, `"9"` | venus_heatpump_* |
| `source` | `"1"` (Onduleur), `"2"` (Réseau) | ats_voltage_v, ats_freq_hz |
| `phase` | `"a"`, `"b"`, `"c"` | ats_voltage_v |
| `id` | `"1"` … `"6"` (Tasmota), `"1"` (Shelly) | tasmota_*, shelly_* |
| `ch` | `"0"`, `"1"` | shelly_* |

---

## 1. BMS Daly (2 × 16 cellules)

```promql
# Tension totale BMS-1
bms_voltage{bms_id="0x01"}

# Courant instantané BMS-2
bms_current{bms_id="0x02"}

# SOC des deux BMS (instant)
bms_soc

# SOC moyen des deux BMS
avg(bms_soc)

# Delta cellule max (mV) — surveillance équilibrage
bms_cell_delta_mv

# Tension cellule C07 du BMS-1
bms_cell_voltage{bms_id="0x01", cell="C07"}

# Toutes les tensions de cellules du BMS-1 — heatmap
bms_cell_voltage{bms_id="0x01"}

# Température max BMS-1 sur 6h
bms_temp_max{bms_id="0x01"}[6h]

# État MOS charge BMS-1 (1=autorisé, 0=bloqué)
bms_charge_mos{bms_id="0x01"}

# Puissance totale des deux BMS
sum(bms_power)

# Énergie totale sur 24h (Wh = intégrale de la puissance)
# NB : nécessite des données continues
increase(bms_capacity_ah[24h])
```

---

## 2. ET112 Carlo Gavazzi (3 compteurs)

```promql
# Puissance active de chaque ET112
et112_power_w

# Puissance Micro-onduleurs uniquement
et112_power_w{address="0x07"}

# Puissance Maison (PAC Chauffe-eau)
et112_power_w{address="0x08"}

# Puissance Réseau (import = positif, export = négatif)
et112_power_w{address="0x09"}

# Tension réseau (ET112 Réseau)
et112_voltage_v{address="0x09"}

# Courant instantané tous ET112
et112_current_a

# Énergie importée totale ET112 Réseau (Wh cumulatif)
et112_energy_import_wh{address="0x09"}

# Énergie exportée ET112 Micro-onduleurs (Wh cumulatif)
et112_energy_export_wh{address="0x07"}

# Énergie importée sur les dernières 24h (delta)
increase(et112_energy_import_wh{address="0x09"}[24h])

# Facteur de puissance ET112 Réseau
et112_power_factor{address="0x09"}

# Fréquence réseau
et112_frequency_hz{address="0x09"}

# Puissance apparente
et112_apparent_power_va
```

---

## 3. Capteur Irradiance PRALRAN (RS485)

```promql
# Irradiance instantanée (W/m²)
irradiance_wm2

# Irradiance sur 6h (graphe temporel)
irradiance_wm2[6h]

# Moyenne irradiance sur 1h
avg_over_time(irradiance_wm2[1h])

# Irradiance max aujourd'hui
max_over_time(irradiance_wm2[24h])
```

---

## 4. SmartShunt Victron

```promql
# Courant batterie instantané (A — négatif = décharge)
venus_shunt_current_a

# Tension batterie (V)
venus_shunt_voltage_v

# Puissance batterie (W — négatif = décharge)
venus_shunt_power_w

# SOC batterie (%)
venus_shunt_soc_percent

# Énergie chargée aujourd'hui (kWh)
venus_shunt_energy_in_kwh

# Énergie déchargée aujourd'hui (kWh)
venus_shunt_energy_out_kwh

# Ah chargés aujourd'hui
venus_shunt_ah_charged_today

# Ah déchargés aujourd'hui
venus_shunt_ah_discharged_today

# Historique courant sur 6h
venus_shunt_current_a[6h]

# Bilan énergétique du jour (kWh)
venus_shunt_energy_in_kwh - venus_shunt_energy_out_kwh
```

---

## 5. Solar / MPPT agrégé

```promql
# Production solaire totale (MPPT1 + MPPT2 + Micro-onduleurs)
solar_total_w

# Puissance MPPT uniquement (sans micro-onduleurs)
mppt_power_w

# Production totale du jour (kWh)
solar_yield_kwh

# Historique production sur 24h
solar_total_w[24h]

# Production max instantanée sur la journée
max_over_time(solar_total_w[24h])
```

---

## 6. MPPT Victron SmartSolar (2 chargeurs)

> Instances : **273** (MPPT1) et **289** (MPPT2)

```promql
# Puissance de chaque MPPT
venus_mppt_power_w

# Puissance MPPT1 uniquement
venus_mppt_power_w{instance="273"}

# Puissance MPPT2 uniquement
venus_mppt_power_w{instance="289"}

# Tension panneau PV par MPPT
venus_mppt_pv_voltage_v

# Courant DC sortie chargeur (vers batterie)
venus_mppt_dc_current_a

# Production du jour par MPPT (kWh)
venus_mppt_yield_today_kwh

# Puissance max du jour par MPPT (W)
venus_mppt_max_power_today_w

# Puissance totale des 2 MPPT
sum(venus_mppt_power_w)

# Comparaison MPPT1 vs MPPT2
venus_mppt_power_w{instance=~"273|289"}
```

---

## 7. Inverter / EasySolar II GX

```promql
# Tension DC entrée onduleur (= tension batterie côté onduleur)
venus_inverter_voltage_v

# Courant DC entrée
venus_inverter_current_a

# Puissance DC totale consommée par l'onduleur
venus_inverter_power_w

# Tension AC sortie (V)
venus_inverter_ac_output_voltage_v

# Courant AC sortie (A)
venus_inverter_ac_output_current_a

# Puissance AC sortie (W)
venus_inverter_ac_output_power_w

# Fréquence AC sortie (Hz)
venus_inverter_ac_freq_hz

# Mode îlotage actif (1=AC input ignoré, 0=normal)
venus_inverter_ac_in_ignore

# Efficacité onduleur (%) — approximation
(venus_inverter_ac_output_power_w / venus_inverter_power_w) * 100

# Historique puissance AC sortie sur 6h
venus_inverter_ac_output_power_w[6h]
```

---

## 8. Capteur Température / Humidité

> Instance **20** — Température extérieure (Outdoor)

```promql
# Température extérieure (°C)
venus_temp_c{instance="20"}

# Humidité extérieure (%)
venus_humidity_percent{instance="20"}

# Température sur 24h
venus_temp_c[24h]

# Température min/max du jour
min_over_time(venus_temp_c[24h])
max_over_time(venus_temp_c[24h])
```

---

## 9. Heatpump / PAC (chauffe-eau ET112)

> Index MQTT : **8** (PAC Chauffe-eau), **9** (PAC Climatisation)
> State : 0=Off/Vacances, 1=Pompe chaleur, 2=Turbo

```promql
# État PAC Chauffe-eau (0=Off, 1=HEAT_PUMP, 2=Turbo)
venus_heatpump_state{idx="8"}

# Température eau courante (°C)
venus_heatpump_temp_c{idx="8"}

# Température eau cible (°C)
venus_heatpump_target_temp_c{idx="8"}

# Puissance consommée (W)
venus_heatpump_power_w{idx="8"}

# Énergie totale consommée (kWh cumulatif)
venus_heatpump_energy_kwh{idx="8"}

# Énergie consommée aujourd'hui (delta)
increase(venus_heatpump_energy_kwh{idx="8"}[24h])

# Puissance totale PAC + Climatisation
sum(venus_heatpump_power_w)
```

---

## 10. ATS CHINT NXZB

> active_source : 0=Onduleur (Source1 / AC2), 1=Réseau (Source2 / AC1), 2=Neutre

```promql
# Source active (0=Onduleur, 1=Réseau, 2=Neutre)
ats_active_source

# SW1 fermé = Onduleur alimenté (0/1)
ats_sw1_closed

# SW2 fermé = Réseau alimenté (0/1)
ats_sw2_closed

# Tension Source 1 (Onduleur) phase A
ats_voltage_v{source="1", phase="a"}

# Tension Source 2 (Réseau) phase A
ats_voltage_v{source="2", phase="a"}

# Toutes les tensions ATS
ats_voltage_v

# Fréquence Source 1 (Hz) — MN uniquement
ats_freq_hz{source="1"}

# Nombre de commutations Source1→Source2 sur 24h
increase(ats_active_source[24h])

# Historique source active sur 6h
ats_active_source[6h]
```

---

## 11. Tasmota Tongou (6 switchs)

> IDs : 1=Tongou Switch1, 2=Switch2, 3=Switch3, 4=Switch4, 5=Switch5, 6=Switch6 (tongou_3ACC34)

```promql
# État de tous les switchs (1=ON, 0=OFF)
tasmota_power_on

# État Tongou Switch1
tasmota_power_on{id="1"}

# Puissance consommée par switch
tasmota_power_w

# Puissance totale tous switchs
sum(tasmota_power_w)

# Tension alimentation (V)
tasmota_voltage_v

# Courant (A)
tasmota_current_a

# Énergie consommée aujourd'hui (kWh)
tasmota_energy_today_kwh

# Énergie totale tous switchs aujourd'hui
sum(tasmota_energy_today_kwh)

# Historique puissance Switch1 sur 24h
tasmota_power_w{id="1"}[24h]

# Switchs allumés en ce moment
tasmota_power_on == 1
```

---

## 12. Shelly Pro 2PM (DEYE)

> ID **1** — 2 canaux : ch=0 (canal 1), ch=1 (canal 2)

```promql
# État relais canal 0 (1=ON, 0=OFF)
shelly_output{id="1", ch="0"}

# État relais canal 1
shelly_output{id="1", ch="1"}

# Puissance canal 0 (W)
shelly_power_w{id="1", ch="0"}

# Puissance canal 1 (W)
shelly_power_w{id="1", ch="1"}

# Puissance totale Shelly (2 canaux)
sum(shelly_power_w{id="1"})

# Tension (V)
shelly_voltage_v{id="1", ch="0"}

# Courant (A)
shelly_current_a{id="1", ch="0"}

# Énergie totale canal 0 (Wh cumulatif)
shelly_energy_wh{id="1", ch="0"}

# Énergie canal 0 aujourd'hui (delta)
increase(shelly_energy_wh{id="1", ch="0"}[24h])
```

---

## Requêtes d'analyse globale

```promql
# Bilan énergétique instantané (W)
# Production totale
solar_total_w

# Consommation maison
et112_power_w{address="0x08"}

# Puissance réseau (+ = import, - = export)
et112_power_w{address="0x09"}

# État batterie global
venus_shunt_soc_percent

# Puissance batterie (+ = charge, - = décharge)
venus_shunt_power_w


# Vérification cardinality — toutes les métriques présentes
# VMUI → Explore → Cardinality
# ou : curl http://192.168.1.141:8428/api/v1/label/__name__/values | jq '.data | length'

# Dernier point de chaque métrique (vérification fraîcheur)
{__name__=~"bms_soc|venus_shunt_soc_percent|solar_total_w|ats_active_source|tasmota_power_on"}
```

---

## Throttles d'écriture VM

| Source | Intervalle écriture | Fréquence données source |
|--------|--------------------|--------------------------| 
| BMS | 5 s | 1 s (RS485) |
| ET112 | 5 s | 2 s (RS485) |
| Irradiance | 30 s | 5 s (RS485) |
| SmartShunt | 5 s | push MQTT (energy-manager) |
| Inverter | 5 s | push MQTT |
| MPPT | 10 s | push MQTT |
| Solar agrégé | 5 s | 1 s (POST energy-manager) |
| Température | 60 s | push MQTT |
| Heatpump | 10 s | push MQTT |
| ATS | 5 s | 2 s (RS485) |
| Tasmota | 10 s | push MQTT |
| Shelly | 10 s | push MQTT |
