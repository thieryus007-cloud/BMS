## 📋 Intégration vmalert + webhook Rust + dashboard alertes

### 🎯 Résumé des changements

1. Charger les règles depuis un fichier externe (TOML ou YAML)
   → modifier bridge/alerts.rs pour lire /etc/daly-bms/alert-rules.toml
   
2. Étendre AlertContext avec toutes les sources
   → BmsSnapshot (déjà là)
   → VenusSmartShunt (venus_shunt_current_a, soc)  
   → Et112Snapshot (puissance réseau)
   → VenusInverter (tension DC bus)

3. Support `for: duration` per règle
   → Remplacer cooldown par un début de déclenchement + durée requise
   → Déjà partiellement là avec RuleState, ajouter pending_since: Option<Instant>

4. API REST + dashboard Askama
   → Lire la table alert_events existante
   → Ajouter acknowledge
---

BatterySOCCritical    → bms_soc < 15 for 3m      ✅ AlertEngine extensible
BatterySOCWarning     → bms_soc < 20 for 5m      ✅
HighDischargeCurrent  → shunt_current > 100 for 2m ✅ (SmartShunt dans AppState)
BatteryVoltageHigh    → bms_voltage > 57 for 1m   ✅
BatteryVoltageLow     → bms_voltage < 44 for 1m   ✅
BatteryTempHigh       → bms_temp > 45 for 3m      ✅ déjà là

Résumé des changements
9 fichiers modifiés/créés, 752 insertions, 124 suppressions.

## Ce qui change concrètement
Fichier	Ce qui a été fait
bridges/alerts.rs	Réécriture complète — for: duration, contexte SmartShunt, 2 nouvelles règles, méthodes API SQLite
config.rs	pack_ovp_v, pack_uvp_v + 5 champs de durée *_for_secs
state.rs	alert_engine: Option<Arc<AlertEngine>>
main.rs	AlertEngine créé avant AppState, partagé via Arc
api/alerts.rs	Nouveau — GET list/stats, POST acknowledge
api/mod.rs	3 nouvelles routes /api/v1/alerts/*
dashboard/mod.rs	Page /dashboard/alerts
templates/alerts.html	Nouveau — journal temps réel, filtres, pagination, bouton Ack
Config.toml	db_path activé, durées configurées, seuils ESS ajustés
Les 9 règles actives (7 existantes + 2 nouvelles)


