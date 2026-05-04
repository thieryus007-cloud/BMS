# Rules:
Si les trois conditions sont réunies, **alors** POST mode: "HEAT_PUMP".  
  1- On se trouve en mode Offgrid: AC IN ingnore Actif.  // IgnoreAcIn1: 0=grid, 1=off-grid.  
  2- Le SOC est supérieure à 90%.  
  3- l'irradiance est superieure à 300 w/m2.  // irradiance_wm2.  
  >> Sinon alors POST mode: "VACATION".    

Attention respécter la notation LG: 'HEAT_PUMP', 'TURBO', 'VACATION' .  
---

# 🧾 🧠 PR — Fix reliable mode switching for LG ThinQ water heater.  

## 🎯 Objectif

Garantir que :

* ✅ un **POST est envoyé quand nécessaire**
* ✅ pas de spam inutile vers LG.  
* ✅ comportement déterministe (indépendant d’un état local potentiellement faux).  
* ✅ robuste aux erreurs LG / reboot / désync.  

---

# 🚨 Problème actuel

La logique actuelle repose sur :

```rust
if target_mode == current_mode {
    continue;
}
```

👉 Ce modèle suppose que :

> `current_mode` reflète fidèlement l’état réel du chauffe-eau LG

❌ Ce n’est pas garanti :

* LG peut ignorer une commande
* délai de propagation
* reboot du service
* perte de synchro

👉 Résultat :

> ❌ blocage silencieux → aucun POST envoyé alors que nécessaire

---

# ✅ Solution proposée

👉 Introduire une logique basée sur :

### 🔑 1. `last_sent_mode`

➡️ ce que NOUS avons envoyé (source de vérité)

### 🔁 2. rate limiting

➡️ éviter le spam LG

### 🔍 3. (optionnel mais recommandé) sync avec LG

➡️ corriger dérives

---

# 🛠️ Modifications à implémenter

---

## 1. 🔄 Remplacer `current_mode` dans la décision

### ❌ AVANT

```rust
if target_mode == current_mode || !can_change {
    continue;
}
```

---

### ✅ APRÈS

```rust
let should_send = match last_sent_mode {
    Some(last) => last != target_mode,
    None => true,
};

if !should_send || !can_change {
    continue;
}
```

---

## 2. 🧠 Ajouter `last_sent_mode`

Dans `control_task` :

```rust
let mut last_sent_mode: Option<WaterHeaterMode> = None;
```

---

## 3. 📝 Mettre à jour après envoi

```rust
if let Err(e) = lg.set_mode(target_mode).await {
    tracing::error!("LG set_mode error: {e}");
    continue;
}

last_sent_mode = Some(target_mode);
last_change = Some(now);
```

---

## 4. 🔍 (OPTION RECOMMANDÉE) — Vérification réelle LG

👉 Ajouter un check périodique (ex: toutes les 5–10 minutes)

### Objectif :

Corriger ce cas :

```text
last_sent_mode = HeatPump
LG réel = Vacation
```

---

### Implémentation simple :

```rust
let lg_mode = lg.get_mode().await.ok();

if let Some(real_mode) = lg_mode {
    let real = WaterHeaterMode::from_lg_str(&real_mode);

    if Some(real) != last_sent_mode {
        tracing::warn!(
            "LG desync detected: real={:?}, last_sent={:?} → resync",
            real,
            last_sent_mode
        );

        last_sent_mode = Some(real);
    }
}
```

---

## 5. 🔁 (OPTION BONUS) — Refresh périodique

👉 Forcer un POST toutes les X minutes (sécurité)

```rust
let force_refresh = last_change
    .map(|t| (now - t).num_minutes() >= 10)
    .unwrap_or(true);

if (!should_send && !force_refresh) || !can_change {
    continue;
}
```

---

# 🧪 Comportement attendu après PR

| Situation              | Résultat            |
| ---------------------- | ------------------- |
| Conditions changent    | ✅ POST envoyé       |
| Conditions stables     | ❌ pas de spam       |
| LG ignore une commande | ✅ renvoi possible   |
| Redémarrage service    | ✅ resynchronisation |
| Désync LG              | ✅ corrigée          |

---

# ⚠️ Important

👉 `current_mode` dans `EnergyState` :

* ✔️ OK pour affichage / MQTT / debug
* ❌ NE DOIT PAS être utilisé pour décider d’envoyer une commande

---

# 🧼 Refactor recommandé (optionnel)

Remplacer :

```rust
current_mode
```

par :

```rust
// uniquement informatif
observed_mode
```

---

# 🧠 TL;DR

👉 Avant :

> “je n’envoie que si je pense que LG n’est pas dans le bon mode”

👉 Après :

> “j’envoie si je n’ai pas déjà envoyé ce mode récemment”

---

# ✅ Résultat final

* logique simple
* robuste
* prédictible
* fidèle à ton design Node-RED initial
* sans bug silencieux

---

version modéle **`control_task`**:

* ✅ envoie un POST quand nécessaire
* ✅ évite le spam
* ✅ ne dépend PAS de `current_mode`
* ✅ gère désynchronisation LG
* ✅ reste simple et lisible

---

# 🧠 Principes appliqués

* 🔑 **source de vérité = `last_sent_mode`**
* ❌ on ignore `current_mode` pour décider
* ⏱️ rate limit conservé
* 🔁 resync périodique avec LG
* 🔄 refresh de sécurité

---

# 🧾 ✅ Implémentation complète

```rust
async fn control_task(
    cfg: WaterHeaterConfig,
    lg: Arc<LgThinqClient>,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
) {
    let mut rule_engine = match rules::WaterHeaterRuleEngine::new() {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("Failed to init water heater rule engine: {e}");
            return;
        }
    };

    let mut last_change: Option<DateTime<Utc>> = None;
    let mut last_sent_mode: Option<WaterHeaterMode> = None;
    let mut last_lg_check: Option<DateTime<Utc>> = None;

    let mut ticker = interval(Duration::from_secs(30));

    loop {
        ticker.tick().await;
        let now = Utc::now();

        // ------------------------------------------------------------------
        // Read inputs
        // ------------------------------------------------------------------
        let (ac_ignore, soc, irradiance) = {
            let s = state.read().await;
            (
                s.ac_ignore.unwrap_or(0),
                s.soc_pct.unwrap_or(0.0),
                s.irradiance_wm2,
            )
        };

        let irradiance_low = irradiance
            .map(|w| w < cfg.irradiance_min_wm2)
            .unwrap_or(true);

        let grid_connected = ac_ignore == 0;

        // ------------------------------------------------------------------
        // Evaluate rules
        // ------------------------------------------------------------------
        let target_mode_str = match rule_engine.evaluate(
            grid_connected,
            soc,
            irradiance_low,
        ) {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("Rule engine error: {e} — fallback Vacation");
                "VACATION".to_string()
            }
        };

        let target_mode = match target_mode_str.as_str() {
            "HEAT_PUMP" => WaterHeaterMode::HeatPump,
            _ => WaterHeaterMode::Vacation,
        };

        // ------------------------------------------------------------------
        // Rate limiting
        // ------------------------------------------------------------------
        let can_change = last_change
            .map(|t| (now - t).num_seconds() as u64 >= cfg.mode_change_min_secs)
            .unwrap_or(true);

        // ------------------------------------------------------------------
        // Decide if we should send
        // ------------------------------------------------------------------
        let should_send = match last_sent_mode {
            Some(last) => last != target_mode,
            None => true, // first run
        };

        // Safety refresh (anti-stuck)
        let force_refresh = last_change
            .map(|t| (now - t).num_minutes() >= 10)
            .unwrap_or(true);

        if (!should_send && !force_refresh) || !can_change {
            continue;
        }

        tracing::info!(
            "Water heater: SEND {:?} (soc={:.1}%, grid={}, irradiance_low={})",
            target_mode,
            soc,
            grid_connected,
            irradiance_low
        );

        // ------------------------------------------------------------------
        // Send command
        // ------------------------------------------------------------------
        if let Err(e) = lg.set_mode(target_mode).await {
            tracing::error!("LG set_mode error: {e}");
            continue;
        }

        last_sent_mode = Some(target_mode);
        last_change = Some(now);

        // Update local state (for UI only)
        {
            let mut s = state.write().await;
            s.water_heater_mode = target_mode;
            s.water_heater_last_change = Some(now);
        }

        publish_to_venus(&bus, &state).await;

        // ------------------------------------------------------------------
        // Set temperature (delayed)
        // ------------------------------------------------------------------
        let delay_secs = cfg.temp_set_delay_secs;
        let target_temp = match target_mode {
            WaterHeaterMode::HeatPump => cfg.heat_pump_target_c,
            _ => cfg.vacation_target_c,
        };

        let lg2 = lg.clone();
        let bus2 = bus.clone();
        let state2 = state.clone();

        tokio::spawn(async move {
            sleep(Duration::from_secs(delay_secs)).await;

            if let Err(e) = lg2.set_target_temp(target_temp).await {
                tracing::error!("LG set_target_temp error: {e}");
                return;
            }

            {
                let mut s = state2.write().await;
                s.water_heater_target_c = Some(target_temp);
            }

            publish_to_venus(&bus2, &state2).await;
        });

        // ------------------------------------------------------------------
        // Periodic LG sync (every 5 min)
        // ------------------------------------------------------------------
        let need_sync = last_lg_check
            .map(|t| (now - t).num_minutes() >= 5)
            .unwrap_or(true);

        if need_sync {
            last_lg_check = Some(now);

            match lg.get_mode().await {
                Ok(real_mode_str) => {
                    let real_mode = WaterHeaterMode::from_lg_str(&real_mode_str);

                    if Some(real_mode) != last_sent_mode {
                        tracing::warn!(
                            "LG desync detected → real={:?}, last_sent={:?}",
                            real_mode,
                            last_sent_mode
                        );

                        last_sent_mode = Some(real_mode);
                    }
                }
                Err(e) => {
                    tracing::warn!("LG get_mode failed: {e}");
                }
            }
        }
    }
}
```

---

# 🎯 Ce que ce code garantit

### ✅ envoies quand il faut

* changement de conditions → POST

### ✅ n’envoies pas inutilement

* même mode → pas de spam

### ✅ récupères les erreurs LG

* désync détectée → corrigée

### ✅ ne dépends plus d’un état faux

* `current_mode` ignoré pour décision

---
