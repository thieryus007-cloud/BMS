# Rules:
Si les trois conditions sont réunies, **alors** POST mode: "HEAT_PUMP".
  1- On se trouve en mode Offgrid: AC IN ingnore Actif
  2- Le SOC est supérieure à 90%
  3- l'irradiance est superieure à 300 w/m2
  >> Sinon alors POST mode: "VACATION".  

Attention respécter la notation LG: 'HEAT_PUMP', 'TURBO', 'VACATION' 
---

# 🧾 🧠 PR — Fix reliable mode switching for LG ThinQ water heater

## 🎯 Objectif

Garantir que :

* ✅ un **POST est envoyé quand nécessaire**
* ✅ pas de spam inutile vers LG
* ✅ comportement déterministe (indépendant d’un état local potentiellement faux)
* ✅ robuste aux erreurs LG / reboot / désync

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
