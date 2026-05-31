# redb — Référence des métriques et requêtes PromQL

> Backend : **redb** (`/mnt/nvme/daly-bms/metrics.redb`), interrogé via le **shim PromQL** de `daly-bms-server` sur le port **8080**.
> URL API : `http://192.168.1.141:8080/api/v1/query?query=<PROMQL>`
> Range   : `http://192.168.1.141:8080/api/v1/query_range?query=<PROMQL>&start=…&end=…&step=…`
> Visualisation : dashboard custom interne **`/dashboard/history`** et **Grafana** (`:3000`, datasource « Daly Metrics (redb) »).

> ℹ️ **Sous-ensemble PromQL supporté** — Le shim redb n'implémente qu'un sous-ensemble audité de PromQL (cf. `crates/metrics-store/src/promql/validate.rs` et `docs/plan_migration_vm_redb.md` §6.5). Toute construction hors liste blanche est rejetée avec `status=error`, `errorType=bad_data`.
>
> **Fonctions à fenêtre** (`f(m[range])`) : `increase`, `rate`, `irate`, `delta`, `deriv`, `predict_linear`, `changes`, `resets`, `avg_over_time`, `sum_over_time`, `min_over_time`, `max_over_time`, `count_over_time`, `last_over_time`, `stddev_over_time`, `stdvar_over_time`, `quantile_over_time`, `absent_over_time`.
> **Fonctions instantanées** : `abs`, `clamp_min`, `clamp_max`, `clamp`, `ceil`, `floor`, `round`, `sqrt`, `exp`, `ln`, `log2`, `log10`, `sgn`, `absent`.
> **Manipulation de labels** : `label_replace`, `label_join`.
> **Agrégateurs** : `sum`, `max`, `min`, `avg`, `count` (avec `by (…)` / `without (…)`), `topk(k, …)`, `bottomk(k, …)`.
> **Opérateurs** : arithmétiques `+ - * /` (vecteur⊗scalaire ou vecteur⊗vecteur **aligné**), comparaisons `== != > < >= <=` (filtre ou `bool`).
>
> **Non supporté** : `integrate` et les autres fonctions MetricsQL, les **subqueries** `[Xh:Ym]`, les modifiers `offset` / `@`, le vector matching `on()` / `ignoring()` / `group_left` / `group_right`, les set ops `and` / `or` / `unless`, les agrégateurs paramétrés `quantile` / `count_values`.

---

## Calculs de charge / décharge en Ampères-heures (Ah)

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

### 📐 Formule du taux de cyclage
```
Taux de cyclage (%) = (Ah chargés + Ah déchargés) / Capacité batterie × 100
```
> ⚠️ On additionne les valeurs absolues : une batterie qui charge 50 Ah puis décharge 50 Ah a échangé **100 Ah**, soit un cyclage de 50% sur une batterie de 200 Ah.

---

### ✅ Requête PromQL pour votre configuration

#### 🔹 Avec capacité en dur (ex: 200 Ah)
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

#### 🔹 Avec capacité dynamique (via un metric)
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

### 🌐 URL API prête à l'emploi
```
http://192.168.1.141:8080/api/v1/query?query=(avg_over_time(clamp_min(venus_shunt_current_a,0)[24h])-avg_over_time(clamp_max(venus_shunt_current_a,0)[24h]))*24/200*100
```

#### 🧪 Test rapide avec `curl`
```bash
# Taux de cyclage sur 24h (capacité 200 Ah)
curl -s "http://192.168.1.141:8080/api/v1/query" \
  --data-urlencode "query=(avg_over_time(clamp_min(venus_shunt_current_a,0)[24h])-avg_over_time(clamp_max(venus_shunt_current_a,0)[24h]))*24/200*100" \
  | jq -r '.data.result[0].value[1] + " %"'
```
→ Résultat attendu : `XX.XX %`

---

### 📊 Interprétation des résultats

| Taux de cyclage | Interprétation | Impact batterie |
|----------------|----------------|-----------------|
| **0–20 %** | Usage léger | ✅ Longévité maximale |
| **20–50 %** | Usage modéré | ✅ Normal pour usage quotidien |
| **50–80 %** | Usage intensif | ⚠️ Surveiller la température et la tension |
| **> 80 %** | Cyclage profond | 🔋 Privilégier batteries LiFePO4 ; éviter sur plomb |

> 💡 Pour les batteries au plomb, il est recommandé de ne pas dépasser **50 % de profondeur de décharge** (DoD) pour préserver leur durée de vie.

---

### 🎨 Intégration Grafana (bonus)

#### 1. Panel "Taux de cyclage quotidien"
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

#### 2. Variable pour la capacité (optionnel)
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

### ⚠️ Points de vigilance

1. **Fenêtre glissante vs jour calendaire**  
   `[24h]` calcule sur les dernières 24h glissantes. Pour un jour calendaire (minuit → maintenant), il faut calculer un `offset` dynamique côté client — le modifier `offset` n'étant pas supporté par le shim, ajustez la borne `start`/`end` de `query_range` côté appelant.

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

## Récapitulatif des métriques par appareil

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


# Vérification : nombre de métriques présentes (valeurs distinctes de __name__)
# curl http://192.168.1.141:8080/api/v1/label/__name__/values | jq '.data | length'
# (NB : /api/v1/labels renvoie les NOMS de labels — bms_id, address… — pas les métriques)
# ou via le dashboard custom /dashboard/history (sélecteur de série)

# Dernier point de chaque métrique (vérification fraîcheur)
{__name__=~"bms_soc|venus_shunt_soc_percent|solar_total_w|ats_active_source|tasmota_power_on"}
```

---

## Throttles d'écriture redb

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
