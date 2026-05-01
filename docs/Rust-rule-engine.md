# 🚀 Migration `water_heater.rs` vers `rust-rule-engine`

---

## 📋 Votre logique actuelle (résumé)

```rust
// Évaluation toutes les 30s avec 5 conditions :
// 1. grid_on (ac_ignore == 0)
// 2. soc_low (soc < 95%)
// 3. discharge_too_long (batterie décharge > debounce_secs)
// 4. solar_too_low (solar <= solar_min_w > debounce_secs)
// 5. irradiance_low (irradiance < min, immédiat)
// → Si ANY true : mode = Vacation, sinon HeatPump
// + Rate limiting 15min + debounce 5min + delay température
```

---

## 🎯 Architecture de migration en 3 phases

### Phase 1 : Setup et types partagés

```toml
# Cargo.toml - Ajoutez rust-rule-engine
[dependencies]
rust-rule-engine = { version = "1.20", default-features = false, features = ["streaming"] }
```

```rust
// src/logic/water_heater/rules.rs (NOUVEAU)
use rust_rule_engine::{
    RustRuleEngine, KnowledgeBase, Facts, Value, GRLParser, 
    streaming::{WindowConfig, WindowType}
};
use serde::{Deserialize, Serialize};

/// Faits injectés dans le moteur de règles
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaterFacts {
    pub grid_connected: bool,
    pub soc_pct: f64,
    pub batt_current_a: f64,
    pub solar_total_w: f64,
    pub irradiance_wm2: Option<f64>,
    pub current_mode: String, // "HeatPump" | "Vacation"
    pub last_change_secs_ago: Option<u64>,
    
    // Config (peut venir du fichier TOML)
    pub debounce_secs: u64,
    pub solar_min_w: f64,
    pub irradiance_min_wm2: f64,
    pub mode_change_min_secs: u64,
}

impl HeaterFacts {
    pub fn to_facts(&self) -> Facts {
        let mut facts = Facts::new();
        facts.insert("grid_connected", Value::Bool(self.grid_connected));
        facts.insert("soc_pct", Value::Number(self.soc_pct.into()));
        facts.insert("batt_current_a", Value::Number(self.batt_current_a.into()));
        facts.insert("solar_total_w", Value::Number(self.solar_total_w.into()));
        facts.insert("irradiance_wm2", match self.irradiance_wm2 {
            Some(v) => Value::Number(v.into()),
            None => Value::Null,
        });
        facts.insert("current_mode", Value::String(self.current_mode.clone()));
        facts.insert("debounce_secs", Value::Number(self.debounce_secs.into()));
        facts.insert("solar_min_w", Value::Number(self.solar_min_w.into()));
        facts.insert("irradiance_min_wm2", Value::Number(self.irradiance_min_wm2.into()));
        facts.insert("mode_change_min_secs", Value::Number(self.mode_change_min_secs.into()));
        
        if let Some(secs) = self.last_change_secs_ago {
            facts.insert("last_change_secs_ago", Value::Number(secs.into()));
        }
        facts
    }
}
```

---

### Phase 2 : Règles GRL externes (fichier `water_heater.grl`)

```grl
// rules/water_heater.grl - Chargé depuis un fichier externe
// ✅ Avantage : règles modifiables sans recompilation !

// ── Conditions élémentaires ─────────────────────────────

rule "Condition_GridConnected" salience 100 {
    when
        grid_connected == true
    then
        facts.insert("condition_grid", Value::Bool(true));
}

rule "Condition_SOC_Low" salience 100 {
    when
        soc_pct < 95.0
    then
        facts.insert("condition_soc_low", Value::Bool(true));
}

rule "Condition_Irradiance_Low_Immediate" salience 200 {
    when
        irradiance_wm2 != null && irradiance_wm2 < irradiance_min_wm2
    then
        facts.insert("condition_irradiance_low", Value::Bool(true));
}

// ── Conditions avec debounce (via streaming window) ─────

// Note: Le debounce est géré en amont dans Rust, 
// mais on peut aussi utiliser les fenêtres temporelles du moteur

rule "Condition_Discharging_Long" salience 100 {
    when
        batt_current_a < 0.0  // Décharge
        // Le debounce est appliqué avant injection des faits
        && condition_discharging_debounced == true
    then
        facts.insert("condition_discharge_long", Value::Bool(true));
}

rule "Condition_Solar_Low_Long" salience 100 {
    when
        solar_total_w <= solar_min_w
        && condition_solar_low_debounced == true
    then
        facts.insert("condition_solar_low_long", Value::Bool(true));
}

// ── Décision finale ─────────────────────────────────────

rule "Decide_Vacation_Mode" salience 500 {
    when
        condition_grid == true 
        || condition_soc_low == true 
        || condition_irradiance_low == true
        || condition_discharge_long == true
        || condition_solar_low_long == true
    then
        facts.insert("target_mode", Value::String("Vacation".to_string()));
        facts.insert("should_change_mode", Value::Bool(true));
}

rule "Decide_HeatPump_Mode" salience 400 {
    when
        target_mode == null  // Aucune condition Vacation déclenchée
    then
        facts.insert("target_mode", Value::String("HeatPump".to_string()));
        facts.insert("should_change_mode", Value::Bool(
            facts.get("current_mode").and_then(|v| v.as_string())
                .map(|m| m != "HeatPump").unwrap_or(false)
        ));
}

// ── Rate limiting ───────────────────────────────────────

rule "Respect_Rate_Limit" salience 600 {
    when
        should_change_mode == true
        && last_change_secs_ago != null
        && last_change_secs_ago < mode_change_min_secs
    then
        facts.insert("should_change_mode", Value::Bool(false));
        facts.insert("rate_limit_wait_secs", Value::Number(
            (mode_change_min_secs - last_change_secs_ago).into()
        ));
}
```

---

### Phase 3 : Intégration dans `water_heater.rs`

```rust
// src/logic/water_heater.rs - Version migrée
use rust_rule_engine::{RustRuleEngine, KnowledgeBase, GRLParser};
use crate::logic::water_heater::rules::{HeaterFacts, WaterHeaterConfig};

pub struct WaterHeaterEngine {
    engine: RustRuleEngine,
    config: WaterHeaterConfig,
}

impl WaterHeaterEngine {
    pub fn new(config: WaterHeaterConfig) -> anyhow::Result<Self> {
        // Charger les règles depuis fichier (ou string en fallback)
        let grl_content = std::fs::read_to_string("rules/water_heater.grl")
            .unwrap_or_else(|_| include_str!("../../rules/water_heater.grl").to_string());
        
        let rules = GRLParser::parse_rules(&grl_content)
            .map_err(|e| anyhow::anyhow!("GRL parse error: {}", e))?;
        
        let mut kb = KnowledgeBase::new("water_heater");
        for rule in rules {
            kb.add_rule(rule)
                .map_err(|e| anyhow::anyhow!("Rule add error: {}", e))?;
        }
        
        Ok(Self {
            engine: RustRuleEngine::new(kb),
            config,
        })
    }

    /// Évalue les conditions et retourne la décision
    pub async fn evaluate(&mut self, facts: HeaterFacts) -> HeaterDecision {
        let mut engine_facts = facts.to_facts();
        
        // Exécuter le moteur (chaînage avant)
        if let Err(e) = self.engine.evaluate(&mut engine_facts) {
            tracing::error!("Rule engine evaluation error: {}", e);
            return HeaterDecision::NoChange; // Fail-safe
        }
        
        // Extraire la décision
        let should_change = engine_facts.get("should_change_mode")
            .and_then(|v| v.as_bool()).unwrap_or(false);
            
        if !should_change {
            return HeaterDecision::NoChange;
        }
        
        let target_mode = engine_facts.get("target_mode")
            .and_then(|v| v.as_string())
            .and_then(|s| match s.as_str() {
                "Vacation" => Some(WaterHeaterMode::Vacation),
                "HeatPump" => Some(WaterHeaterMode::HeatPump),
                _ => None,
            });
            
        let wait_secs = engine_facts.get("rate_limit_wait_secs")
            .and_then(|v| v.as_number()).map(|n| n as u64);
        
        match (target_mode, wait_secs) {
            (Some(mode), None) => HeaterDecision::Change { 
                mode, 
                set_temp_after: Some(self.config.temp_set_delay_secs) 
            },
            (Some(mode), Some(wait)) => HeaterDecision::RateLimited { 
                mode, 
                wait_secs: wait 
            },
            _ => HeaterDecision::NoChange,
        }
    }
}

/// Résultat de l'évaluation
pub enum HeaterDecision {
    NoChange,
    Change { 
        mode: WaterHeaterMode, 
        set_temp_after: Option<u64> 
    },
    RateLimited { 
        mode: WaterHeaterMode, 
        wait_secs: u64 
    },
}
```

---

### Phase 4 : Boucle principale simplifiée

```rust
// Dans control_task() - version migrée
async fn control_task(
    cfg: WaterHeaterConfig,
    lg: Arc<LgClient>,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
) {
    // Initialiser le moteur de règles (une fois)
    let mut rule_engine = match WaterHeaterEngine::new(cfg.clone()) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("Failed to init rule engine: {}", e);
            return; // Fallback: désactiver l'auto-management
        }
    };
    
    let mut ticker = interval(Duration::from_secs(30));
    
    loop {
        ticker.tick().await;
        
        // 1. Collecter les faits depuis l'état partagé
        let facts = {
            let s = state.read().await;
            let now = Utc::now();
            HeaterFacts {
                grid_connected: s.ac_ignore.unwrap_or(0) == 0,
                soc_pct: s.soc_pct.unwrap_or(0.0),
                batt_current_a: s.battery_current_a.unwrap_or(0.0),
                solar_total_w: s.solar_total_w,
                irradiance_wm2: s.irradiance_wm2,
                current_mode: s.water_heater_mode.to_string(),
                last_change_secs_ago: s.water_heater_last_change
                    .map(|t| (now - t).num_seconds() as u64),
                // Config
                debounce_secs: cfg.debounce_secs,
                solar_min_w: cfg.solar_min_w,
                irradiance_min_wm2: cfg.irradiance_min_wm2,
                mode_change_min_secs: cfg.mode_change_min_secs,
            }
        };
        
        // 2. Évaluer avec le moteur de règles
        let decision = rule_engine.evaluate(facts).await;
        
        // 3. Appliquer la décision
        match decision {
            HeaterDecision::Change { mode, set_temp_after } => {
                info!("Water heater: rule engine decided {:?} → {:?}", 
                    facts.current_mode, mode);
                
                if let Err(e) = lg.set_mode(mode).await {
                    tracing::error!("LG set_mode error: {e}");
                    continue;
                }
                
                // Mettre à jour l'état
                {
                    let mut s = state.write().await;
                    s.water_heater_mode = mode;
                    s.water_heater_last_change = Some(Utc::now());
                }
                publish_to_venus(&bus, &state).await;
                
                // Delay pour température
                if let Some(delay) = set_temp_after {
                    let target_temp = match mode {
                        WaterHeaterMode::HeatPump => cfg.heat_pump_target_c,
                        _ => cfg.vacation_target_c,
                    };
                    spawn_set_temp_delay(lg.clone(), bus.clone(), state.clone(), target_temp, delay).await;
                }
            }
            HeaterDecision::RateLimited { mode, wait_secs } => {
                debug!("Water heater: mode change to {mode:?} rate-limited, {wait_secs}s remaining");
            }
            HeaterDecision::NoChange => {
                // Rien à faire - logging optionnel en debug
            }
        }
    }
}
```

---

## ✅ Avantages de cette migration

| Aspect | Avant (Rust pur) | Après (rust-rule-engine) |
|--------|-----------------|-------------------------|
| **Modif règles** | Recompilation nécessaire | Édition fichier `.grl` seule |
| **Lisibilité** | Logique dispersée dans le code | Règles métier centralisées, déclaratives |
| **Tests** | Tests unitaires Rust complexes | Tests GRL + snapshots simples |
| **Debug** | Logs manuels | `engine.trace()` intégré |
| **Évolutivité** | Ajouter condition = modifier code | Ajouter règle `.grl` sans toucher au Rust |
| **Sécurité** | `unwrap()` potentiels | Gestion d'erreur centralisée au parse/eval |

---

## 🧪 Exemple de test GRL (fichier `tests/water_heater_rules_test.grl`)

```grl
// Test: SOC bas → Vacation
@test "soc_94_triggers_vacation" {
    given
        grid_connected = false,
        soc_pct = 94.0,
        batt_current_a = 5.0,
        solar_total_w = 2000.0,
        irradiance_wm2 = 800.0,
        current_mode = "HeatPump",
        debounce_secs = 300,
        solar_min_w = 500.0,
        irradiance_min_wm2 = 100.0,
        mode_change_min_secs = 900
    when
        evaluate
    then
        assert target_mode == "Vacation",
        assert should_change_mode == true
}

// Test: Rate limiting bloque changement
@test "rate_limit_blocks_change" {
    given
        grid_connected = true,  // Devrait trigger Vacation
        soc_pct = 98.0,
        current_mode = "HeatPump",
        last_change_secs_ago = 300,  // Seulement 5min, besoin de 15min
        mode_change_min_secs = 900
    when
        evaluate
    then
        assert should_change_mode == false,
        assert rate_limit_wait_secs == 600  // 10min d'attente
}
```

Exécution :
```bash
cargo test --test water_heater_rules
# Ou via le moteur : engine.run_tests("tests/water_heater_rules_test.grl")?;
```

---

## 🔄 Stratégie de déploiement progressive

1. **Semaine 1** : Ajouter `rust-rule-engine` en dépendance, créer `rules/water_heater.grl` avec 2 règles simples (grid + SOC)
2. **Semaine 2** : Faire tourner les deux systèmes en parallèle (log uniquement), comparer les décisions
3. **Semaine 3** : Basculer en production avec feature flag `--features use-rule-engine`
4. **Semaine 4** : Retirer l'ancienne logique, ajouter les tests GRL dans la CI

```toml
# Cargo.toml - Feature flag pour migration douce
[features]
default = []
use-rule-engine = ["dep:rust-rule-engine"]

# Dans water_heater.rs
#[cfg(feature = "use-rule-engine")]
mod rules;
#[cfg(feature = "use-rule-engine")]
use rules::WaterHeaterEngine;
```

---

## ⚠️ Points d'attention spécifiques BMS

```rust
// Gestion d'erreur CRITIQUE pour embarqué
match rule_engine.evaluate(&mut facts) {
    Ok(_) => { /* continuer */ }
    Err(e) => {
        // NE PANIQUEZ PAS en production embarquée !
        tracing::error!("Rule engine failed, fallback to safe mode: {}", e);
        // Retourner à l'état le plus sûr (Vacation = moins de conso)
        return HeaterDecision::Change { 
            mode: WaterHeaterMode::Vacation,
            set_temp_after: None 
        };
    }
}

// Validation des faits en entrée (éviter les règles qui tournent sur données corrompues)
impl HeaterFacts {
    pub fn validate(&self) -> anyhow::Result<()> {
        if !(0.0..=100.0).contains(&self.soc_pct) {
            anyhow::bail!("Invalid SOC: {}", self.soc_pct);
        }
        if self.solar_total_w < 0.0 {
            anyhow::bail!("Negative solar power");
        }
        Ok(())
    }
}
```

---

## 🎁 Bonus : Hot-reload des règles en production

```rust
// Ajouter un watcher sur le fichier .grl pour recharger sans restart
use notify::{Watcher, RecursiveMode};

pub fn spawn_rule_hot_reload(
    engine: Arc<RwLock<WaterHeaterEngine>>,
    path: &str,
) -> anyhow::Result<()> {
    let path = path.to_string();
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = notify::recommended_watcher(tx)?;
        watcher.watch(path.as_ref(), RecursiveMode::NonRecursive)?;
        
        for event in rx {
            if let notify::EventKind::Modify(_) = event? {
                tracing::info!("Rule file changed, reloading...");
                match WaterHeaterEngine::new(cfg.clone()) {
                    Ok(new_engine) => {
                        *engine.blocking_write() = new_engine;
                        tracing::info!("Rules reloaded successfully");
                    }
                    Err(e) => tracing::error!("Reload failed: {}", e),
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    });
    Ok(())
}
```
# Hot-reload:
// Ajouter un watcher sur le fichier .grl pour recharger sans restart
use notify::{Watcher, RecursiveMode};

pub fn spawn_rule_hot_reload(
    engine: Arc<RwLock<WaterHeaterEngine>>,
    path: &str,
) -> anyhow::Result<()> {
    let path = path.to_string();
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = notify::recommended_watcher(tx)?;
        watcher.watch(path.as_ref(), RecursiveMode::NonRecursive)?;
        
        for event in rx {
            if let notify::EventKind::Modify(_) = event? {
                tracing::info!("Rule file changed, reloading...");
                match WaterHeaterEngine::new(cfg.clone()) {
                    Ok(new_engine) => {
                        *engine.blocking_write() = new_engine;
                        tracing::info!("Rules reloaded successfully");
                    }
                    Err(e) => tracing::error!("Reload failed: {}", e),
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    });
    Ok(())
}
---

> 💡 **Recommandation finale** : Commencez par migrer **seulement la règle SOC < 95%** vers GRL, gardez le reste en Rust le temps de valider l'approche. Cela réduit le risque et vous permet d'itérer rapidement.

