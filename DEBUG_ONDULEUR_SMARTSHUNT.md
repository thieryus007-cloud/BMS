# INVESTIGATION — Onduleur & SmartShunt Pas de Données

## 🔍 PROCÉDURE D'INVESTIGATION SYSTÉMATIQUE

Exécuter chaque commande **dans l'ordre exact** sur Pi5, noter les résultats.

---

## ÉTAPE 1: Vérifier que les services tournent

```bash
# Sur Pi5 — État des services
systemctl status daly-bms | head -20
docker ps | grep mosquitto
```

**Résultat attendu:**
- daly-bms: `active (running)`
- mosquitto: container running

**Si NOK:** 
- Redémarrer: `sudo systemctl restart daly-bms && docker restart mosquitto`

---

## ÉTAPE 2: Vérifier les logs BMS (erreurs MQTT)

```bash
# Sur Pi5 — Logs du serveur BMS (30 dernières lignes)
journalctl -u daly-bms -n 30 --no-pager
```

**Chercher:**
- Erreurs MQTT: `"MQTT connection error"`, `"Failed to subscribe"`
- Erreurs parsing: `"Failed to parse JSON"`
- Messages reçus: `"Updated inverter"`, `"Updated smartshunt"`

**Si erreurs MQTT:**
- → Vérifier MQTT broker (étape 3)

**Si PAS de messages "Updated":**
- → Vérifier les topics MQTT avec mosquitto_sub

---

## ÉTAPE 3: Vérifier le broker MQTT

```bash
# Sur Pi5 — Vérifier que Mosquitto écoute
netstat -tlnp | grep 1883
docker exec mosquitto mosquitto_sub -h localhost -p 1883 -t '$SYS/#' -C 1
```

**Résultat attendu:**
- Mosquitto écoute sur 0.0.0.0:1883
- `$SYS` topics publiés = broker actif

**Si broker down:**
- Redémarrer: `docker restart mosquitto`
- Attendre 5s
- Tester à nouveau

---

## ÉTAPE 4: Vérifier les topics MQTT (node-RED publie?)

```bash
# Sur Pi5 — Watch tous les topics santuario pendant 30 secondes
echo "Watching MQTT for 30 seconds..."
timeout 30 docker exec mosquitto mosquitto_sub -h localhost -p 1883 -t 'santuario/#' -v 2>&1 | head -100
```

**Résultat attendu:**
```
santuario/inverter/venus {"Voltage": ..., "AcPower": ...}
santuario/system/venus {"Voltage": ..., "Current": ...}
santuario/meteo/venus {"MpptPower": ...}
santuario/bms/1/venus {...}
santuario/bms/2/venus {...}
```

### 🔴 SI `santuario/inverter/venus` EST ABSENT:

**Diagnostic:**
```bash
```

### 🔴 SI `santuario/system/venus` EST ABSENT:

---

## ÉTAPE 5: Vérifier les API endpoints

```bash
# Sur Pi5 — Tester les endpoints directement
echo "=== INVERTER ENDPOINT ==="
curl -s http://localhost:8080/api/v1/venus/inverter | jq '.'

echo "=== SMARTSHUNT ENDPOINT ==="
curl -s http://localhost:8080/api/v1/venus/smartshunt | jq '.'

echo "=== MPPT ENDPOINT ==="
curl -s http://localhost:8080/api/v1/venus/mppt | jq '.'
```

**Résultat attendu:**
```json
{
  "connected": true,
  "inverter": {
    "voltage_v": 48.2,
    "ac_output_power_w": 1286.0,
    ...
  }
}
```

### 🔴 SI `"connected": false`:
- **Cause:** AppState n'a jamais reçu de données MQTT

**Debug:**
- Vérifier étape 4 (MQTT topics publiés?)
- Vérifier logs BMS (étape 2) pour messages "Updated"

### 🔴 SI API endpoint retourne 404:
- **Cause:** Route non enregistrée dans router

**Debug:**
- Vérifier `crates/daly-bms-server/src/api/mod.rs` contient `.route("/api/v1/venus/inverter", ...)`
- Vérifier compilation était sans erreurs

---

## ÉTAPE 6: Vérifier les MQTT handlers reçoivent les données

```bash
# Sur Pi5 — Activer logs debug
# Éditer: /etc/daly-bms/config.toml ou Config.toml
# Ajouter ou modifier:
RUST_LOG=debug

# Redémarrer avec logs debug
RUST_LOG=debug systemctl restart daly-bms
sleep 2
journalctl -u daly-bms -f &  # Laisser tourner en background

# Dans une autre terminal, déclencher un message MQTT:
docker exec mosquitto mosquitto_pub -h localhost -p 1883 -t 'santuario/inverter/venus' \
  -m '{"Voltage": 48.2, "Current": 3.5, "Power": 168.7, "AcVoltage": 229.8, "AcCurrent": 5.6, "AcPower": 1286.0, "State": "on", "Mode": "inverter"}'

# Vérifier si le handler a reçu et parsé le message
# Doit voir: "Updated inverter" dans les logs
```

---

## ÉTAPE 7: Vérifier le Dashboard fetch les endpoints

```bash
# Sur Pi5 — Ouvrir le dashboard et tester dans la console du navigateur
# Ouvrir: http://192.168.1.141:8080/visualization
# Ouvrir DevTools (F12) → Console

# Exécuter dans la console:
fetch('/api/v1/venus/inverter').then(r => r.json()).then(console.log)
fetch('/api/v1/venus/smartshunt').then(r => r.json()).then(console.log)

# Résultat attendu: affiche les objets JSON avec connected: true et les données
```

---

## 🎯 ARBRE DE DÉCISION — TROUVER LA CAUSE

```
Onduleur affiche "—" ?
    │
    ├─→ Vérifier étape 4: Topic 'santuario/inverter/venus' publié?
    │   │
    │   ├─ NON → Node-RED flow inverter.json pas déployé
    │   │        ACTION: Aller à section "FIX NODE-RED"
    │   │
    │   └─ OUI → Aller à étape 5
    │
    └─→ Vérifier étape 5: API /venus/inverter retourne connected: true?
        │
        ├─ NON (connected: false) → MQTT reçu par handler?
        │   │
        │   └─→ Vérifier logs étape 2 pour "Updated inverter"
        │       │
        │       ├─ NON → MQTT handler pas appelé
        │       │        ACTION: Vérifier mqtt.rs a le subscribe correct
        │       │
        │       └─ OUI → AppState pas mise à jour
        │                ACTION: Vérifier on_venus_inverter() est appelé
        │
        ├─ OUI (connected: true) → Dashboard ne fetch pas?
        │                          ACTION: Vérifier étape 7
        │
        └─ API 404 → Route non enregistrée
                     ACTION: Vérifier api/mod.rs
```

---

## 🔧 FIXES RAPIDES

### FIX #3: MQTT handler pas enregistré

**Vérifier:** `crates/daly-bms-server/src/bridges/mqtt.rs`

Doit contenir (vers ligne 50-60):
```rust
// Dans async fn connect_mqtt():
mqtt_client.subscribe("santuario/inverter/venus", QoS::AtLeastOnce).await?;
mqtt_client.subscribe("santuario/system/venus", QoS::AtLeastOnce).await?;

// Dans le match pattern de réception (vers ligne 100-120):
"santuario/inverter/venus" => handle_inverter_topic(&state, &json).await,
"santuario/system/venus" => handle_system_topic(&state, &json).await,
```

**Si absent:**
```bash
git pull origin claude/realtime-metrics-dashboard-lUKF3
make build-arm
sudo systemctl restart daly-bms
```

### FIX #4: API route non enregistrée

**Vérifier:** `crates/daly-bms-server/src/api/mod.rs`

Doit contenir (vers ligne 50-60):
```rust
.route("/api/v1/venus/inverter", get(system::get_venus_inverter))
.route("/api/v1/venus/smartshunt", get(system::get_venus_smartshunt))
```

**Si absent:** même procédure que FIX #3

### FIX #5: Serveur BMS pas recompilé avec changements récents

```bash
cd ~/Daly-BMS-Rust
git pull origin claude/realtime-metrics-dashboard-lUKF3
make build-arm
sudo systemctl stop daly-bms
sudo cp target/aarch64-unknown-linux-gnu/release/daly-bms-server /usr/local/bin/
sudo systemctl start daly-bms
sleep 3
journalctl -u daly-bms -n 10
```

---

## ✅ CHECKLIST DE VÉRIFICATION

```
□ Services tournent (BMS, Mosquitto, energy-manager)
  Commande: systemctl status daly-bms && docker ps

□ MQTT topics publiés
  Commande: mosquitto_sub -t 'santuario/#' -v

□ API endpoints répondent
  Commande: curl http://localhost:8080/api/v1/venus/inverter

□ Dashboard peut fetch l'API
  Console navigateur: fetch('/api/v1/venus/inverter').then(r => r.json()).then(console.log)

  Accès: http://192.168.1.141:1880 → vérifier flows présents et "Deploy" gris

□ Logs BMS sans erreurs
  Commande: journalctl -u daly-bms -n 50
```

---

## 📋 RAPPORT À FOURNIR

Une fois le problème identifié, fournir:

1. **Résultat étape 4:** Topics MQTT publiés oui/non
2. **Résultat étape 5:** API endpoints répondent oui/non (connected: true/false)
3. **Logs pertinents:** Copier/coller depuis journalctl
5. **Dernière action prise:** Quelle commande exécutée

Cela permettra de diagnostiquer rapidement.

---

## 🆘 COMMANDES DE DÉPANNAGE RAPIDE

```bash
# Redémarrer tout
sudo systemctl restart daly-bms
docker restart mosquitto
sleep 5

# Vérifier MQTT
timeout 10 docker exec mosquitto mosquitto_sub -h localhost -t 'santuario/#' -v

# Vérifier API
for ep in inverter smartshunt mppt temperatures; do
  echo "=== $ep ==="
  curl -s http://localhost:8080/api/v1/venus/$ep | jq '.connected'
done

# Vérifier logs
journalctl -u daly-bms -n 30 | grep -E "Updated|error|Error|MQTT"
```

---

## 📞 SI TOUJOURS BLOQUÉ

Fournir exactement:
1. Résultat étape 4 (MQTT topics)
2. Résultat étape 5 (API endpoints)
3. Output de: `journalctl -u daly-bms -n 50`
4. Output de: `