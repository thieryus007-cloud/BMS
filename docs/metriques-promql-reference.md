# Catalogue des métriques & référence PromQL — Daly-BMS-Rust

> Catalogue exhaustif des métriques stockées dans le **metrics-store (redb)** et référence
> du **sous-ensemble PromQL** servi par `daly-bms-server` sur le port **8080**
> (`/api/v1/query`, `/api/v1/query_range`, `/api/v1/labels`). Inclut les requêtes d'exemple
> par appareil, la roadmap d'implémentation et l'audit de conformité.
>
> ⚠️ **Convention de labels critique** : les adresses sont écrites en **hexadécimal**
> (`address="0x07"`, `address="0x08"`, `address="0x09"` — cf. `redb_writes.rs::write_et112`),
> **jamais en décimal** (`address="7"` → 0 série). Toute requête PromQL doit utiliser la forme hex.
>
> **Source de vérité : le code réel** (`crates/metrics-store/`, `crates/daly-bms-server/src/redb_writes.rs`).
> Fait partie de l'[architecture documentaire](./ARCHITECTURE.md). Dernière consolidation : 2026-06-07.

## Table des matières

- [Catalogue des métriques (write-path / référence Grafana)](#catalogue-des-métriques-write-path--référence-grafana)
- [Référence des métriques et requêtes PromQL (par appareil)](#référence-des-métriques-et-requêtes-promql-par-appareil)
- [Sous-ensemble PromQL supporté — roadmap d'implémentation](#sous-ensemble-promql-supporté--roadmap-dimplémentation)
- [Conformité PromQL — audit détaillé](#conformité-promql--audit-détaillé)

## Voir aussi

- [Architecture des métriques — redb](./metriques-redb-architecture.md) — moteur de stockage, schéma, tiering, write/read path.
- [Grafana & dashboards](./grafana-dashboards.md) — datasource PromQL (UID `daly-metrics`) et panels.
- [Intégration matériel](./integration-materiel.md) — appareils sources des métriques (inventaire RS485/D-Bus).
- [daly-bms-server](./app-daly-bms-server.md) — serveur exposant le shim PromQL.

---

## Catalogue des métriques (write-path / référence Grafana)

> Liste exhaustive des métriques effectivement écrites dans redb (source :
> `crates/daly-bms-server/src/redb_writes.rs`), avec leurs labels et regroupées par appareil.


> Généré le 2026-05-23. Source de vérité : `crates/daly-bms-server/src/redb_writes.rs`.

### Datasource Grafana

```
URL        : http://192.168.1.141:8080/api/v1/redb
Health     : GET /api/v1/redb/healthy
Type       : Prometheus (simple JSON)
```

Paramètres Grafana (datasource Prometheus custom) :
- **Scrape interval** : 15s (affichage), données écrites toutes les 5–60 s
- **Query timeout** : 30s

### Rétention & tiering automatique

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
cf. `./metriques-promql-reference.md`) :

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

### 1. BMS — Batteries Daly

**Labels :** `bms_id` = `"0x01"` (360 Ah) | `"0x02"` (320 Ah)

**Intervalle écriture :** 5 s (mesures), 30 s (énergie), 60 s (temp)

#### Tension / Courant / Puissance

| Métrique             | Unité | Description                    |
|----------------------|-------|--------------------------------|
| `bms_voltage`        | V     | Tension pack                   |
| `bms_current`        | A     | Courant (+ charge / − décharge)|
| `bms_power`          | W     | Puissance instantanée          |
| `bms_soc`            | %     | État de charge                 |

#### Cellules

| Métrique              | Unité | Labels supplémentaires |
|-----------------------|-------|------------------------|
| `bms_cell_voltage`    | mV    | `cell` (1…N)           |
| `bms_cell_balancing`  | 0/1   | `cell`                 |
| `bms_min_cell_voltage`| mV    | —                      |
| `bms_max_cell_voltage`| mV    | —                      |
| `bms_cell_delta_mv`   | mV    | Delta max−min          |

#### Températures

| Métrique         | Unité | Description         |
|------------------|-------|---------------------|
| `bms_temp_max`   | °C    | Temp. cellule max   |
| `bms_temp_min`   | °C    | Temp. cellule min   |
| `bms_mos_temp_c` | °C    | Temp. MOSFET        |

#### Capacité & Énergie (intervalle 30 s)

| Métrique                    | Unité | Description                  |
|-----------------------------|-------|------------------------------|
| `bms_capacity_ah`           | Ah    | Capacité nominale installée  |
| `bms_capacity_remaining_ah` | Ah    | Capacité restante            |
| `bms_consumed_ah`           | Ah    | Ah consommés (cumul)         |
| `bms_reported_capacity_ah`  | Ah    | Capacité rapportée BMS       |
| `bms_total_ah_drawn`        | Ah    | Total Ah tirés (historique)  |
| `bms_charge_cycles`         | —     | Nombre de cycles             |

#### Santé & Temps restant

| Métrique           | Unité | Description              |
|--------------------|-------|--------------------------|
| `bms_soh`          | %     | State of Health          |
| `bms_time_to_go_secs` | s  | Temps avant vide/plein   |

#### Flags d'état (0/1)

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

#### Limites de charge

| Métrique                   | Unité | Description              |
|----------------------------|-------|--------------------------|
| `bms_max_charge_voltage`   | V     | Tension max charge       |
| `bms_max_charge_current`   | A     | Courant max charge       |
| `bms_max_discharge_current`| A     | Courant max décharge     |
| `bms_max_charge_cell_voltage`| V   | Tension max par cellule  |
| `bms_charge_request`       | —     | Valeur de demande charge |

#### Chauffage

| Métrique            | Unité | Description           |
|---------------------|-------|-----------------------|
| `bms_heating_current`| A    | Courant chauffage     |
| `bms_heating_power`  | W    | Puissance chauffage   |

#### Modules (multi-BMS)

| Métrique                       | Description                  |
|--------------------------------|------------------------------|
| `bms_modules_online`           | Modules en ligne             |
| `bms_modules_offline`          | Modules hors ligne           |
| `bms_modules_blocking_charge`  | Modules bloquant la charge   |
| `bms_modules_blocking_discharge`| Modules bloquant la décharge|

#### Historique extrêmes

| Métrique            | Unité |
|---------------------|-------|
| `bms_min_voltage_hist`| V   |
| `bms_max_voltage_hist`| V   |

#### Alarmes (0/1)

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

### 2. ET112 — Compteurs énergie AC

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

### 3. Venus / Victron — MPPT Chargeurs solaires

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

### 4. Venus / Victron — SmartShunt (moniteur batterie)

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

### 5. Venus / Victron — Onduleur/Chargeur

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

### 6. Venus / Victron — Températures

**Labels :** `instance`, `device_type="temperature"`

| Métrique                | Unité | Description           |
|-------------------------|-------|-----------------------|
| `venus_temp_c`          | °C    | Température           |
| `venus_humidity_percent`| %     | Humidité relative     |
| `venus_pressure_mbar`   | mbar  | Pression barométrique |
| `venus_connected`       | 0/1   | Connectivité capteur  |

---

### 7. Venus / Victron — Heatpumps (ET112 via D-Bus)

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

### 8. Solaire — Puissance & Rendement globaux

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

### 9. Irradiance — PRALRAN (addr. 0x05)

**Labels :** aucun (singleton)

| Métrique        | Unité | Description      |
|-----------------|-------|------------------|
| `irradiance_wm2`| W/m²  | Irradiance solaire|

---

### 10. ATS CHINT — Commutateur de source

**Labels :** aucun (singleton)

#### Tensions par phase et source

| Métrique                            | Unité | Description          |
|-------------------------------------|-------|----------------------|
| `ats_v1a`, `ats_v1b`, `ats_v1c`    | V     | Source 1 (onduleur) phases A/B/C |
| `ats_v2a`, `ats_v2b`, `ats_v2c`    | V     | Source 2 (réseau) phases A/B/C   |
| `ats_voltage_v`                      | V     | Tension source active (moyenne)  |

#### Fréquences

| Métrique       | Unité |
|----------------|-------|
| `ats_freq1_hz` | Hz    |
| `ats_freq2_hz` | Hz    |
| `ats_freq_hz`  | Hz    |

#### État & Compteurs

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

#### Statut de phase (0=Normal, 1=SousTension, 2=SurTension, 3=Erreur)

`ats_phase_s1a`, `ats_phase_s1b`, `ats_phase_s1c`,
`ats_phase_s2a`, `ats_phase_s2b`, `ats_phase_s2c`

#### Maxima historiques

| Métrique    | Unité |
|-------------|-------|
| `ats_max1_v`| V     |
| `ats_max2_v`| V     |

---

### 11. Tasmota — Prises intelligentes

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

### 12. Shelly EM — Compteurs énergie WiFi

**Label :** `id` (identifiant Shelly)

#### Niveau appareil

| Métrique       | Unité | Description        |
|----------------|-------|--------------------|
| `shelly_power_w`| W    | Puissance totale   |
| `shelly_voltage_v`| V  | Tension            |
| `shelly_rssi`  | dBm   | Signal WiFi        |

#### Niveau canal (`id` + `channel`)

| Métrique                | Unité | Description             |
|-------------------------|-------|-------------------------|
| `shelly_channel_power_w`| W     | Puissance canal         |
| `shelly_current_a`      | A     | Courant canal           |
| `shelly_output`         | 0/1   | Sortie canal            |
| `shelly_energy_wh`      | Wh    | Énergie (cumul)         |
| `shelly_power_factor`   | —     | Facteur de puissance    |
| `shelly_returned_wh`    | Wh    | Énergie retournée       |

---

### 13. Chauffe-eau LG ThinQ

**Labels :** aucun (singleton)

| Métrique          | Unité | Description           |
|-------------------|-------|-----------------------|
| `wh_current_temp_c`| °C   | Température eau actuelle |
| `wh_target_temp_c` | °C   | Température cible        |
| `wh_mode`          | code | Mode opérationnel        |

---

### 14. Pi5 — Monitoring système (daly-bms-server)

**Labels variés** (voir détails)

#### CPU / Mémoire / Disque

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

#### Température & Réseau

| Métrique          | Unité | Description           |
|-------------------|-------|-----------------------|
| `pi5_cpu_temp_c`  | °C    | Température CPU       |
| `pi5_net_rx_bps`  | bps   | Débit réseau entrant  |
| `pi5_net_tx_bps`  | bps   | Débit réseau sortant  |
| `pi5_uptime_secs` | s     | Uptime système        |
| `pi5_serial_port_ok`| 0/1 | État port RS485       |

#### Services & Processus

| Métrique                     | Label    | Description                |
|------------------------------|----------|----------------------------|
| `pi5_service_active`         | `name`   | Service systemd actif (0/1)|
| `pi5_network_service_active` | `name`   | Service réseau actif (0/1) |
| `pi5_process_cpu_percent`    | `process`| CPU par processus (%)      |
| `pi5_process_mem_mb`         | `process`| Mémoire par processus (MB) |

---

### 15. Energy Manager — Monitoring système

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

### 16. Rule Engine — Métriques règles

**Label :** `rule` (nom de la règle)

| Métrique         | Description                   |
|------------------|-------------------------------|
| `rule_eval_total`| Compteur d'évaluations/règle  |

**Requête :**
```promql
rate(rule_eval_total[5m])
```

---

### Récapitulatif — Dashboards Grafana suggérés

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

### Commandes de diagnostic

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

---

## Référence des métriques et requêtes PromQL (par appareil)

> Requêtes PromQL prêtes à l'emploi par appareil, calculs Ah charge/décharge, taux de cyclage,
> URL d'API et points de vigilance.


> Backend : **redb** (`/mnt/nvme/daly-bms/metrics.redb`), interrogé via le **shim PromQL** de `daly-bms-server` sur le port **8080**.
> URL API : `http://192.168.1.141:8080/api/v1/query?query=<PROMQL>`
> Range   : `http://192.168.1.141:8080/api/v1/query_range?query=<PROMQL>&start=…&end=…&step=…`
> Visualisation : dashboard custom interne **`/dashboard/history`** et **Grafana** (`:3000`, datasource « Daly Metrics (redb) »).

> ℹ️ **Sous-ensemble PromQL supporté** — Le shim redb n'implémente qu'un sous-ensemble audité de PromQL (cf. `crates/metrics-store/src/promql/validate.rs` et `./metriques-redb-architecture.md` §6.5). Toute construction hors liste blanche est rejetée avec `status=error`, `errorType=bad_data`.
>
> **Fonctions à fenêtre** (`f(m[range])`) : `increase`, `rate`, `irate`, `delta`, `deriv`, `predict_linear`, `changes`, `resets`, `avg_over_time`, `sum_over_time`, `min_over_time`, `max_over_time`, `count_over_time`, `last_over_time`, `stddev_over_time`, `stdvar_over_time`, `quantile_over_time`, `absent_over_time`.
> **Fonctions instantanées** : `abs`, `clamp_min`, `clamp_max`, `clamp`, `ceil`, `floor`, `round`, `sqrt`, `exp`, `ln`, `log2`, `log10`, `sgn`, `absent`.
> **Manipulation de labels** : `label_replace`, `label_join`.
> **Agrégateurs** : `sum`, `max`, `min`, `avg`, `count` (avec `by (…)` / `without (…)`), `topk(k, …)`, `bottomk(k, …)`.
> **Opérateurs** : arithmétiques `+ - * /` (vecteur⊗scalaire ou vecteur⊗vecteur **aligné**), comparaisons `== != > < >= <=` (filtre ou `bool`).
>
> **Modifier `offset`** (`m offset 5m`, `m[w] offset 1h`, y compris négatif) : **supporté** — décale l'instant d'évaluation à `t − offset`.
>
> **Non supporté** : `integrate` et les autres fonctions MetricsQL, les **subqueries** `[Xh:Ym]`, le modifier `@`, le vector matching `on()` / `ignoring()` / `group_left` / `group_right`, les set ops `and` / `or` / `unless`, les agrégateurs paramétrés `quantile` / `count_values`.

---

### Calculs de charge / décharge en Ampères-heures (Ah)

Pour convertir une courbe d'intensité (A) en charge totale en Ampères-heures (Ah) sur une période, on **n'utilise PAS** la fonction MetricsQL `integrate` (absente du shim). On approxime l'intégrale par `avg_over_time(...) * durée`, ce qui est exact pour un pas d'échantillonnage régulier.

> ⚠️ **Non supporté par le shim redb** : `integrate(venus_shunt_current_a[6h])`.
> **Alternative supportée** : `avg_over_time(venus_shunt_current_a[6h]) * 6` (courant moyen × nombre d'heures → Ah). Toutes les requêtes ci-dessous reposent sur cette équivalence.

```bash
# Décharge sur les deux dernières heures
curl -s "http://192.168.1.141:8080/api/v1/query" \
  --data-urlencode "query=-avg_over_time(clamp_max(venus_shunt_current_a,0)[2h])*2" \
  | jq -r '.data.result[0].value[1]'
# → Affiche : 11.950000127156335

http://192.168.1.141:8080/api/v1/query?query=-avg_over_time(clamp_max(venus_shunt_current_a,0)[2h])*2

http://192.168.1.141:8080/api/v1/query?query=-avg_over_time(clamp_max(venus_shunt_current_a,0)[10h])*10

# 🔋 Ah chargés (24h)
curl -s "http://192.168.1.141:8080/api/v1/query" \
  --data-urlencode "query=-avg_over_time(clamp_min(venus_shunt_current_a,0)[24h])*24"

# 🔌 Ah déchargés (24h)
curl -s "http://192.168.1.141:8080/api/v1/query" \
  --data-urlencode "query=avg_over_time(clamp_max(venus_shunt_current_a,0)[24h])*24"

# ⚖️ Ah nets (24h)
curl -s "http://192.168.1.141:8080/api/v1/query" \
  --data-urlencode "query=avg_over_time(venus_shunt_current_a[24h])*24"


# Charge (toujours positif)
abs(avg_over_time(clamp_min(venus_shunt_current_a, 0)[24h]) * 24)

# Décharge (toujours positif)
abs(avg_over_time(clamp_max(venus_shunt_current_a, 0)[24h]) * 24)
```

🧮 Synthèse de la requête

---

#### 📐 Formule du taux de cyclage
```
Taux de cyclage (%) = (Ah chargés + Ah déchargés) / Capacité batterie × 100
```
> ⚠️ On additionne les valeurs absolues : une batterie qui charge 50 Ah puis décharge 50 Ah a échangé **100 Ah**, soit un cyclage de 50% sur une batterie de 200 Ah.

---

#### ✅ Requête PromQL pour votre configuration

##### 🔹 Avec capacité en dur (ex: 200 Ah)
```promql
(
  avg_over_time(clamp_min(venus_shunt_current_a, 0)[24h]) 
  - avg_over_time(clamp_max(venus_shunt_current_a, 0)[24h])
) * 24 / 200 * 100
```

| Élément | Rôle |
|---------|------|
| `clamp_min(..., 0)` | Garde les courants ≥ 0 → **charge** (positif dans votre cas) |
| `clamp_max(..., 0)` | Garde les courants ≤ 0 → **décharge** (négatif dans votre cas) |
| `- avg_over_time(clamp_max...)` | Soustraire une moyenne négative = ajouter sa valeur absolue |
| `* 24` | Conversion A moyen → Ah sur 24h |
| `/ 200 * 100` | Normalisation par capacité et conversion en % |

> 💡 **Métrique dérivée recommandée** : le serveur émet `venus_shunt_current_abs = |I|` (cf. `write_venus_smartshunt`). Le numérateur ci-dessus se simplifie alors exactement en `avg_over_time(venus_shunt_current_abs[24h])`, plus précis et sans subquery :
> ```promql
> avg_over_time(venus_shunt_current_abs[24h]) * 24 / 200 * 100
> ```

##### 🔹 Avec capacité dynamique (via un metric)
Si votre exporter Victron expose la capacité nominale (ex: `venus_battery_capacity_ah`), utilisez :
```promql
(
  avg_over_time(clamp_min(venus_shunt_current_a, 0)[24h]) 
  - avg_over_time(clamp_max(venus_shunt_current_a, 0)[24h])
) * 24 / venus_battery_capacity_ah * 100
```
> ✅ Avantage : la requête s'adapte automatiquement si vous changez de batterie.
> ⚠️ Note : la division vecteur⊗vecteur exige des séries **alignées** sur le même `step`. Le shim ne supporte que l'alignement exact tous-labels (pas de `on()` / `ignoring()`).

---

#### 🌐 URL API prête à l'emploi
```
http://192.168.1.141:8080/api/v1/query?query=(avg_over_time(clamp_min(venus_shunt_current_a,0)[24h])-avg_over_time(clamp_max(venus_shunt_current_a,0)[24h]))*24/200*100
```

##### 🧪 Test rapide avec `curl`
```bash
# Taux de cyclage sur 24h (capacité 200 Ah)
curl -s "http://192.168.1.141:8080/api/v1/query" \
  --data-urlencode "query=(avg_over_time(clamp_min(venus_shunt_current_a,0)[24h])-avg_over_time(clamp_max(venus_shunt_current_a,0)[24h]))*24/200*100" \
  | jq -r '.data.result[0].value[1] + " %"'
```
→ Résultat attendu : `XX.XX %`

---

#### 📊 Interprétation des résultats

| Taux de cyclage | Interprétation | Impact batterie |
|----------------|----------------|-----------------|
| **0–20 %** | Usage léger | ✅ Longévité maximale |
| **20–50 %** | Usage modéré | ✅ Normal pour usage quotidien |
| **50–80 %** | Usage intensif | ⚠️ Surveiller la température et la tension |
| **> 80 %** | Cyclage profond | 🔋 Privilégier batteries LiFePO4 ; éviter sur plomb |

> 💡 Pour les batteries au plomb, il est recommandé de ne pas dépasser **50 % de profondeur de décharge** (DoD) pour préserver leur durée de vie.

---

#### 🎨 Intégration Grafana (bonus)

##### 1. Panel "Taux de cyclage quotidien"
- **Type** : `Stat` ou `Gauge`
- **Datasource** : `Daly Metrics (redb)` (UID `daly-metrics`)
- **Requête** :
  ```promql
  (avg_over_time(clamp_min(venus_shunt_current_a,0)[24h]) - avg_over_time(clamp_max(venus_shunt_current_a,0)[24h])) * 24 / 200 * 100
  ```
- **Unit** : `Percent (0–100)`
- **Min/Max** : `0` / `150` (pour visualiser les sur-cyclages)
- **Thresholds** :
  - `50` → orange
  - `80` → rouge

##### 2. Variable pour la capacité (optionnel)
Dans *Dashboard Settings → Variables* :
```
Name: battery_capacity
Type: Custom
Values: 100,200,300,400
```
Puis dans la requête :
```promql
... * 24 / $battery_capacity * 100
```

---

#### ⚠️ Points de vigilance

1. **Fenêtre glissante vs jour calendaire**  
   `[24h]` calcule sur les dernières 24h glissantes. Le modifier `offset` est supporté (`m[24h] offset 24h` = la veille glissante). Pour un jour **calendaire** exact (minuit → maintenant), `offset` à durée fixe ne suffit pas : ajustez les bornes `start`/`end` de `query_range` côté appelant.

2. **Précision**  
   L'approximation par `avg_over_time` est fiable si votre intervalle d'écriture est ≤ 30s (cf. throttles plus bas).

   > ⚠️ **Non supporté par le shim redb** : la sous-requête `[24h:10s]` (échantillonnage interne).
   > **Alternative** : le shim agrège **tous les points bruts** de la fenêtre (tier raw ≤ 7 j), donc `avg_over_time(m[24h])` est déjà exact sans sous-échantillonnage. Pour un proxy plus précis, utiliser la métrique dérivée `venus_shunt_current_abs`.

3. **Données manquantes**  
   Vérifiez la continuité des données :
   ```promql
   count_over_time(venus_shunt_current_a[24h])
   ```
   Si le résultat est bien inférieur au nombre de points attendu (`24*3600 / intervalle_écriture`), des données manquent et le calcul sera sous-estimé.

4. **Capacité réelle vs nominale**  
   La capacité d'une batterie diminue avec l'âge. Pour un calcul plus réaliste, vous pouvez créer un metric `battery_effective_capacity_ah` mis à jour manuellement ou via un test de décharge.

---

### Récapitulatif des métriques par appareil

| Appareil | Métriques | Séries |
|----------|-----------|--------|
| BMS ×2 | bms_voltage, bms_current, bms_power, bms_soc, bms_capacity_ah, bms_cell_delta_mv, bms_temp_max, bms_temp_min, bms_charge_mos, bms_discharge_mos, bms_cell_voltage×16 | ~54 |
| ET112 ×3 | et112_voltage_v, et112_current_a, et112_power_w, et112_apparent_power_va, et112_power_factor, et112_frequency_hz, et112_energy_import_wh, et112_energy_export_wh | 24 |
| Irradiance | irradiance_wm2 | 1 |
| SmartShunt | venus_shunt_voltage_v, venus_shunt_current_a, venus_shunt_current_abs, venus_shunt_power_w, venus_shunt_soc_percent, venus_shunt_energy_in_kwh, venus_shunt_energy_out_kwh, venus_shunt_ah_charged_today, venus_shunt_ah_discharged_today | 9 |
| Solar agrégé | solar_total_w, mppt_power_w, solar_yield_kwh | 3 |
| Inverter (EasySolar II GX) | venus_inverter_voltage_v, venus_inverter_current_a, venus_inverter_power_w, venus_inverter_ac_output_voltage_v, venus_inverter_ac_output_current_a, venus_inverter_ac_output_power_w, venus_inverter_ac_freq_hz, venus_inverter_ac_in_ignore | 8 |
| MPPT ×2 | venus_mppt_power_w, venus_mppt_pv_voltage_v, venus_mppt_dc_current_a, venus_mppt_yield_today_kwh, venus_mppt_max_power_today_w | 10 |
| Température/Humidité | venus_temp_c, venus_humidity_percent | 2 |
| Heatpump ×2 (PAC/chauffe-eau) | venus_heatpump_state, venus_heatpump_power_w, venus_heatpump_energy_kwh, venus_heatpump_temp_c, venus_heatpump_target_temp_c | 10 |
| ATS CHINT | ats_sw1_closed, ats_sw2_closed, ats_active_source, ats_voltage_v×6, ats_freq_hz×2 | 11 |
| Tasmota ×6 | tasmota_power_on, tasmota_power_w, tasmota_voltage_v, tasmota_current_a, tasmota_energy_today_kwh | 30 |
| Shelly Pro 2PM | shelly_output×2, shelly_power_w×2, shelly_voltage_v×2, shelly_current_a×2, shelly_energy_wh×2 | 10 |
| **Total** | | **~172** |

---

### Labels utilisés

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

### 1. BMS Daly (2 × 16 cellules)

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

### 2. ET112 Carlo Gavazzi (3 compteurs)

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

### 3. Capteur Irradiance PRALRAN (RS485)

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

### 4. SmartShunt Victron

```promql
# Courant batterie instantané (A — négatif = décharge)
venus_shunt_current_a

# Valeur absolue du courant batterie (|I|, métrique dérivée serveur)
venus_shunt_current_abs

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

### 5. Solar / MPPT agrégé

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

### 6. MPPT Victron SmartSolar (2 chargeurs)

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

### 7. Inverter / EasySolar II GX

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

### 8. Capteur Température / Humidité

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

### 9. Heatpump / PAC (chauffe-eau ET112)

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

### 10. ATS CHINT NXZB

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

### 11. Tasmota Tongou (6 switchs)

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

### 12. Shelly Pro 2PM (DEYE)

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

### Requêtes d'analyse globale

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


# Vérification : nombre de métriques présentes (valeurs distinctes de __name__)
# curl http://192.168.1.141:8080/api/v1/label/__name__/values | jq '.data | length'
# (NB : /api/v1/labels renvoie les NOMS de labels — bms_id, address… — pas les métriques)
# ou via le dashboard custom /dashboard/history (sélecteur de série)

# Dernier point de chaque métrique (vérification fraîcheur)
{__name__=~"bms_soc|venus_shunt_soc_percent|solar_total_w|ats_active_source|tasmota_power_on"}
```

---

### Throttles d'écriture redb

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

---

> _Note historique : avant la migration de mai 2026, ces requêtes étaient servies par VictoriaMetrics (`:8428`, MetricsQL). Elles passent désormais par le shim PromQL de `daly-bms-server` (`:8080`) interrogeant redb._

---

## Sous-ensemble PromQL supporté — roadmap d'implémentation

> Phases d'implémentation du transpileur PromQL (sécurisation, groupement by/without,
> comparaisons, topk/bottomk, irate, fonctions math, opérateurs ensemblistes, matching vectoriel…).


> **✅ ÉTAT : phases 1 → 7 implémentées et testées.** Le sous-ensemble PromQL
> supporté inclut désormais le groupement `by`/`without`, les comparaisons
> (+ `bool`), `topk`/`bottomk`, `irate`, les fonctions math
> (`sqrt exp ln log2 log10 sgn clamp`), la manipulation de labels
> (`label_replace`, `label_join`), `deriv`/`predict_linear`/`*_over_time`,
> `absent`/`changes`/`resets`, le **modifier `offset`** (instant + range,
> négatif inclus), **et — Phase 7 (conformité avancée) —** les opérateurs
> ensemblistes `and`/`or`/`unless`, le matching vectoriel
> `on`/`ignoring`/`group_left`/`group_right`, le modifier `@`
> (`<ts>`/`start()`/`end()`), les agrégateurs `quantile`/`group`/`count_values`/
> `stddev`/`stdvar`, les **sous-requêtes** `expr[range:step]`, et l'arithmétique
> `%`/`^`. La correction P0 (`rate`/`increase` → *no data* sur 1 seul point) est
> intégrée. Voir `./metriques-promql-reference.md` pour l'audit détaillé et
> les propositions de dashboards exploitant ces capacités.
>
> **Restent volontairement hors scope** : `histogram_quantile`, `holt_winters`,
> trigonométrie (`sin/cos/…`), `sort`/`sort_desc`, `scalar`/`vector`, fonctions
> date/heure. Le document ci-dessous reste comme référence de conception.

> **But du document** : plan d'implémentation **autoportant** pour élargir la
> compatibilité PromQL du moteur `metrics-store` (le shim que Grafana
> interroge directement). Conçu pour être **repris dans une conversation
> neuve** : tout le contexte, les fichiers, les formes exactes de l'AST, les
> esquisses de code et les tests y sont.
>
> Branche de travail : 
> max, `main` + 1 active). Workflow par phase : commit → push → PR vers `main`
> → après merge `make sync && bash scripts/deploy-pi5.sh`.

---

### 0. Contexte (architecture réelle)

Grafana (:3000) et le dashboard interne SSR interrogent **directement**
`daly-bms-server` (:8080), qui implémente une **API compatible Prometheus**
adossée à `metrics-store` (redb). **Il n'y a NI Prometheus NI scraping** (migré
depuis VictoriaMetrics, Phase 5). redb est **mono-processus** ⇒ un plugin
Grafana→redb direct est impossible ; le shim PromQL est la bonne approche
(décision validée).

```
RS485 (BMS Daly + ET112 + irradiance) ─┐ (lecture directe)
Victron → NanoPi (dbus-mqtt-venus) ─────┼─► daly-bms-server (Rust) ─► metrics-store / redb
energy-manager ─────────────────────────┘ (MQTT)   │   (tiering raw→hourly→daily)
                                                    └─► API PromQL-compat :8080 ◄── Grafana (PromQL)
```

#### Fichiers clés
| Rôle | Chemin |
|------|--------|
| Validation (liste blanche AST) | `crates/metrics-store/src/promql/validate.rs` |
| Évaluateur | `crates/metrics-store/src/promql/exec.rs` |
| Erreurs (format Prometheus) | `crates/metrics-store/src/promql/error.rs` |
| Reader redb (helpers de scan) | `crates/metrics-store/src/reader.rs` |
| Tests golden (dashboards) | `crates/metrics-store/tests/golden_promql.rs` |
| Endpoints HTTP consommateurs | `crates/daly-bms-server/src/api/redb.rs`, `state.rs` |

#### Sous-ensemble supporté aujourd'hui (état de départ)
- Sélecteurs : `=` `!=` `=~` `!~` (pas `offset`, pas `@`).
- Binaires arith : `+ - * /` (vec/scalar ; vec/vec **aligné exact tous-labels**
  via `align_and_op`, drop `__name__`).
- Agrégateurs : `sum max min avg count` **sans `by`/`without`** (collapse total,
  labels vides).
- Fonctions fenêtre : `rate increase delta {avg,sum,min,max,count,last}_over_time`
  (increase/rate gèrent les resets de compteur via `counter_increase`,
  `raw_counter_increase`, `buckets_counter_increase`).
- Fonctions instant : `abs ceil floor round clamp_min clamp_max`.
- **Non supporté** : subqueries `[r:s]`, `offset`, `@`, comparaisons, `bool`,
  set ops (`and/or/unless`), `by/without`, `on/ignoring/group_left/right`,
  `topk/bottomk/quantile/stddev/group/count_values`, `histogram_quantile`,
  `label_replace/label_join/vector/scalar`, `irate/idelta/deriv/predict_linear`,
  `%` `^`, fonctions math/date.

#### ⚠️ Risque actuel (motive la Phase 1)
`validate_aggregate` **ne rejette pas** le modifier `by/without`, et
`eval_aggregate` l'**ignore** → `sum by (bms_id)(m)` renvoie une valeur
**collapsée fausse, sans erreur**. Idem `on()/ignoring()/group_left` dans les
binaires (alignement exact tous-labels au lieu du matching demandé). Nos 16
dashboards n'utilisent **aucune** de ces constructions (vérifié : 9 agrégations,
toutes nues) ⇒ **aucune régression** à durcir ce comportement.

#### AST `promql-parser` 0.9 — formes EXACTES (vérifiées dans la crate)
```rust
// --- Agrégation ---
pub struct AggregateExpr {
    pub op: TokenType,                  // op.to_string() => "sum","max","topk",...
    pub expr: Box<Expr>,
    pub param: Option<Box<Expr>>,       // k de topk/bottomk/quantile
    pub modifier: Option<LabelModifier>,
    pub mod_span: ...,
}
pub enum LabelModifier { Include(Labels), Exclude(Labels) } // Include=by, Exclude=without
impl LabelModifier { pub fn labels(&self) -> &Labels; pub fn is_include(&self) -> bool; }

// --- Liste de labels de groupement ---
// NB : c'est le type `promql_parser::label::Labels`, PAS le BTreeMap de exec.rs.
pub struct Labels { pub labels: Vec<Label> }   // Label = String (nom de label)

// --- Binaire ---
pub struct BinaryExpr {
    pub op: TokenType, pub lhs: Box<Expr>, pub rhs: Box<Expr>,
    pub modifier: Option<BinModifier>,
}
pub struct BinModifier {
    pub card: VectorMatchCardinality,
    pub matching: Option<LabelModifier>,   // on(...) / ignoring(...)
    pub return_bool: bool,
}
pub enum VectorMatchCardinality { OneToOne, ManyToOne(Labels), OneToMany(Labels), ManyToMany }
```
> Dans `exec.rs`, le type de labels d'un échantillon est l'alias
> `type Labels = BTreeMap<String,String>` — **à ne pas confondre** avec
> `promql_parser::label::Labels` (la liste de noms du modifier). Désambiguïser
> les imports.

#### Règles transverses (toutes phases)
1. **Aucune régression** : `provisioned_grafana_dashboards_coverage` (211 expr,
   16 dashboards) + tous les tests existants restent verts.
2. 1 phase = **1 PR** dédiée : tests unitaires + golden + `cargo build -p
   daly-bms-server` (API publique intacte) + `cargo clippy -p metrics-store`.
3. Sémantique : agrégation (sauf topk/bottomk) et comparaisons **droppent
   `__name__`** ; topk/bottomk **conservent** tous les labels d'origine.
4. Déploiement : changement embarqué dans le binaire ⇒ `make sync && bash
   scripts/deploy-pi5.sh` (pas de migration redb, pas de changement de config).

---

### Phase 1 — Sécurisation (rejeter ce qui est silencieusement ignoré)

**Objectif** : transformer les résultats faux silencieux en **erreurs claires**
(`bad_data`). Petit, sans risque, haute valeur. **À FAIRE EN PREMIER.**

#### `validate.rs`
1. `validate_aggregate` — rejeter le modifier tant que Phase 2 n'est pas faite :
```rust
fn validate_aggregate(a: &AggregateExpr) -> Result<(), PromQlError> {
    let op_str = a.op.to_string();
    if !SUPPORTED_AGGREGATORS.contains(&op_str.as_str()) {
        return unsupported(&format!("aggregator: {op_str}"));
    }
    if a.param.is_some() {
        return unsupported(&format!("parameterized aggregator: {op_str}"));
    }
    if a.modifier.is_some() {                      // Phase 1
        return unsupported("aggregation grouping (by/without) — non encore supporté");
    }
    validate(&a.expr)
}
```
2. `validate_binary` — rejeter le matching de vecteurs non trivial :
```rust
use promql_parser::parser::VectorMatchCardinality;
// ... dans validate_binary, après le check return_bool existant :
if let Some(m) = &b.modifier {
    if m.return_bool { return unsupported("bool modifier"); }
    if m.matching.is_some() || !matches!(m.card, VectorMatchCardinality::OneToOne) {
        return unsupported("vector matching (on/ignoring/group_left/group_right) — non supporté");
    }
}
```

#### Tests (`validate.rs` #[cfg(test)])
```rust
ko("sum by (bms_id)(bms_voltage)", "grouping");
ko("sum without (x)(m)",           "grouping");
ko("a / on(x) b",                  "vector matching");
ko("a * on(x) group_left b",       "vector matching");
```
Golden inchangé (nos dashboards n'utilisent pas ces formes).

**Effort** ~1–2 h. **PR** : `feat(promql): rejette by/without et vector matching (anti-résultats faux)`.

---

### Phase 2 — Groupement `by` / `without`

**Objectif** : **le plus gros gain** de compatibilité Grafana réelle.

#### Sémantique
- `op by (l1,…)(vec)` → grouper par les **valeurs** de `l1,…` ; 1 sample/groupe ;
  labels de sortie = `{l1:…,…}` (drop le reste + `__name__`).
- `op without (l1,…)(vec)` → grouper par **tous les labels sauf** `l1,…` **et**
  `__name__` ; sortie conserve ces labels.
- Sans modifier → comportement actuel (1 seul groupe, labels vides).
- `avg`=sum/cnt par groupe ; `count`=cnt du groupe.

#### `exec.rs::eval_aggregate` (remplacer le corps)
```rust
fn eval_aggregate(&self, a: &AggregateExpr, t: i64) -> Result<Value, PromQlError> {
    let inner = match self.eval_at(&a.expr, t)? {
        Value::Vector(v) => v,
        Value::Scalar(s) => return Ok(Value::Scalar(s)),
    };
    if inner.is_empty() { return Ok(Value::Vector(vec![])); }
    let op = a.op.to_string();

    // Noms de labels du modifier (promql_parser::label::Labels { labels: Vec<String> })
    let grp_names: Vec<String> = match &a.modifier {
        Some(m) => m.labels().labels.iter().map(|l| l.to_string()).collect(),
        None => Vec::new(),
    };
    let is_by = matches!(&a.modifier, Some(LabelModifier::Include(_)));

    // Clé de groupe (exec::Labels = BTreeMap<String,String>)
    let group_key = |labels: &Labels| -> Labels {
        let mut g = Labels::new();
        match &a.modifier {
            None => {}                                            // collapse total
            Some(LabelModifier::Include(_)) => {                  // by (...)
                for k in &grp_names {
                    if let Some(v) = labels.get(k) { g.insert(k.clone(), v.clone()); }
                }
            }
            Some(LabelModifier::Exclude(_)) => {                  // without (...)
                for (k, v) in labels.iter() {
                    if k == "__name__" || grp_names.contains(k) { continue; }
                    g.insert(k.clone(), v.clone());
                }
            }
        }
        let _ = is_by; // (gardé si besoin de distinguer plus tard)
        g
    };

    use std::collections::BTreeMap;
    struct Acc { sum: f64, min: f64, max: f64, cnt: u64 }
    let mut groups: BTreeMap<Labels, Acc> = BTreeMap::new();
    for s in &inner {
        let e = groups.entry(group_key(&s.labels))
            .or_insert(Acc{sum:0.0,min:f64::INFINITY,max:f64::NEG_INFINITY,cnt:0});
        e.sum += s.value; e.min = e.min.min(s.value); e.max = e.max.max(s.value); e.cnt += 1;
    }
    let mut out = Vec::with_capacity(groups.len());
    for (labels, acc) in groups {
        let value = match op.as_str() {
            "sum" => acc.sum, "min" => acc.min, "max" => acc.max,
            "avg" => acc.sum / acc.cnt as f64, "count" => acc.cnt as f64,
            other => return Err(PromQlError::Unsupported(format!("aggregator {other}"))),
        };
        out.push(InstantSample { labels: Arc::new(labels), value });
    }
    Ok(Value::Vector(out))
}
```

#### `validate.rs`
Retirer le rejet Phase 1 du **modifier d'agrégation** (garder celui du matching
binaire — non implémenté).

#### Tests (intégration, lib.rs — voir motifs existants `promql_smoke_*`)
Écrire `bms_v{bms_id="1"}=1`, `bms_v{bms_id="2"}=2`, `bms_v{bms_id="1",phase="a"}=3` :
```rust
// sum by (bms_id)        → 2 séries : {bms_id=1}=4, {bms_id=2}=2
// avg by (bms_id)        → {bms_id=1}=2, {bms_id=2}=2
// count by (bms_id)      → {bms_id=1}=2, {bms_id=2}=1
// max by (bms_id)        → {bms_id=1}=3, {bms_id=2}=2
// sum without (phase)    → {bms_id=1}=4, {bms_id=2}=2  (+ __name__ retiré)
// sum (sans modifier)    → 6 (inchangé)
// vérifier l'ABSENCE de "__name__" dans les labels de sortie
```
Golden reste vert.

**Effort** ~½ j. **PR** : `feat(promql): groupement by/without des agrégations`.

---

### Phase 3 — Extensions à la demande (3 sous-lots indépendants, 1 PR chacun)

À implémenter seulement si un dashboard en a besoin. Indépendants entre eux.

#### 3a — Comparaisons `== != > < >= <=` (+ `bool`)
**Sémantique** :
- `vec OP scalar` **sans `bool`** → **filtre** : ne garder que les samples où la
  condition est vraie, **valeur inchangée**.
- avec `bool` → garder tous les samples, valeur = `1.0`/`0.0`.
- `scalar OP scalar` → PromQL **exige `bool`** (sinon le parser refuse).
- `vec OP vec` aligné → filtre/bool par paire alignée (étendre `align_and_op`).

**Changements** :
- `validate.rs` : nouveau set `SUPPORTED_CMP_OPS = ["==","!=",">","<",">=","<="]`,
  accepté dans `validate_binary` ; ne plus rejeter `bool` quand l'op est une
  comparaison ; `bool` reste rejeté sur les binaires arithmétiques.
- `exec.rs::eval_binary` : si l'op est une comparaison, router vers une logique
  dédiée (filtre vs bool) au lieu de `scalar_fn`. Gérer les 4 combinaisons
  (scalar/scalar requiert bool, vec/scalar, scalar/vec, vec/vec). Drop `__name__`.
- Attention NaN (toute comparaison avec NaN est fausse).

**Tests** : `bms_v > 1.5` (filtre), `bms_v > bool 1.5` (0/1 — ⚠️ le mot-clé
`bool` se place **après l'opérateur**, pas après le rhs), `bms_v == 2`,
vec/vec.

**Effort** ~½–1 j. **PR** : `feat(promql): opérateurs de comparaison + bool`.

#### 3b — `topk` / `bottomk`
**Sémantique** : `topk(k, vec)` = les `k` samples de plus grande valeur,
**labels d'origine CONSERVÉS** (y compris `__name__`). `param` = `k` (scalaire,
évalué via `eval_at`). Optionnel : `topk(k, vec) by (l)` = top-k **par groupe**.

**Changements** :
- `validate.rs::validate_aggregate` : autoriser `param.is_some()` **uniquement**
  pour `topk`/`bottomk` ; les ajouter à un set autorisé.
- `exec.rs::eval_aggregate` : cas spécial avant le regroupement standard :
  évaluer `param` → `k` (`as usize`), trier les samples desc (topk) / asc
  (bottomk), tronquer à `k` (par groupe si modifier). **Ne pas dropper** les
  labels.

**Tests** : `topk(1, bms_v)` → 1 sample (max, labels conservés) ;
`bottomk(2, bms_v)`.

**Effort** ~½ j. **PR** : `feat(promql): topk/bottomk`.

#### 3c — `irate`
**Sémantique** : taux instantané sur les **deux derniers points** de la fenêtre :
`(v_last − v_prev) / ((t_last − t_prev)/1000)`, avec gestion de reset
(`counter_increase(v_prev, v_last)` existant). Diffère de `rate` (toute la
fenêtre).

**Changements** :
- `validate.rs` : ajouter `irate` à `SUPPORTED_RANGE_FUNCS`.
- `reader.rs` : helper `last_two_in_range_with_tx(rtx, sid, from, to) ->
  Result<Option<((i64,f64),(i64,f64))>>` (scan inverse, prend les 2 derniers ;
  garde `from > to ⇒ None` comme les autres helpers).
- `exec.rs::eval_range_call` : router `irate` (tier raw) vers ce helper ; appli :
  `counter_increase(prev.1, last.1) / ((last.0 - prev.0) as f64 / 1000.0)`
  (None si < 2 points ou Δt=0).
- Tier compacté : `irate` mal défini → utiliser les 2 derniers buckets
  (`prev.last → last.last`) **ou** rejeter au-delà du tier raw (documenter le
  choix retenu).

**Tests** : série croissante → irate = pente locale ; reset géré ; 1 seul
point → pas de valeur.

**Effort** ~½ j. **PR** : `feat(promql): irate`.

---

### Phase 4 — Fonctions math & manipulation de labels

**Objectif** : préparer les dashboards plus sophistiqués à venir (légendes
dynamiques, échelles log, normalisation).

#### 4a — Math instant
- `validate.rs` : ajouter `sqrt exp ln log2 log10 sgn clamp` à
  `SUPPORTED_INSTANT_FUNCS`.
- `exec.rs` : helper `unary_math` (propagation NaN naturelle ; `sgn` renvoie
  -1/0/1/NaN à la Prometheus) + `clamp_val` (NaN si `min > max`). Branché dans
  `apply_instant_fn` (vecteur) et `apply_instant_scalar` (scalaire).

#### 4b — Labels
- `validate.rs` : `SUPPORTED_LABEL_FUNCS = [label_replace, label_join]` ;
  `validate_call` valide le 1er arg (vecteur) et **autorise les `StringLiteral`**
  pour les arguments suivants (seul endroit où une string est acceptée). NB :
  `promql-parser` type-check déjà la signature → un mauvais type donne une
  `ParseError`.
- `exec.rs` : routage dédié dans `eval_call` (les args string ne sont pas
  évaluables). `label_replace` utilise la crate `regex` (ancrage `^(?:…)$`,
  expansion `$1`/`${name}` via `Captures::expand`) ; valeur vide ⇒ label retiré.
  `label_join` concatène les labels source avec le séparateur.
- Dépendance : `regex = "1"` (déjà présente transitivement via promql-parser).

**Tests** : `sqrt/clamp/sgn`, `label_replace` (match, non-match, label retiré),
`label_join`. **PR** : `feat(promql): math functions + label_replace/label_join`.

---

### Phase 7 — Conformité avancée (✅ fait)

**Objectif** : couvrir **toutes** les requêtes/alertes Grafana avancées d'un ESS
multi-BMS / multi-MPPT identifiées dans `./metriques-promql-reference.md`
(jointures conditionnelles, normalisations, SLO, comparaisons temporelles,
lissage), sans régression. Implémenté en un lot cohérent.

#### 7a — Opérateurs ensemblistes `and` / `or` / `unless`
- `validate.rs` : `SUPPORTED_SET_OPS = ["and","or","unless"]`, acceptés dans
  `validate_binary` ; `bool` reste réservé aux comparaisons.
- `exec.rs::eval_set_op` : appariement par **signature de labels**
  (`matching_sig`, avec `on`/`ignoring` éventuel). `and` = garde le lhs si une
  série rhs partage la signature ; `unless` = l'inverse ; `or` = lhs ∪ (rhs dont
  la signature est absente du lhs). **Labels conservés** (dont `__name__`), pas
  d'arithmétique. Erreur explicite si un opérande est scalaire.

#### 7b — Matching vectoriel `on` / `ignoring` + `group_left` / `group_right`
- `exec.rs::matching_sig` : calcule la signature d'appariement
  (`on` ⇒ ne garder que les labels listés ; `ignoring` ⇒ tous sauf listés +
  `__name__` ; défaut ⇒ tous sauf `__name__`).
- `exec.rs::result_metric` : construit les labels du résultat à la Prometheus
  (`OneToOne` : drop name + keep/del selon on/ignoring ; `group_left`/
  `group_right` : tous les labels du côté « many » + recopie des labels
  `include` depuis le côté « one »).
- `exec.rs::vector_binary_op` : routage générique (closure `combine`) partagé
  par l'arithmétique et les comparaisons vec×vec. Détecte les doublons de
  signature côté « one » **et** côté « many » en `OneToOne` (exige alors
  `group_left`/`group_right`, conforme Prometheus).

#### 7c — Modifier `@` (`<ts>` / `start()` / `end()`)
- `validate.rs` : `@` n'est plus rejeté dans `validate_vector_selector`.
- `exec.rs::resolve_eval_time` : `@` fixe l'instant (`@ <ts>` en **secondes**
  epoch, `@ start()`/`@ end()` = bornes de la requête, mémorisées dans
  `query_start_ms`/`query_end_ms`), puis `offset` décale relativement.
  Appliqué aux sélecteurs instant, aux fonctions à fenêtre et à `absent`.

#### 7d — Agrégateurs `quantile` / `group` / `count_values` / `stddev` / `stdvar`
- `validate.rs` : `SCALAR_PARAM_AGGREGATORS = [topk,bottomk,quantile]`,
  `STRING_PARAM_AGGREGATORS = [count_values]`, et `group`/`stddev`/`stdvar`
  ajoutés à `SUPPORTED_AGGREGATORS`.
- `exec.rs::eval_aggregate` : `quantile(φ, vec)` = percentile **spatial** par
  groupe (interpolation linéaire) ; `count_values("l", vec)` = comptage par
  valeur distincte (ajoute le label `l`) ; `group` = présence binaire (1) ;
  `stddev`/`stdvar` = écart-type/variance de population via **algorithme de
  Welford** (stable numériquement sur compteurs cumulés à grande magnitude).

#### 7e — Sous-requêtes `expr[range:step]`
- `validate.rs` : `Expr::Subquery` valide l'expression interne (plus de rejet).
- `exec.rs::eval_subquery_call` : matérialise une série synthétique en évaluant
  l'expression interne à chaque sous-pas (`step` explicite, sinon 1 min), puis
  applique la fonction fenêtre (`apply_range_fn_raw`). Garde anti-explosion
  (`MAX_SUBQUERY_STEPS = 11 000`) et garde anti-boucle (saturation `i64::MAX`).

#### 7f — Arithmétique `%` (modulo) / `^` (puissance) + correction P0
- `exec.rs::arith_fn` : `%` et `^` ajoutés.
- `exec.rs::apply_range_fn_raw` : `rate`/`increase` retournent ***no data*** (et
  non `0`) lorsqu'un seul point tombe sous la fenêtre — fiabilise les bilans
  énergétiques sur séries clairsemées (P0 §4.6 de l'audit).

**Tests** : `promql_set_ops_and_vector_matching`, `promql_new_aggregators`,
`promql_at_modifier_and_subquery`, `promql_rate_single_point_no_data` (lib.rs) +
couverture `validate.rs` ; golden 16 dashboards toujours vert (77 tests OK).
**PR** : `feat(metrics-store): conformité PromQL avancée` + revue Gemini
(boucle subquery, variance Welford, matching OneToOne).

---

### Récapitulatif

| Phase | Contenu | Effort | Risque | Priorité | État |
|-------|---------|--------|--------|----------|------|
| 1 | Rejet explicite by/without + vector matching | 1–2 h | très faible | **haute** | ✅ fait |
| 2 | Groupement by/without | ½ j | faible | **haute** | ✅ fait |
| 3a | Comparaisons + bool | ½–1 j | moyen | moyenne | ✅ fait |
| 3b | topk/bottomk | ½ j | faible | moyenne | ✅ fait |
| 3c | irate | ½ j | faible | basse | ✅ fait |
| 4a | Math instant (sqrt/exp/ln/log/sgn/clamp) | ¼ j | faible | moyenne | ✅ fait |
| 4b | label_replace / label_join | ½ j | faible | **haute** | ✅ fait |
| 5 (P2) | deriv, predict_linear, quantile/stddev/stdvar_over_time | ½ j | moyen | moyenne | ✅ fait |
| 5 (P3) | absent, absent_over_time, changes, resets + fix `round(v,to_nearest)` | ½ j | faible | moyenne | ✅ fait |
| 6 | modifier `offset` (instant + range, négatif inclus) | ¼ j | faible | moyenne | ✅ fait |
| 7a | set ops `and`/`or`/`unless` | ½ j | moyen | **haute** | ✅ fait |
| 7b | matching `on`/`ignoring`/`group_left`/`group_right` | ½–1 j | moyen | **haute** | ✅ fait |
| 7c | modifier `@` (`<ts>`/`start()`/`end()`) | ¼ j | faible | moyenne | ✅ fait |
| 7d | agrégateurs `quantile`/`group`/`count_values`/`stddev`/`stdvar` | ½ j | faible | moyenne | ✅ fait |
| 7e | sous-requêtes `[range:step]` | ½ j | moyen | moyenne | ✅ fait |
| 7f | arithmétique `%`/`^` + fix P0 `rate`/`increase` 1 point | ¼ j | faible | **haute** | ✅ fait |

**Ordre recommandé** : 1 → 2 → (3a/3b/3c à la demande) → 4 → 5 → 7 (toutes les
constructions PromQL avancées requises par Grafana sont désormais couvertes).

#### Phase 5 — notes d'implémentation
- `eval_call` localise le `MatrixSelector` **n'importe où** dans les args (gère
  `quantile_over_time(φ, m[w])` dont la matrice est le 2ᵉ arg) ; le scalaire
  éventuel (`φ`, durée `predict_linear`) est extrait comme « 1er arg non-matrice ».
- `deriv`/`predict_linear` : régression linéaire moindres carrés
  (`linear_regression`, origine x = `intercept_time`). `predict_linear` ancre
  l'ordonnée à l'instant d'éval et renvoie `slope*T + intercept`.
- `absent` / `absent_over_time` : routés à part (besoin des matchers du
  sélecteur) ; renvoient 1 sample étiqueté des matchers `=` quand la série est
  absente. `absent` reste instant-only (le parser rejette `absent(m[w])`) ;
  `absent_over_time(m[w])` couvre l'absence sur fenêtre. L'argument est
  dé-parenthésé (`Expr::Paren`) avant analyse.
- `linear_regression` : garde anti division par zéro via comparaison des
  timestamps de bornes (`pts[0] == pts[last]`, exact) plutôt que `var_x == 0.0`.
- Tier compacté : approximations documentées (cf. §6.5 du plan de migration).
- **Phase 6 — modifier `offset`** (✅ fait) : retiré du rejet dans
  `validate_vector_selector` (le `@` reste rejeté) ; `exec.rs::offset_ms`
  applique `t_eff = t − offset` au lookback instantané, à la fenêtre des
  fonctions à fenêtre et à `absent_over_time`. Couvre l'offset négatif
  (`offset -5m`, vers le futur). Test sémantique : `promql_offset_modifier`
  (lib.rs). Permet la comparaison de périodes (`m offset 24h`, `m offset 1y`)
  sans contournement client.
#### Phase 7 — notes d'implémentation
- **Deux types `Labels`** toujours valables : `exec::Labels` (BTreeMap) vs la
  liste de noms du modifier ; `matching_sig`/`result_metric` manipulent des
  `exec::Labels`.
- `vector_binary_op` est partagé par arithmétique **et** comparaisons (closure
  `combine` : `Some(v)` émet, `None` filtre). Set ops passent par `eval_set_op`
  (pas d'arithmétique, labels conservés).
- `@ <ts>` est en **secondes** epoch (sémantique Prometheus) → multiplié par
  1000 dans `resolve_eval_time`. `@ start()`/`end()` lisent les bornes
  mémorisées au début de `eval_range`/`eval_instant`.
- `stddev`/`stdvar` utilisent **Welford** (`mean`/`m2`) et non `Σx²/n−(Σx/n)²`.
- Sous-requêtes : `step` par défaut = 1 min ; bornes `MAX_SUBQUERY_STEPS` et
  garde de saturation `i64::MAX`. Réutilise `apply_range_fn_raw` (raw path).

- **Restants (hors scope, à la demande)** : `histogram_quantile`,
  `holt_winters`, trigo (`sin/cos/…`), `sort`/`sort_desc`, `scalar`/`vector`,
  fonctions date/heure, `{__name__=~…}`.

#### Définition de « terminé » par PR
- [ ] `cargo test -p metrics-store` vert (unitaires + golden + coverage 16 dashboards).
- [ ] `cargo build -p daly-bms-server` OK (API publique intacte).
- [ ] `cargo clippy -p metrics-store` sans nouvelle alerte.
- [ ] Matrice de compat mise à jour (ce doc + doc utilisateur).
- [ ] Aucune régression sur les dashboards (golden).

#### Notes de prudence (cruciales en conversation neuve)
- **Deux types `Labels`** : `exec::Labels = BTreeMap<String,String>` (labels d'un
  sample) vs `promql_parser::label::Labels { labels: Vec<String> }` (noms du
  modifier). Accès aux noms : `modifier.labels().labels`.
- Agrégations (sauf topk/bottomk) et comparaisons **droppent `__name__`** ;
  topk/bottomk **conservent** tous les labels.
- `BTreeMap` comme clé de groupe ⇒ sortie déterministe (tests stables).
- Ne pas casser l'optim P2 (instant selector `last_point_in_range`,
  increase/rate `raw_counter_increase`/`buckets_counter_increase` — déjà en place).
- Le test `provisioned_grafana_dashboards_coverage` est le garde-fou anti-régression.
- Réutiliser `counter_increase` (gestion reset) pour `irate`.

---

## Conformité PromQL — audit détaillé

> Matrice de conformité, écarts majeurs (§4.x), approximations documentées, recommandations
> et récapitulatif par dashboard ESS. Les ancres §4.6/§7 sont référencées par le code.

Voici l'audit de conformité PromQL du crate `metrics-store` du dépôt **Daly-BMS-Rust**.

---

### 0. STATUT D'IMPLÉMENTATION (mise à jour)

> Les écarts identifiés ci-dessous ont été **implémentés** dans le moteur
> (`crates/metrics-store/src/promql/{validate,exec}.rs`). Couverture désormais
> assurée pour toutes les requêtes/alertes Grafana avancées listées dans ce
> document :
>
> | Fonctionnalité | Statut | Localisation |
> |---|---|---|
> | `offset` | ✅ implémenté | `exec::offset_ms` |
> | `@ <ts>` / `@ start()` / `@ end()` | ✅ implémenté | `exec::resolve_eval_time` |
> | `and` / `or` / `unless` | ✅ implémenté | `exec::eval_set_op` |
> | `on` / `ignoring` | ✅ implémenté | `exec::matching_sig`, `vector_binary_op` |
> | `group_left` / `group_right` | ✅ implémenté | `exec::result_metric`, `vector_binary_op` |
> | `quantile` (agrégateur) | ✅ implémenté | `exec::eval_aggregate` |
> | `group` / `count_values` / `stddev` / `stdvar` (agrégateurs) | ✅ implémenté | `exec::eval_aggregate` |
> | Sous-requêtes `[range:step]` | ✅ implémenté | `exec::eval_subquery_call` |
> | `%` (modulo) / `^` (puissance) | ✅ implémenté | `exec::arith_fn` |
> | `rate`/`increase` sur 1 point → *no data* (P0 §4.6) | ✅ corrigé | `exec::apply_range_fn_raw` |
> | `round(v, 0)` → défaut `to_nearest=1` | ⚠️ écart documenté (§4.7) — conservé volontairement | `exec::round_to` |
>
> Les limitations résiduelles documentées (§5 : approximations sur tiers
> compactés hourly/daily) restent inchangées : elles relèvent de la stratégie de
> tiering et non d'un manque de fonctionnalité PromQL.

---

### 1. Résumé exécutif

Le crate `metrics-store` implémente un **shim PromQL personnalisé** exécuté sur une base redb (TSDB embarquée). Il repose sur le parser externe `promql-parser` v0.9.0 (dernière version stable, compatible Prometheus v2.45.0), sur lequel est construit un évaluateur maison avec une couche de validation par liste blanche.

**Verdict global :**  
✅ **Conforme pour un sous-ensemble fonctionnel bien délimité** (golden set + extensions).  
⚠️ **Non conforme** sur les opérations avancées de matching vectoriel, les sous-requêtes, les modificateurs temporels et certains agrégateurs.  
⚠️ **Approximations documentées** sur les données tierées (hourly/daily) pour les fonctions statistiques et les compteurs.

---

### 2. Méthodologie

L'audit a porté sur les fichiers suivants :
- `src/promql/mod.rs` — orchestration parse/validate/exec
- `src/promql/exec.rs` — évaluateur (`Evaluator`, `eval_range`, `eval_instant`)
- `src/promql/validate.rs` — liste blanche et rejet des constructions non supportées
- `src/promql/error.rs` — format d'erreur Prometheus
- `src/lib.rs` — tests d'intégration PromQL

Référence de conformité : sémantique Prometheus v2.45+ (PromQL officiel).

---

### 3. Matrice de conformité PromQL

| Catégorie | Fonction/Opérateur | Statut | Notes |
|---|---|---|---|
| **Sélecteurs** | `metric{}` instant | ✅ | Lookback 5 min configurable |
| | `= != =~ !~` | ✅ | Gestion correcte des valeurs vides pour `!=` et `!~` |
| | `offset` | ❌ | Rejeté par validate |
| | `@` | ❌ | Rejeté par validate |
| **Arithmétique** | `+ - * /` (vec×scalar) | ✅ | |
| | `+ - * /` (vec×vec) | ✅ | Alignement exact par labels (hors `__name__`) |
| **Comparaisons** | `== != > < >= <=` | ✅ | Filtre ou `bool` ; NaN toujours faux |
| **Agrégations** | `sum max min avg count` | ✅ | `by`/`without` supportés |
| | `topk` / `bottomk` | ✅ | Labels d'origine conservés (dont `__name__`) |
| | `quantile` | ❌ | Non supporté |
| | `group` | ❌ | Non supporté |
| | `count_values` | ❌ | Non supporté |
| **Set ops** | `and` / `or` / `unless` | ❌ | Rejetées |
| **Matching vectoriel** | `on` / `ignoring` | ❌ | Rejeté |
| | `group_left` / `group_right` | ❌ | Rejeté |
| **Fonctions fenêtre** | `increase` / `rate` | ✅ | Gestion des resets intermédiaires |
| | `irate` | ✅ | Raw uniquement ; approximé sur tier compacté |
| | `delta` | ✅ | |
| | `deriv` / `predict_linear` | ✅ | Régression linéaire moindres carrés |
| | `changes` / `resets` | ✅ | NaN consécutifs ignorés |
| | `avg/sum/min/max/count_over_time` | ✅ | |
| | `last_over_time` | ✅ | |
| | `stddev/stdvar_over_time` | ✅ | Approximé sur tiers compactés (moyennes) |
| | `quantile_over_time` | ✅ | Interpolation linéaire |
| | `absent_over_time` | ✅ | |
| **Fonctions instant** | `abs ceil floor round` | ✅ | `round(v, to_nearest)` honoré |
| | `clamp clamp_min clamp_max` | ✅ | NaN et `min>max` → NaN |
| | `sqrt exp ln log2 log10` | ✅ | |
| | `sgn` | ✅ | `sgn(0)=0`, `sgn(NaN)=NaN` |
| | `absent` | ✅ | |
| **Labels** | `label_replace` | ✅ | Regex ancrée `^(?:…)$` |
| | `label_join` | ✅ | |
| **Sous-requêtes** | `[range:step]` | ❌ | Rejetées |
| **Exposition** | `prom_text` | 🔶 | Hors scope de cet audit |

---

### 4. Écarts majeurs (non-conformités)

#### 4.1 Opérateurs de matching vectoriel absents
Les opérateurs `on(...)`, `ignoring(...)`, `group_left` et `group_right` sont rejetés. L'évaluateur ne supporte que l'alignement **OneToOne** strict sur tous les labels (hors `__name__`).  
**Impact :** Impossible de faire des jointures partielles ou des opérations many-to-one/one-to-many.

#### 4.2 Opérateurs ensemblistes
`and`, `or`, `unless` ne sont pas implémentés.  
**Impact :** Les requêtes de filtrage croisé (ex. `foo and bar`) doivent être décomposées côté client.

#### 4.3 Sous-requêtes (`subquery`)
Rejetées avec message explicite.  
**Impact :** Les requêtes de type `avg_over_time(rate(foo[5m])[1h:1m])` ne passent pas.

#### 4.4 Modificateurs temporels
`offset` et `@` sont rejetés.  
**Impact :** Pas de requêtes historiques point-in-time ni de décalage temporel.

#### 4.5 Agrégateurs manquants
- `quantile(0.9, ...)` (agrégateur instant, différent de `quantile_over_time`)
- `group(...)`
- `count_values("label", ...)`

#### 4.6 `rate` / `increase` avec un seul point
Dans Prometheus, `rate` et `increase` nécessitent **au moins 2 points** sous la fenêtre ; avec un seul point, ils retournent *no data*.  
Dans `exec.rs`, `raw_counter_increase` sur 1 point retourne `0.0`, donc `rate` retourne `0 / range` et `increase` retourne `0`.  
**Impact :** Faux positifs silencieux sur les séries très peu denses.

#### 4.7 `round(v, 0)` — déviation sémantique
Prometheus : `round(v, 0)` → `NaN` (division par zéro).  
Le code : `to_nearest == 0.0` est remplacé par `1.0` (défaut défensif).  
**Impact :** Résultat différent de Prometheus pour ce cas limite.

---

### 5. Approximations et limitations documentées

#### 5.1 Tiering (raw → hourly → daily)
L'évaluateur sélectionne automatiquement le tier selon la durée de la fenêtre :
- ≤ 7 j → raw
- ≤ 90 j → hourly
- > 90 j → daily

**Fonctions approximées sur tiers compactés :**
- `stddev_over_time` / `stdvar_over_time` : calculées sur les **moyennes** des buckets, pas sur les valeurs brutes. La variance des moyennes ≠ variance de population.
- `deriv` / `predict_linear` : régression sur les `avg` des buckets.
- `changes` / `resets` : séquence `first→last` par bucket ; les oscillations intra-bucket sont invisibles.
- `irate` : approximé par les deux derniers buckets.

**Reset de compteur invisible :** un reset à l'intérieur d'un bucket horaire/journalier est perdu (les points raw ont été purgés). Documenté en commentaire.

#### 5.2 `absent()` sur expression complexe
Prometheus exige un `VectorSelector` simple. L'évaluateur accepte n'importe quelle expression valide, mais si ce n'est pas un sélecteur simple, les labels du résultat seront vides (car `vs_opt` est `None`). Comportement plus permissif mais légèrement différent.

---

### 6. Points forts et bonnes pratiques

1. **Validation explicite** : whitelist claire avec messages d'erreur formatés au standard Prometheus (`status=error`, `errorType=bad_data`/`execution`).
2. **Gestion des resets** : `counter_increase` gère correctement les resets intermédiaires (testé `increase` sur `[10, 20, 5, 15]` → `25`).
3. **NaN** : comparaisons avec NaN toujours fausses (conforme PromQL) ; `clamp` propage NaN.
4. **`__name__`** : retrait correct dans les agrégations et comparaisons ; conservation dans `topk`/`bottomk`.
5. **Optimisations mémoire** : transaction redb partagée, catalogue de séries chargé 1×, cache de matching par pointeur (`*const VectorSelector`), `Arc<<Labels>` pour éviter les clones.
6. **Tests exhaustifs** : couverture des cas limites (intervalles inversés, réutilisation d'`Evaluator`, compaction idempotente, fusion de buckets).
7. **`label_replace`** : regex ancrée correctement (`^(?:…)$`) avec expansion `$1`.

---

### 7. Recommandations prioritaires

| Priorité | Recommandation | Fichier concerné |
|---|---|---|
| 🔴 **P0** | Corriger `rate`/`increase` pour retourner `None` (pas `0`) quand il n'y a qu'**1 seul point** sous la fenêtre | `exec.rs` (`apply_range_fn_raw`) |
| 🔴 **P0** | Aligner `round(v, 0)` sur Prometheus (`NaN`) ou documenter explicitement l'écart | `exec.rs` (`round_to`) |
| 🟡 **P1** | Implémenter `and` / `or` / `unless` pour les alertes Grafana courantes | `validate.rs`, `exec.rs` |
| 🟡 **P1** | Implémenter `on` / `ignoring` (matching vectoriel restreint) | `validate.rs`, `exec.rs` |
| 🟡 **P1** | Ajouter `offset` (décalage temporel simple) | `validate.rs`, `exec.rs` |
| 🟢 **P2** | Ajouter l'agrégateur `quantile` (utilisé pour les SLO) | `validate.rs`, `exec.rs` |
| 🟢 **P2** | Documenter dans l'API HTTP les écarts de précision sur les tiers compactés | Documentation utilisateur |
| 🔵 **P3** | Mettre à jour `promql-parser` si une v0.10+ sort avec des correctifs | `Cargo.toml` |

---

### 8. Verdict

Le crate `metrics-store` offre une **implémentation PromQL robuste et bien testée pour un usage embarqué (ESS)**. La couverture du "golden set" est complète et les extensions (math, labels, prédiction, `absent`) sont bien intégrées.

**Conformité estimée : ~70 %** du langage PromQL standard, avec une conformité **~95 %** sur le sous-ensemble déclaré supporté.

Voici des **requêtes Grafana réalistes et sophistiquées** pour un ESS multi-BMS / multi-MPPT qui **échouent actuellement** avec le shim PromQL de `metrics-store`, classées par type de limitation.

---

### 1. Comparaisons temporelles (`offset`, `@`)

#### ❌ Requête : "SOC actuel vs SOC à la même heure hier (comparaison jour J-1)"
**PromQL idéal :**
```promql
bms_soc{bms_id="1"} - bms_soc{bms_id="1"} offset 24h
```
**Pourquoi ça échoue :** `offset` et `@` sont rejetés par le validateur (`validate_vector_selector`).

**Impact ESS :** Impossible de faire des tableaux de bord "tendance 24h", des alertes "dérive anormale par rapport à la veille", ou des graphiques de superposition jour/J-1 dans Grafana.

**Contournement :** Aucun côté PromQL. Il faut exporter deux séries distinctes côté applicatif (ex. `bms_soc` et `bms_soc_yesterday`) ou faire le calcul dans Grafana avec deux requêtes et une transformation — ce qui casse l'alerte PromQL native.

---

### 2. Jointures conditionnelles (`and`, `or`, `unless`)

#### ❌ Requête : "Alerte : BMS en surcharge thermique (SOC < 20% ET température > 45°C)"
**PromQL idéal :**
```promql
bms_soc < 20 and bms_temp_c > 45
```
**Pourquoi ça échoue :** `and` est rejeté comme opérateur binaire non supporté.

**Impact ESS :** Impossible de créer des alertes multi-critères sur le **même équipement** (même `bms_id`). Par exemple : "Déclenchement chauffage si T° < 5°C **et** tension cellule < 2.5V".

**Contournement :** Deux requêtes séparées dans Grafana + transformation `Merge` ou `Math`, mais l'alerte ne peut pas être exprimée en une seule règle PromQL.

---

#### ❌ Requête : "Liste des BMS actifs mais sans communication MPPT (orphans)"
**PromQL idéal :**
```promql
bms_status unless on(bms_id) mppt_status
```
**Pourquoi ça échoue :** `unless` et `on(...)` sont rejetés.

**Impact ESS :** Impossible de détecter des équipements déconnectés logiquement (présents dans la table BMS, absents du bus MPPT).

---

### 3. Matching vectoriel avancé (`on`, `ignoring`, `group_left`, `group_right`)

#### ❌ Requête : "Rendement DC/DC par string PV : Puissance MPPT / Puissance théorique du panneau"
**PromQL idéal :**
```promql
mppt_power_w / on(string_id) pv_panel_theoretical_w
```
**Pourquoi ça échoue :** `on(string_id)` est rejeté. L'évaluateur ne supporte que l'alignement **OneToOne** sur **tous les labels** (hors `__name__`).

**Impact ESS :** Si `mppt_power_w` a les labels `{string_id="A", mppt_id="1"}` et `pv_panel_theoretical_w` a `{string_id="A", model="400W"}`, la division échoue car les labels ne matchent pas exactement (différence de `mppt_id` vs `model`). On ne peut pas dire "divise-les juste sur `string_id`".

**Contournement :** Pré-calculer le rendement côté applicatif et l'exposer comme une nouvelle métrique `mppt_yield_ratio`.

---

#### ❌ Requête : "Puissance par phase, enrichie avec la capacité nominale du BMS (many-to-one)"
**PromQL idéal :**
```promql
bms_power_w * on(bms_id) group_left(capacity_ah) bms_capacity_ah
```
**Pourquoi ça échoue :** `group_left` est rejeté.

**Impact ESS :** Impossible d'attacher des métadonnées statiques (capacité, date de mise en service, type de cellule) à des séries temporelles dynamiques côté requête. C'est pourtant essentiel pour normaliser des indicateurs (ex. "C-rate = courant / capacité").

---

### 4. Agrégateur `quantile` (percentile instantané)

#### ❌ Requête : "95e percentile de la tension cellule sur l'ensemble du parc BMS"
**PromQL idéal :**
```promql
quantile(0.95, bms_cell_voltage_v)
```
**Pourquoi ça échoue :** `quantile` (agrégateur instant, différent de `quantile_over_time`) n'est pas dans `SUPPORTED_AGGREGATORS`.

**Impact ESS :** Impossible de faire des SLO/SLA du type : "95% des cellules doivent rester entre 2.8V et 4.2V". On peut faire `max` ou `min`, mais pas de percentile global.

**Note :** `quantile_over_time(0.95, bms_cell_voltage_v[1h])` **fonctionne** (c'est une fonction range), mais elle calcule le percentile temporel d'une **série unique**, pas le percentile spatial sur l'ensemble des BMS.

---

### 5. Subqueries (`[range:resolution]`)

#### ❌ Requête : "Moyenne mobile sur 1h du taux de charge, évaluée toutes les 5 minutes"
**PromQL idéal :**
```promql
avg_over_time(rate(bms_energy_wh[5m])[1h:5m])
```
**Pourquoi ça échoue :** Les subqueries `[1h:5m]` sont rejetées explicitement.

**Impact ESS :** Très courant pour le suivi de la santé des batteries : on veut lisser le `rate` de décharge sur une fenêtre longue sans sur-échantillonner. Actuellement, il faut choisir entre un `rate` bruité (fenêtre courte) ou un `rate` retardé (fenêtre longue).

---

#### ❌ Requête : "Prédiction du SOC dans 2h basée sur la tendance moyenne des dernières 6h"
**PromQL idéal :**
```promql
predict_linear(bms_soc[1h], 7200) 
# ou, plus sophistiqué :
predict_linear(avg_over_time(bms_soc[10m])[6h:10m], 7200)
```
**Pourquoi ça échoue :** La version simple `predict_linear(bms_soc[1h], 7200)` **fonctionne**, mais la version lissée avec subquery est impossible. Sur un SOC bruité, la prédiction directe sur 1h est instable.

---

### 6. Agrégateur `group`

#### ❌ Requête : "Nombre de BMS actifs (présence binaire, indépendamment de la valeur)"
**PromQL idéal :**
```promql
group(bms_status) 
```
**Pourquoi ça échoue :** `group` n'est pas supporté.

**Impact ESS :** `count(bms_status)` compte les séries, mais si on veut juste vérifier la *présence* d'une série (valeur = 1 peu importe la métrique originale), `group` est le standard PromQL. Utile pour des dashboards "état de la flotte".

---

### 7. Cas limites silencieux (faux positifs)

#### ⚠️ Requête : "Énergie injectée aujourd'hui (Wh) sur un MPPT peu ensoleillé"
**PromQL :**
```promql
increase(mppt_energy_wh[24h])
```
**Piège :** Si le MPPT n'a produit que 2 points dans la journée (ex. matin et soir avec coupure nuageuse), `increase` retourne `0` au lieu de `no data` ou de la vraie différence.

**Pourquoi :** Le bug P0 identifié dans l'audit : `raw_counter_increase` sur 1 seul point retourne `0.0`, donc `increase` retourne `0` et `rate` retourne `0 / range`.

**Impact ESS :** Un MPPT à l'arrêt ou déconnecté apparaît comme "0 Wh produits" (ce qui est vrai) mais un MPPT avec 2 points espacés de 12h apparaît aussi à 0, ce qui est **faux** (la différence entre les 2 points est positive). Cela fausse les bilans énergétiques agrégés.

---

### 8. Récapitulatif par dashboard ESS

| Dashboard / Alert ESS | Requête typique | Statut |
|---|---|---|
| **Bilan énergétique jour** | `sum(increase(bms_energy_wh[24h]))` | ✅ |
| **Bilan vs hier** | `... - ... offset 24h` | ❌ |
| **Alerte surcharge** | `bms_soc < 20 and bms_temp > 45` | ❌ |
| **Rendement par string** | `mppt_power / on(string_id) theoretical` | ❌ |
| **C-rate global** | `sum(bms_current) / on(bms_id) group_left capacity` | ❌ |
| **Santé cellule (SLO)** | `quantile(0.95, bms_cell_v)` | ✅ |
| **Prédiction SOC lissée** | `predict_linear(avg_over_time(...)[6h:10m], ...)` | ✅ |
| **Détection orphans** | `bms_status unless mppt_status` | ✅ |
| **Compteur fiabilisé** | `increase(mppt_energy_wh[24h])` sur série clairsemée | ✅ corrigé |

> **Mise à jour** : toutes les lignes ci-dessus sont désormais **✅** (cf. §0).
> Les statuts `❌` historiques sont conservés dans le corps de l'audit (§3–§7)
> comme trace de l'analyse initiale.

---

### Recommandation immédiate

Pour un ESS en production, je suggère de **prioriser** l'implémentation dans cet ordre :

1. **`and` / `or`** — indispensable pour les alertes multi-critères (sécurité thermique)
2. **`on` / `ignoring`** — nécessaire pour les rendements et normalisations (C-rate)
3. **`offset`** — pour les tendances et comparaisons (optimisation énergétique)
4. **`quantile` (agrégateur)** — pour les SLO de santé batterie
5. **Correction `increase`/`rate` à 1 point** — pour fiabiliser les bilans énergétiques

Les subqueries et `group_left` peuvent attendre si vous pré-calculez les métriques dérivées côté `energy-manager` ou `daly-bms-server`.

Les écarts principaux concernent les opérations de **jointure vectorielle avancée** et les **modificateurs temporels**, qui sont volontairement hors scope pour ce système. Le point le plus critique à corriger est le comportement de `rate`/`increase` sur un seul point, qui peut induire des alertes silencieuses fausses.

---

## 9. Dashboards Grafana évolués — réalisés (exploitation des nouvelles capacités)

> ✅ **Statut : les 4 dashboards ci-dessous sont créés** dans
> `contrib/grafana/dashboards/` (`17-flotte-sante.json`, `18-rendement-pv.json`,
> `19-bilan-energie.json`, `20-alertes-avancees.json`) et **validés** par le test
> golden `provisioned_grafana_dashboards_coverage` (chaque `expr` est acceptée
> par le moteur). Section §9.2 = évolutions optionnelles de dashboards existants
> (catalogue de panneaux à intégrer à la demande).
>
> **Contraintes de provisioning** (cf. CLAUDE.md règle 14) : format provisioning
> (pas d'export), **pas** de `__inputs`/`__requires`, datasource UID =
> `daly-metrics`. Tout nouveau fichier JSON dans `contrib/grafana/dashboards/`
> est validé automatiquement par le test golden
> `provisioned_grafana_dashboards_coverage`.
> Labels réels : `bms_*{bms_id="0x01"|"0x02"}`, `et112_*{address="0x07|0x08|0x09"}`.

### 9.1 Nouveaux dashboards proposés

#### 🆕 `17-flotte-sante.json` — « Santé de flotte & SLO batterie »
Vue consolidée multi-BMS orientée **service-level objectives**, pensée pour le
coup d'œil quotidien et l'alerting.

| Panel | Type | Requête PromQL | Apport (nouveauté) |
|---|---|---|---|
| BMS actifs | stat | `count(group(bms_soc) by (bms_id))` | `group` = présence binaire |
| SOC médian parc (P50) | gauge | `quantile(0.5, bms_soc)` | percentile **spatial** (SLO) |
| C-rate normalisé / pack | timeseries | `abs(bms_current) / on(bms_id) bms_reported_capacity_ah` | `on()` = courant ÷ capacité |
| SoH min parc | stat | `min(bms_soh)` | agrégat (santé) |
| SOC P05 / P50 / P95 du parc | timeseries | `quantile(0.05, bms_soc)`, `quantile(0.5, bms_soc)`, `quantile(0.95, bms_soc)` | percentile **spatial** (SLO) |
| Tension cellule — bande P05↔P95 | timeseries | `quantile(0.95, bms_cell_voltage)` et `quantile(0.05, bms_cell_voltage)` | dispersion parc, alerte 2.8–4.2 V |
| Dispersion cellules / pack | timeseries | `stddev by (bms_id)(bms_cell_voltage)` | `stddev` (santé équilibrage) |
| Distribution SoH | bar gauge | `count_values("soh", round(bms_soh, 0.1))` | histogramme d'état (arrondi 0,1 %) |
| 3 cellules les plus basses | table | `bottomk(3, bms_min_cell_voltage)` | top-k (déjà supporté) |

#### 🆕 `18-rendement-pv.json` — « Rendements & lissage »
Ratios, lissage par sous-requête, comparaisons J vs J-1.

| Panel | Type | Requête PromQL | Apport |
|---|---|---|---|
| Production solaire totale | timeseries | `sum(venus_mppt_power_w) + sum(pvinv_power_w)` | agrégat (MPPT + micro-onduleurs) |
| Pic puissance 24 h (glissant) | stat | `max_over_time(sum(venus_mppt_power_w)[24h:5m])` | **sous-requête sur agrégat** |
| Rendement onduleur (AC/DC) | gauge | `sum(pvinv_power_w) / sum(dc_pv_power_w) * 100` | ratio (%) |
| Puissance MPPT lissée 1 h | timeseries | `avg_over_time(venus_mppt_power_w[1h:5m])` | **sous-requête** (anti-bruit) |
| Yield aujourd'hui vs hier | timeseries | `sum(venus_mppt_yield_today_kwh)` **et** `sum(venus_mppt_yield_today_kwh offset 24h)` (2 séries superposées) | `offset` (tendance) |

#### 🆕 `19-bilan-energie.json` — « Bilan énergétique J / J-1 / 7 j »
Comparaisons temporelles via `offset` + `@`, fiabilisées par le fix P0.

| Panel | Requête PromQL | Apport |
|---|---|---|
| Import réseau aujourd'hui | `increase(et112_energy_import_wh{address="0x09"}[24h]) / 1000` | P0 (séries clairsemées) |
| Import : aujourd'hui vs hier | `increase(et112_energy_import_wh{address="0x09"}[24h]) - increase(et112_energy_import_wh{address="0x09"}[24h] offset 24h)` | `offset` |
| Export / Import (ratio jour) | `increase(et112_energy_export_wh{address="0x09"}[24h]) / on(address) increase(et112_energy_import_wh{address="0x09"}[24h])` | `on(address)` |
| SOC à minuit (point-in-time) | `venus_shunt_soc_percent @ start()` | modifier `@` |
| Dérive SOC sur 24 h | `venus_shunt_soc_percent - venus_shunt_soc_percent offset 24h` | `offset` |
| Décharge cumulée 7 j | `sum(increase(venus_shunt_energy_out_kwh[7d]))` | tiering hourly |

#### 🆕 `20-alertes-avancees.json` — « Centre d'alertes multi-critères »
Dashboard d'alerting natif PromQL (chaque panneau = une règle Grafana Alert
exprimable en **une seule** requête grâce à `and`/`or`/`unless`).

| Alerte | Requête PromQL (déclenche si résultat non vide) | Apport |
|---|---|---|
| Surcharge thermique | `bms_soc < 20 and bms_temp_max > 45` | `and` multi-critères |
| Déséquilibre + cellule basse | `bms_cell_delta_mv > 50 and bms_min_cell_voltage < 3.0` | `and` |
| Sur-courant prolongé | `abs(bms_current) / on(bms_id) bms_reported_capacity_ah > 0.5` | `on()` (C-rate > 0.5C) |
| BMS muet (heartbeat) | `absent(bms_soc{bms_id="0x01"}) or absent(bms_soc{bms_id="0x02"})` | `absent` + `or` (orphans) |
| Onduleur OFF mais PV présent | `(venus_inverter_state == 0) and on() (sum(venus_mppt_power_w) > 100)` | `and` + comparaison |
| Toute alarme BMS active | `bms_alarm_high_temp > 0 or bms_alarm_low_voltage > 0 or bms_alarm_high_voltage > 0` | `or` |

### 9.2 Évolutions de dashboards existants

| Dashboard | Ajout proposé | Requête PromQL | Nouveauté |
|---|---|---|---|
| `01-bms` | Rangée « SLO & dispersion » | `quantile(0.95, bms_cell_voltage)`, `stddev by (bms_id)(bms_cell_voltage)` | `quantile`, `stddev` |
| `03-mppt` | Panneau « Rendement par instance » | `venus_mppt_power_w / on() venus_mppt_max_power_today_w` | `on()` |
| `04-smartshunt` | « Prédiction SOC 2 h (lissée) » | `predict_linear(avg_over_time(venus_shunt_soc_percent[10m])[2h:10m], 7200)` | sous-requête + predict_linear |
| `08-solaire` | « Production vs hier » (overlay) | `sum(venus_mppt_power_w)` **et** `sum(venus_mppt_power_w offset 24h)` | `offset` |
| `15-energy-manager` | « CPU lissé 5 min » | `avg_over_time(em_cpu_percent[5m:30s])` | sous-requête (anti-pic) |

### 9.3 Process de mise en œuvre (complet, sans régression)

#### Étape 1 — Validation locale (déjà faite, garde CI permanente)
```bash
# Vérifie que CHAQUE expr PromQL des 20 dashboards est acceptée par le moteur.
cargo test -p metrics-store --test golden_promql
# → provisioned_grafana_dashboards_coverage ... ok  (aucun panneau cassé)
```
Ce test lit dynamiquement `contrib/grafana/dashboards/*.json` : tout nouveau
dashboard (ou panneau) y est automatiquement couvert. Un `expr` non supporté
ferait **échouer la CI** avant tout déploiement.

#### Étape 2 — Récupération du code sur le Pi5
```bash
# (sur le Pi5, user pi5compute)
make sync                       # git fetch + reset --hard origin/<branche>
```

#### Étape 3 — Déploiement Grafana
```bash
bash scripts/deploy-pi5.sh      # importe TOUS les *.json via l'API Grafana
# (la boucle scripts/fix-grafana.sh itère contrib/grafana/dashboards/*.json :
#  les 4 nouveaux dashboards sont importés sans modification de script)
```
> ⚠️ **Aucun binaire à recompiler** : les dashboards ne sont pas embarqués dans
> `daly-bms-server` (ce sont des fichiers Grafana). Le moteur PromQL, lui, doit
> déjà tourner avec la version qui supporte les nouvelles constructions
> (Phase 7 — cf. `./metriques-promql-reference.md`). Sinon : `make build-arm` +
> redéploiement du binaire (cf. CLAUDE.md §0).

#### Étape 4 — Vérification post-déploiement
```bash
curl -s http://localhost:3000/api/search?tag=daly-bms | jq '.[].title'   # 20 dashboards
# Sanity-check d'une requête avancée directement sur le backend :
curl -s "http://localhost:8080/api/v1/query?query=quantile(0.95,bms_cell_voltage)" | jq .
curl -s "http://localhost:8080/api/v1/query?query=count(group(bms_soc)by(bms_id))" | jq .
```

#### Étape 5 — Alerting (dashboard 20)
Convertir chaque panneau de `20-alertes-avancees` en **règle Grafana Alert** :
condition « `count() > 0` sur la série de la requête » (la requête filtre déjà :
un résultat non vide = alerte active). Chaque alerte tient en **une seule**
requête PromQL, sans transformation Grafana ni jointure côté client.

#### Rollback
```bash
git revert <commit>             # puis make sync + bash scripts/deploy-pi5.sh
# ou supprimer les dashboards 17–20 via l'UI Grafana (Dashboards → Delete).
```

> **Note** : noms de métriques/labels issus des dashboards provisionnés actuels
> (`bms_*{bms_id="0x01"|"0x02"}`, `et112_*{address="0x09"}`, `venus_*`). Requêtes
> validées contre le moteur (mêmes constructions que les tests d'intégration
> `promql_set_ops_and_vector_matching`, `promql_new_aggregators`,
> `promql_at_modifier_and_subquery`).

---

## Sources consolidées

Ce document fusionne et **remplace** les anciens fichiers suivants :
`docs/redb-queries.md`, `docs/audit-metriques-redb.md`, `docs/promql-compat-roadmap.md`, `docs/Evolution-compliance-PromQL.md`.
