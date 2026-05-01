## rust-logic-graph peut améliorer votre système à plusieurs niveaux :

1. Centralisation et lisibilité : RLG permet d'extraire toutes vos règles métiers dispersées (comme la logique de courant de charge dans charge_current.rs) dans un fichier YAML/JSON unique ou un langage dédié (GRL). Cela apporte une vision centralisée de la logique métier de gestion d'énergie, qui se comporte alors comme un système expert, connu pour sa clarté et sa maintenabilité pour les règles métiers complexes.
2. Évolutivité et découplage : Si votre système doit complexifier ses règles d'orchestration, RLG devient précieux. Il offre des primitives avancées (circuit breakers, patrons Saga pour transactions distribuées) qui vous évitent de les coder à la main, éliminant ainsi la "dette architecturale" potentielle.

Je vous propose une approche de migration progressive pour concrétiser cela :

1. Phase 1 - Intégration côte-à-côte: Ajoutez RLG comme dépendance dans votre projet et définissez un premier graphe de logique (ex: décision de courant de batterie) dans un fichier YAML. Le reste de votre système reste inchangé.
2. Phase 2 - Service de décision dédié: Implémentez un petit service (ou endpoint HTTP) dans un module energy-decision. Ce service utilisera RLG pour évaluer les règles et retourner la décision.
3. Phase 3 - Refactorisation progressive: Modifiez un à un vos modules de logique (commençant par charge_current) pour qu'ils interrogent ce nouveau service au lieu d'implémenter la logique directement.

Voici un exemple concret d'intégration avec RLG :

```rust
// Fichier: energy-decision/src/main.rs
use rust_logic_graph::{Graph, Executor, Context};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Charger la définition du graphe depuis un fichier YAML
    let graph_def = Graph::from_yaml_file("rules/charge_decision.yaml")?;
    
    // 2. Créer l'exécuteur
    let mut executor = Executor::new(graph_def);
    
    // 3. Préparer le contexte avec vos données d'entrée (remplace state.read().await)
    let context = Context::new()
        .with_data("pv_power_w", json!(2450.0))
        .with_data("house_consumption_w", json!(800.0))
        .with_data("is_offgrid", json!(false));

    // 4. Exécuter le graphe de règles et récupérer les décisions
    let result = executor.execute(context).await?;
    
    // 5. Extraire la décision de courant de charge
    if let Some(charge_current) = result.get("max_charge_current_a") {
        // Publier sur MQTT ou via votre AppBus
        println!("Decision: Set charge current to {} A", charge_current);
    }
    Ok(())
}
```

Le fichier YAML correspondant (rules/charge_decision.yaml) aurait une structure claire et déclarative, par exemple :

```yaml
nodes:
  - id: "excess_pv"
    type: "rule"
    fields:
      - name: "excess_w"
        expr: "pv_power_w - house_consumption_w"
  - id: "charge_decision"
    type: "conditional"
    depends_on: ["excess_pv"]
    conditions:
      - when: "is_offgrid == true"
        set: "max_charge_current_a = 50"
      - when: "excess_w > 500"
        set: "max_charge_current_a = 30"
```


# Plan de migration détaillé pour trois modules clés de votre energy-manager. 

```markdown
# Plan de migration vers rust-logic-graph (RLG)

## Objectif
Remplacer progressivement la logique métier codée en dur dans les modules `charge_current`, `water_heater` et `solar_power` par des graphes de règles définis en YAML, exécutés via `rust-logic-graph`.  
La migration est **incrémentale** : chaque module reste fonctionnel pendant la transition.

---

## Architecture cible (par module)

```

energy-manager/
├── src/
│   ├── energy_state.rs          ← inchangé (état partagé)
│   ├── mqtt_client.rs           ← inchangé
│   ├── app_bus.rs               ← inchangé
│   ├── decision_engine/         ← NOUVEAU
│   │   ├── mod.rs
│   │   ├── executor.rs          ← wrapper autour de RLG
│   │   └── rules/               ← fichiers YAML par module
│   │       ├── charge_current.yaml
│   │       ├── water_heater.yaml
│   │       └── solar_power.yaml
│   ├── charge_current.rs        ← refactoré (appel au moteur)
│   ├── water_heater.rs          ← refactoré
│   └── solar_power.rs           ← refactoré

```

---

## Migration étape par étape

### Phase 0 – Prérequis (1 soirée)
1. Ajouter `rust-logic-graph` et `serde_yaml` dans `Cargo.toml` :
   ```toml
   [dependencies]
   rust-logic-graph = "0.4"   # ou dernière version
   serde_yaml = "0.9"
   tokio = { version = "1", features = ["full"] }
```

1. Créer le module decision_engine avec un exécuteur basique :
   ```rust
   // decision_engine/mod.rs
   use rust_logic_graph::{Graph, Executor, Context, Value};
   use std::collections::HashMap;
   use anyhow::Result;
   
   pub struct RuleEngine {
       executor: Executor,
   }
   
   impl RuleEngine {
       pub fn from_yaml(path: &str) -> Result<Self> {
           let graph = Graph::from_yaml_file(path)?;
           Ok(Self { executor: Executor::new(graph) })
       }
       
       pub async fn evaluate(&mut self, inputs: HashMap<String, Value>) -> Result<HashMap<String, Value>> {
           let ctx = Context::from(inputs);
           let output = self.executor.execute(ctx).await?;
           Ok(output.into())
       }
   }
   ```

---

Phase 1 – Migration de charge_current (2 jours)

1. Écrire les règles YAML

Fichier decision_engine/rules/charge_current.yaml :

```yaml
name: "Charge current decision"
nodes:
  - id: "excess_pv"
    type: "rule"
    fields:
      - name: "excess_w"
        expr: "pv_power_w - house_consumption_w"
      - name: "battery_soc_percent"
        expr: "battery_soc"
  - id: "current_limits"
    type: "conditional"
    depends_on: ["excess_pv"]
    conditions:
      - when: "is_offgrid == true"
        set:
          max_charge_current_a: 50
          max_discharge_current_a: 50
      - when: "excess_w > 500 && battery_soc_percent < 85"
        set: "max_charge_current_a = 30"
      - when: "excess_w <= 500 && battery_soc_percent < 90"
        set: "max_charge_current_a = 10"
      - default:
          max_charge_current_a: 0
```

2. Modifier charge_current.rs

Avant (logique codée en dur) → Après (délégation au moteur) :

```rust
// NOUVEAU : appel au moteur de règles
use crate::decision_engine::RuleEngine;
use once_cell::sync::Lazy;
use std::sync::Mutex;

static CHARGE_ENGINE: Lazy<Mutex<RuleEngine>> = Lazy::new(|| {
    Mutex::new(RuleEngine::from_yaml("rules/charge_current.yaml").unwrap())
});

pub async fn handle_charge_current(state: Arc<RwLock<EnergyState>>) -> Result<()> {
    let state_guard = state.read().await;
    let inputs = HashMap::from([
        ("pv_power_w".into(), state_guard.pv_power.into()),
        ("house_consumption_w".into(), state_guard.house_consumption.into()),
        ("battery_soc".into(), state_guard.battery_soc.into()),
        ("is_offgrid".into(), state_guard.is_offgrid.into()),
    ]);
    drop(state_guard);

    let mut engine = CHARGE_ENGINE.lock().unwrap();
    let outputs = engine.evaluate(inputs).await?;
    
    let new_current = outputs.get("max_charge_current_a")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    
    // Publier sur MQTT/AppBus (inchangé)
    publish_charge_current(new_current as u16).await?;
    Ok(())
}
```

3. Tester

· Conserver l’ancienne logique commentée pour comparaison.
· Jouer des scénarios (offgrid, excédent PV faible/fort) et vérifier la décision.

---

Phase 2 – Migration de water_heater (1.5 jour)

Règles typiques : chauffe-eau activé si excédent solaire > seuil ET température eau < max.

Fichier rules/water_heater.yaml :

```yaml
nodes:
  - id: "excess_pv"
    expr: "pv_power_w - house_consumption_w"
  - id: "should_heat"
    type: "conditional"
    conditions:
      - when: "excess_pv > 1000 && water_temp_c < 65"
        set: "heater_state = 'ON'"
      - when: "water_temp_c >= 65"
        set: "heater_state = 'OFF'"
      - default: "heater_state = 'OFF'"
```

Dans water_heater.rs : même pattern que pour charge_current.
Point d’attention : certaines règles peuvent dépendre d’une valeur horodatée (ex: heater_duration). Ajoutez un champ duration_minutes dans le contexte si nécessaire.

---

Phase 3 – Migration de solar_power (1 jour)

Ce module contient probablement de la logique de limitation d’injection ou de priorisation. Exemple de règle :

```yaml
nodes:
  - id: "grid_export"
    expr: "pv_power_w - house_consumption_w - charge_power_w"
  - id: "curtailment"
    type: "conditional"
    conditions:
      - when: "grid_export > 3000 && grid_limit_w == 2500"
        set: "pv_curtailment_w = grid_export - 2500"
      - default: "pv_curtailment_w = 0"
```

Intégration : le module solar_power calcule une consigne de limitation et la publie (MQTT solar/setpoint).

---

Tests recommandés :

· Pour chaque module, comparer l’ancienne et la nouvelle décision sur des traces réelles (via tracing).
· Injecter des valeurs extrêmes (PV=0, batterie pleine, eau bouillante…).

---

Rollback

Garder l’ancienne implémentation dans un module charge_current_v1 avec un feature flag :

```rust
#[cfg(feature = "legacy_charge")]
pub async fn handle_charge_current(...) { ... }
```

En cas de problème, recompilez avec --features legacy_charge.

---

Notes supplémentaires

· rust-logic-graph étant synchrone (bloquant) au sein de l’exécution du graphe, mais son exécution est async. Dans vos handlers, appelez .await sans crainte.
· Si vous utilisez tokio-cron-scheduler à côté, vous pouvez lancer l’évaluation périodique du graphe (ex: toutes les 5 secondes) et stocker les décisions dans un cache partagé.
· Pour la persistance des historiques de décision, envisagez d’ajouter un export vers victoriametrics.

---

