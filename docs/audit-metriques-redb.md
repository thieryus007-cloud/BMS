# Audit des métriques reDB — Référence Grafana

> Généré le 2026-05-23. Source de vérité : `crates/daly-bms-server/src/redb_writes.rs`.

## Datasource Grafana

```
URL        : http://192.168.1.141:8080/api/v1/redb
Health     : GET /api/v1/redb/healthy
Type       : Prometheus (simple JSON)
```

Paramètres Grafana (datasource Prometheus custom) :
- **Scrape interval** : 15s (affichage), données écrites toutes les 5–60 s
- **Query timeout** : 30s

## Rétention & tiering automatique

| Tier    | Données                     | Rétention |
|---------|-----------------------------|-----------|
| Raw     | Points bruts (≤5 s)         | 30 jours  |
| Hourly  | Agrégats horaires (avg/min/max/sum/first/last) | 365 jours |
| Daily   | Agrégats journaliers        | 5 ans     |

Sélection automatique selon la plage Grafana :
- `≤ 7 jours` → tier Raw
- `7 j – 90 j` → tier Hourly
- `> 90 j` → tier Daily

**Fonctions PromQL supportées** (étendues via la roadmap promql-compat —
cf. `docs/promql-compat-roadmap.md`) :

- **Agrégation** : `sum max min avg count` (avec `by`/`without`),
  `topk` `bottomk`
- **Opérateurs** : `+ - * /` ; comparaisons `== != > < >= <=` (filtre ou
  `bool`)
- **Fenêtre** : `increase rate irate delta deriv predict_linear changes
  resets avg_over_time sum_over_time min_over_time max_over_time
  count_over_time last_over_time stddev_over_time stdvar_over_time
  quantile_over_time`
- **Instant / math** : `abs clamp clamp_min clamp_max ceil floor round
  sqrt exp ln log2 log10 sgn`
- **Labels** : `label_replace` `label_join`
- **Absence** : `absent` (vecteur instant) / `absent_over_time` (range)

**Non supporté** (rejeté en `bad_data`) : subqueries `[r:s]`, `offset`, `@`,
set ops `and`/`or`/`unless`, vector matching `on`/`ignoring`/`group_left|right`,
`%` `^`, `quantile`/`stddev`/`stdvar`/`group`/`count_values`,
`histogram_quantile`, `holt_winters`, `idelta`, `sort`/`sort_desc`,
`scalar`/`vector`, fonctions date/temps, trigonométrie, `{__name__=~"…"}`.

---

## 1. BMS — Batteries Daly

**Labels :** `bms_id` = `"0x01"` (360 Ah) | `"0x02"` (320 Ah)

**Intervalle écriture :** 5 s (mesures), 30 s (énergie), 60 s (temp)

### Tension / Courant / Puissance

| Métrique             | Unité | Description                    |
|----------------------|-------|--------------------------------|
| `bms_voltage`        | V     | Tension pack                   |
| `bms_current`        | A     | Courant (+ charge / − décharge)|
| `bms_power`          | W     | Puissance instantanée          |
| `bms_soc`            | %     | État de charge                 |

### Cellules

| Métrique              | Unité | Labels supplémentaires |
|-----------------------|-------|------------------------|
| `bms_cell_voltage`    | mV    | `cell` (1…N)           |
| `bms_cell_balancing`  | 0/1   | `cell`                 |
| `bms_min_cell_voltage`| mV    | —                      |
| `bms_max_cell_voltage`| mV    | —                      |
| `bms_cell_delta_mv`   | mV    | Delta max−min          |

### Températures

| Métrique         | Unité | Description         |
|------------------|-------|---------------------|
| `bms_temp_max`   | °C    | Temp. cellule max   |
| `bms_temp_min`   | °C    | Temp. cellule min   |
| `bms_mos_temp_c` | °C    | Temp. MOSFET        |

### Capacité & Énergie (intervalle 30 s)

| Métrique                    | Unité | Description                  |
|-----------------------------|-------|------------------------------|
| `bms_capacity_ah`           | Ah    | Capacité nominale installée  |
| `bms_capacity_remaining_ah` | Ah    | Capacité restante            |
| `bms_consumed_ah`           | Ah    | Ah consommés (cumul)         |
| `bms_reported_capacity_ah`  | Ah    | Capacité rapportée BMS       |
| `bms_total_ah_drawn`        | Ah    | Total Ah tirés (historique)  |
| `bms_charge_cycles`         | —     | Nombre de cycles             |

### Santé & Temps restant

| Métrique           | Unité | Description              |
|--------------------|-------|--------------------------|
| `bms_soh`          | %     | State of Health          |
| `bms_time_to_go_secs` | s  | Temps avant vide/plein   |

### Flags d'état (0/1)

| Métrique               | Description              |
|------------------------|--------------------------|
| `bms_balancing_active` | Équilibrage actif        |
| `bms_system_switch`    | Interrupteur système     |
| `bms_heating_active`   | Chauffage actif          |
| `bms_charge_mos`       | MOSFET charge activé     |
| `bms_discharge_mos`    | MOSFET décharge activé   |
| `bms_balance_mos`      | MOSFET équilibrage activé|
| `bms_heat_mos`         | MOSFET chauffage activé  |
| `bms_external_relay`   | Relais externe           |

### Limites de charge

| Métrique                   | Unité | Description              |
|----------------------------|-------|--------------------------|
| `bms_max_charge_voltage`   | V     | Tension max charge       |
| `bms_max_charge_current`   | A     | Courant max charge       |
| `bms_max_discharge_current`| A     | Courant max décharge     |
| `bms_max_charge_cell_voltage`| V   | Tension max par cellule  |
| `bms_charge_request`       | —     | Valeur de demande charge |

### Chauffage

| Métrique            | Unité | Description           |
|---------------------|-------|-----------------------|
| `bms_heating_current`| A    | Courant chauffage     |
| `bms_heating_power`  | W    | Puissance chauffage   |

### Modules (multi-BMS)

| Métrique                       | Description                  |
|--------------------------------|------------------------------|
| `bms_modules_online`           | Modules en ligne             |
| `bms_modules_offline`          | Modules hors ligne           |
| `bms_modules_blocking_charge`  | Modules bloquant la charge   |
| `bms_modules_blocking_discharge`| Modules bloquant la décharge|

### Historique extrêmes

| Métrique            | Unité |
|---------------------|-------|
| `bms_min_voltage_hist`| V   |
| `bms_max_voltage_hist`| V   |

### Alarmes (0/1)

| Métrique                        |
|---------------------------------|
| `bms_alarm_low_voltage`         |
| `bms_alarm_high_voltage`        |
| `bms_alarm_low_soc`             |
| `bms_alarm_high_charge_current` |
| `bms_alarm_high_discharge_current`|
| `bms_alarm_high_current`        |
| `bms_alarm_cell_imbalance`      |
| `bms_alarm_high_charge_temp`    |
| `bms_alarm_low_charge_temp`     |
| `bms_alarm_low_cell_voltage`    |
| `bms_alarm_low_temp`            |
| `bms_alarm_high_temp`           |
| `bms_alarm_fuse_blown`          |

**Requêtes PromQL types :**
```promql
# Tension totale des deux batteries
bms_voltage

# SOC BMS-360Ah
bms_soc{bms_id="0x01"}

# Delta cellules BMS-320Ah
bms_cell_delta_mv{bms_id="0x02"}

# Toutes les tensions de cellules BMS-360Ah
bms_cell_voltage{bms_id="0x01"}

# Alarmes actives
sum(bms_alarm_low_voltage + bms_alarm_high_voltage + bms_alarm_low_soc)
```

---

## 2. ET112 — Compteurs énergie AC

**Label :** `address` = adresse Modbus (`"7"`, `"8"`, `"9"`)

| Adresse | Rôle                     | D-Bus instance |
|---------|--------------------------|----------------|
| 7       | Micro-onduleurs (SN 119253X) | pvinverter.mqtt_7 (inst. 32) |
| 8       | Maison / Consommation (SN 119215X) | heatpump.mqtt_8 (inst. 30) |
| 9       | Réseau / Grid (SN 061077X) | heatpump.mqtt_9 (inst. 31) |

| Métrique                | Unité | Description          |
|-------------------------|-------|----------------------|
| `et112_power_w`         | W     | Puissance active     |
| `et112_voltage_v`       | V     | Tension              |
| `et112_current_a`       | A     | Courant              |
| `et112_apparent_power_va`| VA   | Puissance apparente  |
| `et112_power_factor`    | —     | Facteur de puissance |
| `et112_frequency_hz`    | Hz    | Fréquence            |
| `et112_reactive_power_var`| VAR | Puissance réactive   |
| `et112_energy_import_wh`| Wh    | Énergie importée (cumul) |
| `et112_energy_export_wh`| Wh    | Énergie exportée (cumul) |

**Requêtes PromQL types :**
```promql
# Puissance réseau (positif = import, négatif = export)
et112_power_w{address="9"}

# Consommation maison
et112_power_w{address="8"}

# Production micro-onduleurs
et112_power_w{address="7"}

# Énergie importée du réseau aujourd'hui (sur 24h)
increase(et112_energy_import_wh{address="9"}[24h])
```

---

## 3. Venus / Victron — MPPT Chargeurs solaires

**Label :** `instance` (numéro D-Bus de l'instance MPPT)

| Métrique                   | Unité | Description              |
|----------------------------|-------|--------------------------|
| `venus_mppt_power_w`       | W     | Puissance sortie MPPT    |
| `venus_mppt_yield_today_kwh`| kWh  | Rendement journalier     |
| `venus_mppt_max_power_today_w`| W  | Puissance max du jour    |
| `venus_mppt_pv_voltage_v`  | V     | Tension panneau PV       |
| `venus_mppt_dc_current_a`  | A     | Courant DC charge        |
| `venus_mppt_state`         | code  | État (0=Off, 3=Bulk, 4=Abs, 5=Float…) |

**Codes état MPPT :** 0=Off, 1=Low power, 2=Fault, 3=Bulk, 4=Absorption, 5=Float, 6=Storage, 7=Equalize, 9=Inverting, 11=Power supply

**Requêtes PromQL types :**
```promql
# Puissance totale tous MPPT
sum(venus_mppt_power_w)

# Rendement journalier par MPPT
venus_mppt_yield_today_kwh

# Tension PV
venus_mppt_pv_voltage_v
```

---

## 4. Venus / Victron — SmartShunt (moniteur batterie)

**Labels :** aucun (singleton)

| Métrique                    | Unité | Description               |
|-----------------------------|-------|---------------------------|
| `venus_shunt_soc_percent`   | %     | SOC SmartShunt            |
| `venus_shunt_voltage_v`     | V     | Tension batterie          |
| `venus_shunt_current_a`     | A     | Courant (+ charge)        |
| `venus_shunt_power_w`       | W     | Puissance                 |
| `venus_shunt_energy_in_kwh` | kWh   | Énergie chargée (cumul)   |
| `venus_shunt_energy_out_kwh`| kWh   | Énergie déchargée (cumul) |
| `venus_shunt_ah_charged_today`| Ah  | Ah chargés aujourd'hui    |
| `venus_shunt_ah_discharged_today`| Ah| Ah déchargés aujourd'hui |
| `venus_shunt_time_to_go_min`| min   | Temps restant estimé      |
| `venus_shunt_state`         | code  | 0=Idle, 1=Charging, 2=Discharging |

---

## 5. Venus / Victron — Onduleur/Chargeur

**Labels :** aucun (singleton)

| Métrique                       | Unité | Description            |
|--------------------------------|-------|------------------------|
| `venus_inverter_voltage_v`     | V     | Tension DC entrée      |
| `venus_inverter_current_a`     | A     | Courant DC entrée      |
| `venus_inverter_power_w`       | W     | Puissance DC entrée    |
| `venus_inverter_ac_output_power_w`| W  | Puissance AC sortie    |
| `venus_inverter_ac_output_voltage_v`| V| Tension AC sortie     |
| `venus_inverter_ac_output_current_a`| A| Courant AC sortie     |
| `venus_inverter_ac_freq_hz`    | Hz    | Fréquence AC           |
| `venus_inverter_ac_in_ignore`  | 0/1   | AC input ignoré        |
| `venus_inverter_state`         | code  | 0=Off, 1=On, 2=Inverting, 3=Charger, 4=Passthrough |
| `venus_inverter_mode`          | code  | 0=Charger, 1=Inverter, 2=Passthrough |

---

## 6. Venus / Victron — Températures

**Labels :** `instance`, `device_type="temperature"`

| Métrique                | Unité | Description           |
|-------------------------|-------|-----------------------|
| `venus_temp_c`          | °C    | Température           |
| `venus_humidity_percent`| %     | Humidité relative     |
| `venus_pressure_mbar`   | mbar  | Pression barométrique |
| `venus_connected`       | 0/1   | Connectivité capteur  |

---

## 7. Venus / Victron — Heatpumps (ET112 via D-Bus)

**Label :** `mqtt_index` (`"8"` = Maison, `"9"` = Réseau)

| Métrique                  | Unité | Description             |
|---------------------------|-------|-------------------------|
| `venus_heatpump_power_w`  | W     | Puissance AC            |
| `venus_heatpump_energy_kwh`| kWh  | Énergie (cumul)         |
| `venus_heatpump_state`    | code  | État opérationnel       |
| `venus_heatpump_temp_c`   | °C    | Température courante    |
| `venus_heatpump_target_temp_c`| °C | Température cible      |
| `venus_heatpump_position` | 0/1   | 0=AC Output, 1=AC Input |
| `venus_heatpump_connected`| 0/1   | Connexion               |

---

## 8. Solaire — Puissance & Rendement globaux

**Labels :** aucun (métriques système agrégées)

| Métrique           | Unité | Description                     |
|--------------------|-------|---------------------------------|
| `solar_total_w`    | W     | Puissance PV totale (MPPT + PVinv) |
| `total_solar_power`| W     | Alias Grafana (même valeur)     |
| `dc_pv_power_w`    | W     | Puissance MPPT seule            |
| `pvinv_power_w`    | W     | Puissance micro-onduleurs       |
| `solar_yield_kwh`  | kWh   | Rendement journalier total      |
| `solar_total_wh`   | Wh    | Rendement journalier en Wh      |

**Requêtes PromQL types :**
```promql
# Puissance solaire totale
solar_total_w

# Rendement journalier (croissant sur la journée)
solar_yield_kwh

# Répartition MPPT vs micro-onduleurs
dc_pv_power_w
pvinv_power_w
```

---

## 9. Irradiance — PRALRAN (addr. 0x05)

**Labels :** aucun (singleton)

| Métrique        | Unité | Description      |
|-----------------|-------|------------------|
| `irradiance_wm2`| W/m²  | Irradiance solaire|

---

## 10. ATS CHINT — Commutateur de source

**Labels :** aucun (singleton)

### Tensions par phase et source

| Métrique                            | Unité | Description          |
|-------------------------------------|-------|----------------------|
| `ats_v1a`, `ats_v1b`, `ats_v1c`    | V     | Source 1 (onduleur) phases A/B/C |
| `ats_v2a`, `ats_v2b`, `ats_v2c`    | V     | Source 2 (réseau) phases A/B/C   |
| `ats_voltage_v`                      | V     | Tension source active (moyenne)  |

### Fréquences

| Métrique       | Unité |
|----------------|-------|
| `ats_freq1_hz` | Hz    |
| `ats_freq2_hz` | Hz    |
| `ats_freq_hz`  | Hz    |

### État & Compteurs

| Métrique           | Unité | Description              |
|--------------------|-------|--------------------------|
| `ats_active_source`| 0/1/2 | 0=onduleur, 1=réseau, 2=neutre |
| `ats_sw1_closed`   | 0/1   | Contacteur source 1      |
| `ats_sw2_closed`   | 0/1   | Contacteur source 2      |
| `ats_sw_mode`      | code  | Mode commutation         |
| `ats_remote`       | 0/1   | Télécommande activée     |
| `ats_middle_off`   | 0/1   | Position milieu (coupure)|
| `ats_fault`        | 0-7   | Code défaut              |
| `ats_cnt1`         | —     | Compteur bascules S1     |
| `ats_cnt2`         | —     | Compteur bascules S2     |
| `ats_runtime_h`    | h     | Heures de fonctionnement |

### Statut de phase (0=Normal, 1=SousTension, 2=SurTension, 3=Erreur)

`ats_phase_s1a`, `ats_phase_s1b`, `ats_phase_s1c`,
`ats_phase_s2a`, `ats_phase_s2b`, `ats_phase_s2c`

### Maxima historiques

| Métrique    | Unité |
|-------------|-------|
| `ats_max1_v`| V     |
| `ats_max2_v`| V     |

---

## 11. Tasmota — Prises intelligentes

**Label :** `id` (identifiant Tasmota)

| Métrique                   | Unité | Description            |
|----------------------------|-------|------------------------|
| `tasmota_power_w`          | W     | Puissance active       |
| `tasmota_voltage_v`        | V     | Tension                |
| `tasmota_current_a`        | A     | Courant                |
| `tasmota_apparent_power_va`| VA    | Puissance apparente    |
| `tasmota_power_factor`     | —     | Facteur de puissance   |
| `tasmota_power_on`         | 0/1   | État relais            |
| `tasmota_energy_today_kwh` | kWh   | Énergie aujourd'hui    |
| `tasmota_energy_yesterday_kwh`| kWh| Énergie hier          |
| `tasmota_energy_total_kwh` | kWh   | Énergie totale (cumul) |
| `tasmota_rssi`             | dBm   | Signal WiFi (optionnel)|

---

## 12. Shelly EM — Compteurs énergie WiFi

**Label :** `id` (identifiant Shelly)

### Niveau appareil

| Métrique       | Unité | Description        |
|----------------|-------|--------------------|
| `shelly_power_w`| W    | Puissance totale   |
| `shelly_voltage_v`| V  | Tension            |
| `shelly_rssi`  | dBm   | Signal WiFi        |

### Niveau canal (`id` + `channel`)

| Métrique                | Unité | Description             |
|-------------------------|-------|-------------------------|
| `shelly_channel_power_w`| W     | Puissance canal         |
| `shelly_current_a`      | A     | Courant canal           |
| `shelly_output`         | 0/1   | Sortie canal            |
| `shelly_energy_wh`      | Wh    | Énergie (cumul)         |
| `shelly_power_factor`   | —     | Facteur de puissance    |
| `shelly_returned_wh`    | Wh    | Énergie retournée       |

---

## 13. Chauffe-eau LG ThinQ

**Labels :** aucun (singleton)

| Métrique          | Unité | Description           |
|-------------------|-------|-----------------------|
| `wh_current_temp_c`| °C   | Température eau actuelle |
| `wh_target_temp_c` | °C   | Température cible        |
| `wh_mode`          | code | Mode opérationnel        |

---

## 14. Pi5 — Monitoring système (daly-bms-server)

**Labels variés** (voir détails)

### CPU / Mémoire / Disque

| Métrique            | Unité | Label      | Description           |
|---------------------|-------|------------|-----------------------|
| `pi5_cpu_percent`   | %     | —          | Utilisation CPU       |
| `pi5_memory_percent`| %     | —          | Utilisation mémoire   |
| `pi5_mem_used_mb`   | MB    | —          | Mémoire utilisée      |
| `pi5_mem_total_mb`  | MB    | —          | Mémoire totale        |
| `pi5_swap_used_mb`  | MB    | —          | Swap utilisé          |
| `pi5_swap_total_mb` | MB    | —          | Swap total            |
| `pi5_disk_percent`  | %     | —          | Utilisation disque    |
| `pi5_load_avg`      | —     | `window` = `"1m"/"5m"/"15m"` | Charge système |

### Température & Réseau

| Métrique          | Unité | Description           |
|-------------------|-------|-----------------------|
| `pi5_cpu_temp_c`  | °C    | Température CPU       |
| `pi5_net_rx_bps`  | bps   | Débit réseau entrant  |
| `pi5_net_tx_bps`  | bps   | Débit réseau sortant  |
| `pi5_uptime_secs` | s     | Uptime système        |
| `pi5_serial_port_ok`| 0/1 | État port RS485       |

### Services & Processus

| Métrique                     | Label    | Description                |
|------------------------------|----------|----------------------------|
| `pi5_service_active`         | `name`   | Service systemd actif (0/1)|
| `pi5_network_service_active` | `name`   | Service réseau actif (0/1) |
| `pi5_process_cpu_percent`    | `process`| CPU par processus (%)      |
| `pi5_process_mem_mb`         | `process`| Mémoire par processus (MB) |

---

## 15. Energy Manager — Monitoring système

**Labels :** idem Pi5 pour `em_load_avg` (`window`)

| Métrique          | Unité | Description              |
|-------------------|-------|--------------------------|
| `em_cpu_percent`  | %     | CPU energy-manager       |
| `em_cpu_temp_c`   | °C    | Température CPU          |
| `em_memory_percent`| %    | Mémoire utilisée (%)     |
| `em_mem_used_mb`  | MB    | Mémoire utilisée         |
| `em_swap_used_mb` | MB    | Swap utilisé             |
| `em_disk_percent` | %     | Disque utilisé (%)       |
| `em_load_avg`     | —     | Charge système           |
| `em_net_rx_bps`   | bps   | Débit réseau entrant     |
| `em_net_tx_bps`   | bps   | Débit réseau sortant     |

---

## 16. Rule Engine — Métriques règles

**Label :** `rule` (nom de la règle)

| Métrique         | Description                   |
|------------------|-------------------------------|
| `rule_eval_total`| Compteur d'évaluations/règle  |

**Requête :**
```promql
rate(rule_eval_total[5m])
```

---

## Récapitulatif — Dashboards Grafana suggérés

| Dashboard            | Préfixe(s) métriques          | Labels clés           | Priorité |
|----------------------|-------------------------------|------------------------|----------|
| **Batteries BMS**    | `bms_*`                       | `bms_id`               | ★★★     |
| **Production solaire**| `solar_*`, `dc_pv_*`, `pvinv_*`, `venus_mppt_*`, `irradiance_*` | `instance` | ★★★ |
| **Réseau AC / ET112**| `et112_*`                     | `address`              | ★★★     |
| **Onduleur Victron** | `venus_inverter_*`, `venus_shunt_*` | —                | ★★★     |
| **Chauffe-eau**      | `wh_*`, `tasmota_*`           | `id`                   | ★★       |
| **ATS CHINT**        | `ats_*`                       | —                      | ★★       |
| **Système Pi5**      | `pi5_*`, `em_*`               | `name`, `process`, `window` | ★★  |
| **Températures**     | `bms_temp_*`, `bms_mos_temp_c`, `venus_temp_c`, `pi5_cpu_temp_c`, `wh_current_temp_c` | `bms_id`, `instance` | ★★ |
| **Alarmes**          | `bms_alarm_*`, `ats_fault`    | `bms_id`               | ★★       |
| **Tasmota / Shelly** | `tasmota_*`, `shelly_*`       | `id`, `channel`        | ★        |
| **Rule Engine**      | `rule_eval_total`             | `rule`                 | ★        |

---

## Commandes de diagnostic

```bash
# Lister toutes les séries en base
curl -s http://localhost:8080/api/v1/redb/series | jq '.data | length'
curl -s http://localhost:8080/api/v1/redb/series | jq '[.data[].metric.__name__] | unique | sort'

# Lister les labels disponibles
curl -s http://localhost:8080/api/v1/redb/labels | jq '.data'

# Valeurs du label bms_id
curl -s http://localhost:8080/api/v1/redb/label/bms_id/values | jq '.data'

# Test requête instant
curl -s "http://localhost:8080/api/v1/redb/query?query=bms_voltage&time=$(date +%s)" | jq .

# Test requête range (dernière heure)
curl -s "http://localhost:8080/api/v1/redb/query_range?query=solar_total_w&start=$(($(date +%s)-3600))&end=$(date +%s)&step=60" | jq .
```
