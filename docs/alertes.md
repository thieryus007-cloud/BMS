# AlertEngine — Moteur d'alertes natif Rust — Daly-BMS-Rust

> Moteur d'alertes intégré à `daly-bms-server` : évaluation de règles BMS avec hysteresis,
> durée minimale de déclenchement, journal persistant SQLite, notifications Telegram, API REST.
> Remplace entièrement vmalert — aucune dépendance externe.
> Fait partie de l'[architecture documentaire](./ARCHITECTURE.md).
> Dernière consolidation : 2026-06-07.

## Table des matières

- [1. Vue d'ensemble](#1-vue-densemble)
- [2. Architecture](#2-architecture)
  - [2.1 Flux de données](#21-flux-de-données)
  - [2.2 Partage entre évaluation et API REST](#22-partage-entre-évaluation-et-api-rest)
- [3. Les 9 règles actives](#3-les-9-règles-actives)
  - [3.1 Règles cellule BMS](#31-règles-cellule-bms)
  - [3.2 Règles ESS niveau pack](#32-règles-ess-niveau-pack)
  - [3.3 Source courant : SmartShunt vs BMS interne](#33-source-courant--smartshunt-vs-bms-interne)
  - [3.4 Diagramme de séquence — déclenchement avec durée minimale](#34-diagramme-de-séquence--déclenchement-avec-durée-minimale)
- [4. Hysteresis et cooldown](#4-hysteresis-et-cooldown)
  - [4.1 Principe général](#41-principe-général)
  - [4.2 Tableau récapitulatif hysteresis et cooldown par règle](#42-tableau-récapitulatif-hysteresis-et-cooldown-par-règle)
- [5. Configuration](#5-configuration)
  - [5.1 Section `[alerts]` dans Config.toml](#51-section-alerts-dans-configtoml)
  - [5.2 Section `[alerts.thresholds]`](#52-section-alertsthresholds)
  - [5.3 Valeurs par défaut (section absente)](#53-valeurs-par-défaut-section-absente)
  - [5.4 Désactivation](#54-désactivation)
- [6. Persistance SQLite](#6-persistance-sqlite)
  - [6.1 Schéma `alert_events`](#61-schéma-alert_events)
  - [6.2 Index](#62-index)
  - [6.3 Migration backward-compatible](#63-migration-backward-compatible)
- [7. Notifications Telegram](#7-notifications-telegram)
  - [7.1 Activation](#71-activation)
  - [7.2 Format des messages](#72-format-des-messages)
  - [7.3 Cooldown et anti-spam](#73-cooldown-et-anti-spam)
- [8. Notifications SMTP (email)](#8-notifications-smtp-email)
  - [8.1 Configuration](#81-configuration)
  - [8.2 Statut d'implémentation](#82-statut-dimplémentation)
- [9. API REST alertes](#9-api-rest-alertes)
  - [9.1 `GET /api/v1/alerts/list`](#91-get-apiv1alertslist)
  - [9.2 `GET /api/v1/alerts/stats`](#92-get-apiv1alertsstats)
  - [9.3 `POST /api/v1/alerts/:id/acknowledge`](#93-post-apiv1alertsidacknowledge)
  - [9.4 Comportement si AlertEngine désactivé](#94-comportement-si-alertengine-désactivé)
- [10. Dashboard web `/dashboard/alerts`](#10-dashboard-web-dashboardalerts)
- [11. Intégration dans le code source](#11-intégration-dans-le-code-source)
  - [11.1 `main.rs` — création et démarrage](#111-mainrs--création-et-démarrage)
  - [11.2 `state.rs` — champ AppState](#112-staters--champ-appstate)
  - [11.3 API — pattern `spawn_blocking`](#113-api--pattern-spawn_blocking)
  - [11.4 Tâche background `run_alert_engine`](#114-tâche-background-run_alert_engine)
- [12. Ajouter ou modifier une règle](#12-ajouter-ou-modifier-une-règle)
  - [12.1 Structure d'une `AlertRule`](#121-structure-dune-alertrule)
  - [12.2 Contexte disponible dans les closures](#122-contexte-disponible-dans-les-closures)
  - [12.3 Ajouter un seuil configurable](#123-ajouter-un-seuil-configurable)
- [13. Fichiers concernés](#13-fichiers-concernés)
- [Voir aussi](#voir-aussi)
- [Sources consolidées](#sources-consolidées)

---

## 1. Vue d'ensemble

`AlertEngine` est le moteur d'alertes intégré à `daly-bms-server`. Il remplace entièrement **vmalert**
(ancienne solution externe) sans aucune dépendance externe. Le moteur évalue des règles métier sur
chaque snapshot BMS reçu via le bus broadcast interne (`AppBus`), journalise les événements dans
SQLite et envoie des notifications Telegram.

**Statut : déployé en production** (depuis mai 2026).

| Fonction | Détail |
|----------|--------|
| **9 règles actives** | Cellules, pack, SOC, température, courant |
| **`for:` duration** | Condition doit rester vraie X secondes avant déclenchement |
| **Hysteresis** | Seuil de réarmement distinct du seuil de déclenchement |
| **Cooldown par règle** | Évite le spam de notifications |
| **Journal SQLite** | Tous les événements `triggered`/`cleared` persistés via `rusqlite` |
| **Acquittement** | Via dashboard ou API REST `POST /api/v1/alerts/:id/acknowledge` |
| **Telegram** | Notification push à chaque déclenchement/effacement |
| **Source SmartShunt** | Courant Victron SmartShunt prioritaire sur courant BMS interne pour la règle `high_current` |

---

## 2. Architecture

### 2.1 Flux de données

```
BmsSnapshot broadcast (AppBus)
        │
        ▼
run_alert_engine()          ← tâche Tokio background
        │  lit shunt_current_a depuis AppState.venus_smartshunt
        │
        ▼
AlertEngine::evaluate()     ← évalue chaque règle sur chaque BMS
        │
        ├─ RuleState (en mémoire, HashMap keyed sur (bms_address, rule_id))
        │     ├─ active: bool
        │     ├─ last_notified: Option<Instant>   ← cooldown
        │     └─ pending_since: Option<Instant>   ← timer "for:"
        │
        ├─ SQLite (alert_events)                  ← journal persisté
        │
        └─ Telegram (optionnel)
```

La boucle d'évaluation reçoit chaque lot de snapshots via `state.subscribe_ws()` (canal broadcast
Tokio). Pour chaque snapshot, elle lit le courant SmartShunt depuis `AppState.venus_smartshunt_get()`
une seule fois pour tous les BMS du lot, puis appelle `engine.evaluate()`.

### 2.2 Partage entre évaluation et API REST

`Arc<AlertEngine>` est partagé entre :
- la tâche background `run_alert_engine` (évaluation continue)
- `AppState.alert_engine` (accès API REST via `spawn_blocking`, car `rusqlite` est synchrone)

L'état en mémoire (`HashMap<(u8, &'static str), RuleState>`) est protégé par un `Mutex` standard.
La connexion SQLite est également protégée par un `Mutex` distinct.

---

## 3. Les 9 règles actives

### 3.1 Règles cellule BMS

| ID | Description | Sévérité | Source champ | Seuil trigger | Seuil clear | Durée min | Cooldown |
|----|-------------|----------|--------------|---------------|-------------|-----------|----------|
| `cell_ovp` | Sur-tension cellule | Critical | `snap.system.max_cell_voltage` | > 3.60 V | < 3.55 V (−50 mV) | immédiat | 5 min |
| `cell_uvp` | Sous-tension cellule | Critical | `snap.system.min_cell_voltage` | < 2.90 V | > 2.95 V (+50 mV) | immédiat | 5 min |
| `cell_imbalance` | Déséquilibre cellules | Warning | `snap.system.cell_delta_mv()` | > 100 mV | < 90 mV (−10 mV) | immédiat | 10 min |

### 3.2 Règles ESS niveau pack

| ID | Description | Sévérité | Source champ | Seuil trigger | Seuil clear | Durée min | Cooldown |
|----|-------------|----------|--------------|---------------|-------------|-----------|----------|
| `soc_low` | SOC bas | Warning | `snap.soc` | < 20 % | > 25 % (+5 %) | 5 min | 15 min |
| `soc_critical` | SOC critique | Critical | `snap.soc` | < 15 % | > 17 % (+2 %) | 3 min | 5 min |
| `temp_high` | Sur-température batterie | Critical | `snap.system.max_cell_temperature` | > 45 °C | < 43 °C (−2 °C) | 3 min | 5 min |
| `high_current` | Sur-courant décharge | Warning | SmartShunt¹ ou `snap.dc.current` | > 100 A | < 95 A (−5 A) | 2 min | 1 min |
| `pack_ovp` | Tension pack trop haute | Critical | `snap.dc.voltage` | > 57.0 V | < 56.5 V (−0.5 V) | 1 min | 5 min |
| `pack_uvp` | Tension pack trop basse | Critical | `snap.dc.voltage` | < 44.0 V | > 44.5 V (+0.5 V) | 1 min | 5 min |

> ¹ La règle `high_current` utilise `|shunt_current_a|` du Victron SmartShunt en priorité.
> Si la valeur SmartShunt n'est pas disponible (`None`), elle se rabat sur `|snap.dc.current|`
> (courant BMS interne). Ce comportement respecte la règle de priorisation des mesures Victron
> (CLAUDE.md règle 13).

> **Note sur les seuils de production** : les valeurs ci-dessus sont celles du `Config.toml`
> de production (`/etc/daly-bms/config.toml`). Les valeurs par défaut du code (section absente)
> diffèrent pour certaines règles — voir [§5.3](#53-valeurs-par-défaut-section-absente).

### 3.3 Source courant : SmartShunt vs BMS interne

La règle `high_current` illustre la priorité des mesures Victron sur les mesures BMS internes :

```rust
let c = ctx.shunt_current_a
    .map(|v| v.abs())
    .unwrap_or_else(|| ctx.snap.dc.current.abs());
if c > ctx.cfg.thresholds.current_high_a { Some(c) } else { None }
```

Le courant SmartShunt est lu depuis `AppState.venus_smartshunt_get().await` (cache MQTT Victron).
Sa valeur est en ampères, signée (positif = charge, négatif = décharge) — la valeur absolue est
utilisée pour la comparaison.

### 3.4 Diagramme de séquence — déclenchement avec durée minimale

```
Temps →
Condition vraie :  ────────────────────────────────────────────────────
                   t0               t0+for_secs        t0+for_secs+ε
                                    │
pending_since = t0                  │ elapsed ≥ for_secs → déclencher
                                    │ state.active = true
                                    │ state.pending_since = None
                                    │ log_event("triggered")
                                    │ Telegram ← notif

Condition redevient fausse avant t0+for_secs :
                   pending_since = None (timer réinitialisé)
                   ← pas de déclenchement

Règles sans durée minimale (cell_ovp, cell_uvp, cell_imbalance) :
                   déclenche dès t0 (min_duration = None)
```

---

## 4. Hysteresis et cooldown

### 4.1 Principe général

L'hysteresis évite le flapping (oscillation rapide déclenchement/effacement) autour du seuil.
Le seuil de **déclenchement** (`trigger`) et le seuil de **réarmement** (`clear`) sont distincts :

- Pour se **déclencher** : la valeur doit franchir le seuil trigger.
- Pour s'**effacer** : la valeur doit repasser du côté opposé avec une marge définie (hysteresis).

Exemple pour `cell_ovp` :
- Déclenchement : `max_cell_voltage > 3.60 V`
- Effacement : `max_cell_voltage < 3.55 V` (hysteresis 50 mV)

Le **cooldown** contrôle la fréquence des notifications Telegram : même si une règle reste active,
une nouvelle notification ne sera envoyée qu'après expiration du cooldown depuis la dernière envoyée.
L'état `active` en mémoire et le journal SQLite ne sont pas affectés par le cooldown.

### 4.2 Tableau récapitulatif hysteresis et cooldown par règle

| Règle | Seuil trigger | Hysteresis seuil clear | Cooldown notif |
|-------|---------------|------------------------|----------------|
| `cell_ovp` | > 3.60 V | −50 mV (< 3.55 V) | 5 min |
| `cell_uvp` | < 2.90 V | +50 mV (> 2.95 V) | 5 min |
| `cell_imbalance` | > 100 mV | −10 mV (< 90 mV) | 10 min |
| `soc_low` | < 20 % | +5 % (> 25 %) | 15 min |
| `soc_critical` | < 15 % | +2 % (> 17 %) | 5 min |
| `temp_high` | > 45 °C | −2 °C (< 43 °C) | 5 min |
| `high_current` | > 100 A | −5 A (< 95 A) | 1 min |
| `pack_ovp` | > 57.0 V | −0.5 V (< 56.5 V) | 5 min |
| `pack_uvp` | < 44.0 V | +0.5 V (> 44.5 V) | 5 min |

---

## 5. Configuration

### 5.1 Section `[alerts]` dans Config.toml

```toml
[alerts]
# Chemin de la base SQLite pour journal des alertes.
# Laisser vide pour désactiver complètement les alertes.
db_path = "/var/lib/daly-bms/alerts.db"

# Intervalle d'évaluation des règles (secondes)
check_interval_sec = 1.0

# Telegram (optionnel)
telegram_token   = ""
telegram_chat_id = ""

# Email SMTP (optionnel)
smtp_host     = ""
smtp_port     = 587
smtp_username = ""
smtp_password = ""
smtp_from     = "dalybms@santuario.local"
smtp_to       = "admin@santuario.local"

# Durées minimales "for:" avant déclenchement (secondes).
# 0 = déclencher immédiatement (comportement legacy).
soc_critical_for_secs = 180   # 3 minutes
soc_low_for_secs      = 300   # 5 minutes
temp_high_for_secs    = 180   # 3 minutes
current_high_for_secs = 120   # 2 minutes
voltage_for_secs      = 60    # 1 minute
```

### 5.2 Section `[alerts.thresholds]`

```toml
# Seuils d'alertes logicielles (indépendants des seuils hardware BMS)
[alerts.thresholds]
cell_ovp_v           = 3.60
cell_uvp_v           = 2.90
cell_delta_mv        = 100
soc_low_percent      = 20.0
soc_critical_percent = 15.0   # SOC critique ESS (valeur de production)
temp_high_c          = 45.0
current_high_a       = 100.0  # Courant décharge max via SmartShunt
pack_ovp_v           = 57.0   # Tension pack trop haute (V)
pack_uvp_v           = 44.0   # Tension pack trop basse (V)
```

### 5.3 Valeurs par défaut (section absente)

Si la section `[alerts]` ou `[alerts.thresholds]` est absente de `Config.toml`, les valeurs
suivantes sont utilisées (définies dans `config.rs`, constantes préfixées `DEFAULT_`) :

| Champ | Défaut code | Valeur production | Unité |
|-------|-------------|-------------------|-------|
| `cell_ovp_v` | 3.60 | 3.60 | V |
| `cell_uvp_v` | 2.90 | 2.90 | V |
| `cell_delta_mv` | 100.0 | 100 | mV |
| `soc_low_percent` | 20.0 | 20.0 | % |
| `soc_critical_percent` | **10.0** | **15.0** | % |
| `temp_high_c` | 45.0 | 45.0 | °C |
| `current_high_a` | **80.0** | **100.0** | A |
| `pack_ovp_v` | 57.0 | 57.0 | V |
| `pack_uvp_v` | 44.0 | 44.0 | V |
| `*_for_secs` | 0 | (voir §5.1) | s (immédiat) |

> **Divergence notable** : deux seuils divergent entre les défauts code et la configuration de
> production :
> - `soc_critical_percent` : défaut code = **10.0 %**, production = **15.0 %** (aligné avec la
>   stratégie ESS ; la valeur 10.0 % correspond à l'ancienne configuration vmalert).
> - `current_high_a` : défaut code = **80.0 A** (BMS seul), production = **100.0 A** (SmartShunt).
>
> La valeur `current_high_a = 100.0 A` de production est volontaire : le SmartShunt mesure le
> courant total batterie (plus précis que le courant interne BMS), ce qui justifie un seuil plus
> élevé.

> **Divergence avec Readme.md** : le Readme.md mentionne les seuils suivants dans sa table
> « Alertes configurables » :
> - `soc_critical < 10%` / hysteresis `+2%`
> - `high_current > 80 A` / hysteresis `−5 A`
>
> Ces valeurs correspondent aux **défauts code** (avant configuration de production), non aux
> valeurs actives dans `Config.toml`. La configuration de production (`/etc/daly-bms/config.toml`)
> fait autorité : `soc_critical = 15 %`, `current_high_a = 100 A`.

### 5.4 Désactivation

Pour désactiver complètement l'AlertEngine, laisser `db_path` vide :

```toml
[alerts]
db_path = ""
```

Le service démarre normalement ; les routes API retournent `503 Service Unavailable`.
Le dashboard `/dashboard/alerts` affiche le message « AlertEngine désactivé ».

---

## 6. Persistance SQLite

La base SQLite est ouverte via `rusqlite` à l'initialisation du moteur. Le chemin est configurable
(`db_path`). Le répertoire parent est créé automatiquement par `main.rs` si absent.

### 6.1 Schéma `alert_events`

```sql
CREATE TABLE IF NOT EXISTS alert_events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    bms_address     INTEGER NOT NULL,
    rule_id         TEXT    NOT NULL,
    description     TEXT    NOT NULL DEFAULT '',
    severity        TEXT    NOT NULL DEFAULT 'warning',
    event           TEXT    NOT NULL,          -- 'triggered' | 'cleared'
    value           REAL    NOT NULL,
    timestamp       TEXT    NOT NULL,          -- datetime('now') UTC
    acknowledged    INTEGER NOT NULL DEFAULT 0,
    acknowledged_at TEXT                       -- NULL si non acquittée
);
```

### 6.2 Index

```sql
CREATE INDEX IF NOT EXISTS idx_alert_ts
    ON alert_events(timestamp);
CREATE INDEX IF NOT EXISTS idx_alert_bms_rule
    ON alert_events(bms_address, rule_id);
CREATE INDEX IF NOT EXISTS idx_alert_event
    ON alert_events(event);
CREATE INDEX IF NOT EXISTS idx_alert_ack
    ON alert_events(acknowledged);
```

### 6.3 Migration backward-compatible

Au démarrage, `init_db()` tente silencieusement d'ajouter les colonnes absentes sur une base
existante. Les erreurs `duplicate column` sont ignorées — une base ancienne est migrée sans perte
de données :

```sql
ALTER TABLE alert_events ADD COLUMN description     TEXT    NOT NULL DEFAULT '';
ALTER TABLE alert_events ADD COLUMN severity        TEXT    NOT NULL DEFAULT 'warning';
ALTER TABLE alert_events ADD COLUMN acknowledged    INTEGER NOT NULL DEFAULT 0;
ALTER TABLE alert_events ADD COLUMN acknowledged_at TEXT;
```

Cette approche garantit la compatibilité avec les bases créées avant l'ajout des colonnes
`description`, `severity` et `acknowledged`.

---

## 7. Notifications Telegram

### 7.1 Activation

Renseigner `telegram_token` et `telegram_chat_id` dans la section `[alerts]` de `Config.toml` :

```toml
telegram_token   = "<votre-token-bot>"
telegram_chat_id = "<votre-chat-id>"
```

Si l'un des deux champs est vide, la notification Telegram est silencieusement ignorée (le journal
SQLite est toujours mis à jour).

### 7.2 Format des messages

Message de déclenchement :

```
🔴 Alerte DÉCLENCHÉE — BMS 0x01
Règle : SOC critique
Valeur : 13.50
Sévérité : critical
```

Message d'effacement :

```
✅ Alerte EFFACÉE — BMS 0x01
Règle : SOC critique
Valeur : 0.00
Sévérité : critical
```

Le message est envoyé via l'API Telegram Bot (`https://api.telegram.org/bot{token}/sendMessage`)
avec `parse_mode: "HTML"`. L'adresse BMS est affichée en hexadécimal (ex. `0x01`).

### 7.3 Cooldown et anti-spam

Les notifications respectent le cooldown défini par règle (voir [§4.2](#42-tableau-récapitulatif-hysteresis-et-cooldown-par-règle)).
L'instant de la dernière notification est conservé dans `RuleState.last_notified` (en mémoire,
réinitialisé au redémarrage du service). Le journal SQLite n'est pas affecté par le cooldown :
tous les événements sont journalisés indépendamment.

---

## 8. Notifications SMTP (email)

### 8.1 Configuration

Les champs SMTP sont présents dans `AlertsConfig` et dans `Config.toml` :

```toml
smtp_host     = ""
smtp_port     = 587
smtp_username = ""
smtp_password = ""
smtp_from     = "dalybms@santuario.local"
smtp_to       = "admin@santuario.local"
```

### 8.2 Statut d'implémentation

> **Note d'implémentation** : le fichier source `bridges/alerts.rs` porte le commentaire
> « notifications Telegram/SMTP » en en-tête de module, et les champs SMTP sont déclarés dans
> `AlertsConfig`. Cependant, la fonction d'envoi SMTP n'est **pas encore implémentée** dans le
> code actuel (seule `send_telegram()` est présente). Les champs SMTP sont disponibles pour une
> implémentation future. En production, seules les notifications Telegram et le journal SQLite
> sont actifs.

---

## 9. API REST alertes

Toutes les routes nécessitent que `db_path` soit configuré (AlertEngine actif).
Si l'AlertEngine est désactivé, toutes les routes retournent `503 Service Unavailable`.

### 9.1 `GET /api/v1/alerts/list`

Liste les événements d'alerte avec pagination et filtres optionnels.

**Paramètres de requête :**

| Param | Type | Défaut | Description |
|-------|------|--------|-------------|
| `limit` | usize | 100 | Nombre max d'événements retournés (maximum : 500) |
| `offset` | usize | 0 | Décalage pour la pagination |
| `severity` | string | — | Filtrer par sévérité : `critical` ou `warning` |
| `active` | bool | false | `true` = événements `triggered` seulement (alertes actives) |

**Exemple de requête :**

```
GET /api/v1/alerts/list?limit=50&offset=0&severity=critical&active=true
```

**Réponse (200 OK) :**

```json
{
  "total": 42,
  "events": [
    {
      "id": 17,
      "bms_address": 1,
      "rule_id": "soc_critical",
      "description": "SOC critique",
      "severity": "critical",
      "event": "triggered",
      "value": 13.5,
      "timestamp": "2026-05-01 14:32:11",
      "acknowledged": false,
      "acknowledged_at": null
    }
  ]
}
```

**Tri** : anti-chronologique (`ORDER BY timestamp DESC`).

### 9.2 `GET /api/v1/alerts/stats`

Retourne les compteurs agrégés du journal.

**Réponse (200 OK) :**

```json
{
  "total": 142,
  "triggered": 3,
  "cleared": 139,
  "critical": 78,
  "warning": 64,
  "unacknowledged": 2
}
```

**Détail des compteurs :**

| Champ | Description |
|-------|-------------|
| `total` | Nombre total d'événements dans la base |
| `triggered` | Événements de type `triggered` (déclenchements) |
| `cleared` | Événements de type `cleared` (effacements) |
| `critical` | Événements de sévérité `critical` |
| `warning` | Événements de sévérité `warning` |
| `unacknowledged` | Événements `triggered` non acquittés (`acknowledged = 0`) |

### 9.3 `POST /api/v1/alerts/:id/acknowledge`

Acquitte l'alerte identifiée par son `id` SQLite. Met à jour `acknowledged = 1` et
`acknowledged_at = datetime('now')` UTC.

**Réponses :**
- `200 OK` — alerte acquittée avec succès.
- `404 Not Found` — aucun enregistrement avec cet `id`.
- `503 Service Unavailable` — AlertEngine désactivé.

### 9.4 Comportement si AlertEngine désactivé

```rust
let engine = state.alert_engine.as_ref()
    .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
    .clone();
```

Toutes les routes API vérifient la présence du moteur avant d'appeler SQLite.
Le pattern `spawn_blocking` est utilisé car `rusqlite` est synchrone :

```rust
let result = spawn_blocking(move || engine.query_events(limit, offset, only_active, severity))
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)??;
```

---

## 10. Dashboard web `/dashboard/alerts`

URL : `http://<Pi5>:8080/dashboard/alerts`

Lien dans la sidebar : **Alertes** (section Système, icône cloche).

**Fonctionnalités :**

- **Stats bar** (4 tuiles) :
  - Actives (rouge)
  - Non acquittées (orange)
  - Critiques total (rouge foncé)
  - Total journalisés (gris)
- **Filtres** :
  - Sévérité : Toutes / Critique / Warning
  - Statut : Tous / Actives seulement
  - Bouton « Réinitialiser »
- **Liste paginée** (50 événements/page) avec tri anti-chronologique
- **Bouton Acquitter** par événement (affiché si `event = 'triggered'` et non encore acquitté)
- **Auto-refresh** toutes les 30 secondes
- **Indicateur de statut** (badge topbar) :
  - Rouge : N alerte(s) active(s)
  - Vert : Aucune alerte active

**État désactivé** : si `alerts.db_path` est vide dans `Config.toml`, le dashboard affiche
un encart « AlertEngine désactivé » avec un lien vers la section de configuration.

Le template Askama est compilé dans le binaire (`templates/alerts.html`). Tout changement au
template nécessite `make build-arm` et un redéploiement du binaire.

---

## 11. Intégration dans le code source

### 11.1 `main.rs` — création et démarrage

```rust
let alert_engine = if !config.alerts.db_path.is_empty() {
    std::fs::create_dir_all(parent)?;
    match alerts::AlertEngine::new(config.alerts.clone()) {
        Ok(e)  => Some(e),
        Err(e) => { warn!("AlertEngine: {}", e); None }
    }
} else { None };

let state = AppState::new(config.clone(), log_buffer, vm_handle, alert_engine.clone());

if let Some(ref engine) = alert_engine {
    tokio::spawn(alerts::run_alert_engine(state.clone(), engine.clone()));
}
```

Si la création de l'AlertEngine échoue (ex. SQLite inaccessible), le serveur démarre malgré tout
avec `alert_engine = None` (dégradation gracieuse).

### 11.2 `state.rs` — champ AppState

```rust
pub alert_engine: Option<Arc<AlertEngine>>,
```

### 11.3 API — pattern `spawn_blocking`

`rusqlite` est une bibliothèque synchrone (bloquante). Toutes les opérations SQLite depuis les
handlers Axum (async) passent par `spawn_blocking` pour ne pas bloquer le runtime Tokio :

```rust
let engine = state.alert_engine.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?.clone();
let result = spawn_blocking(move || engine.query_events(limit, offset, only_active, severity))
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)??;
```

### 11.4 Tâche background `run_alert_engine`

```rust
pub async fn run_alert_engine(state: AppState, engine: Arc<AlertEngine>) {
    info!(db = %engine.cfg.db_path, "AlertEngine démarré");

    let mut rx = state.subscribe_ws();
    loop {
        match rx.recv().await {
            Ok(snaps) => {
                // Lire le courant SmartShunt une seule fois pour tous les BMS
                let shunt_current_a = state.venus_smartshunt_get().await
                    .and_then(|s| s.current_a);

                for snap in snaps.iter() {
                    engine.evaluate(snap, shunt_current_a).await;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                warn!("AlertEngine : {} snapshots manqués", n);
            }
            Err(_) => break,
        }
    }
}
```

La tâche se termine proprement si le canal broadcast est fermé (arrêt du serveur). Les snapshots
manqués (`Lagged`) sont journalisés en `warn` mais ne provoquent pas d'arrêt.

---

## 12. Ajouter ou modifier une règle

Toutes les règles sont définies dans la fonction `build_rules()` dans
`crates/daly-bms-server/src/bridges/alerts.rs`.

### 12.1 Structure d'une `AlertRule`

```rust
AlertRule {
    id:           "ma_regle",           // identifiant unique, snake_case
    description:  "Description lisible", // affiché dans le dashboard et les notifs
    severity:     Severity::Warning,    // ou Severity::Critical
    cooldown:     Duration::from_secs(300),
    min_duration: dur_opt(cfg.xxx_for_secs), // None = immédiat (0 s dans le config)
    trigger: Box::new(|ctx| {
        // Retourner Some(valeur) si la règle doit se déclencher
        let v = ctx.snap.dc.voltage;
        if v > ctx.cfg.thresholds.ma_valeur { Some(v) } else { None }
    }),
    clear: Box::new(|ctx| {
        // Retourner true si la règle doit s'effacer (intègre l'hysteresis)
        ctx.snap.dc.voltage < ctx.cfg.thresholds.ma_valeur - 0.5
    }),
}
```

La fonction utilitaire `dur_opt(secs: u64) -> Option<Duration>` retourne `None` si `secs == 0`
(déclenchement immédiat, comportement legacy) et `Some(Duration::from_secs(secs))` sinon.

### 12.2 Contexte disponible dans les closures

| Champ | Type | Description |
|-------|------|-------------|
| `ctx.snap` | `&BmsSnapshot` | Snapshot complet du BMS |
| `ctx.snap.soc` | `f32` | SOC en % |
| `ctx.snap.dc.voltage` | `f32` | Tension pack (V) |
| `ctx.snap.dc.current` | `f32` | Courant BMS interne (A, + = charge) |
| `ctx.snap.system.max_cell_voltage` | `f32` | Tension max cellule (V) |
| `ctx.snap.system.min_cell_voltage` | `f32` | Tension min cellule (V) |
| `ctx.snap.system.max_cell_temperature` | `f32` | Température max (°C) |
| `ctx.snap.system.cell_delta_mv()` | `f32` | Delta cellules (mV) |
| `ctx.cfg.thresholds.*` | `f32` | Seuils depuis Config.toml |
| `ctx.shunt_current_a` | `Option<f32>` | Courant SmartShunt (A, signé) |

### 12.3 Ajouter un seuil configurable

1. Ajouter le champ dans `AlertThresholds` (`config.rs`) avec sa valeur par défaut.
2. Déclarer la valeur dans `Config.toml` (section `[alerts.thresholds]`).
3. Utiliser `ctx.cfg.thresholds.mon_seuil` dans la closure `trigger` ou `clear`.
4. Ajouter éventuellement un champ `mon_seuil_for_secs` dans `AlertsConfig` pour la durée
   minimale, et passer `dur_opt(cfg.mon_seuil_for_secs)` à `min_duration`.

---

## 13. Fichiers concernés

| Fichier | Rôle |
|---------|------|
| `crates/daly-bms-server/src/bridges/alerts.rs` | Moteur complet : `AlertEngine`, `AlertRule`, `RuleState`, `build_rules()`, `run_alert_engine()`, journal SQLite (`init_db()`, `log_event()`), notification Telegram (`send_telegram()`) |
| `crates/daly-bms-server/src/config.rs` | `AlertsConfig` (db_path, telegram, smtp, for_secs), `AlertThresholds` (9 seuils), constantes `DEFAULT_*`, `impl Default for AlertThresholds` |
| `crates/daly-bms-server/src/state.rs` | Champ `alert_engine: Option<Arc<AlertEngine>>` dans `AppState` |
| `crates/daly-bms-server/src/main.rs` | Création de l'AlertEngine, création du répertoire SQLite, `tokio::spawn(run_alert_engine(...))` |
| `crates/daly-bms-server/src/api/alerts.rs` | Handlers REST : `list_alerts`, `get_stats`, `acknowledge_alert` ; types `AlertListResponse`, `AlertListQuery` |
| `crates/daly-bms-server/src/api/mod.rs` | Routes `/api/v1/alerts/list`, `/api/v1/alerts/stats`, `/api/v1/alerts/:id/acknowledge` |
| `crates/daly-bms-server/src/dashboard/mod.rs` | Route `/dashboard/alerts` (rendu SSR Askama) |
| `crates/daly-bms-server/templates/alerts.html` | Template Askama du dashboard alertes (compilé dans le binaire) |
| `Config.toml` | Sections `[alerts]` et `[alerts.thresholds]` (configuration Pi5 production) |

---

## Voir aussi

- [./app-daly-bms-server.md](./app-daly-bms-server.md) — Architecture interne globale de
  `daly-bms-server` : AppState, ring buffer, bridges, API REST/WS complète.
- [./grafana-dashboards.md](./grafana-dashboards.md) — Dashboards Grafana avancés 17→20 incluant
  les alertes multi-critères PromQL (dashboard `20-alertes-avancees.json`).
- [./metriques-promql-reference.md](./metriques-promql-reference.md) — Catalogue des métriques et
  interface PromQL ; les métriques BMS (SOC, tension, courant, delta) sont les sources des règles
  d'alerte.
- [./diagnostic-depannage.md](./diagnostic-depannage.md) — Dépannage transverse : problèmes
  courants du service `daly-bms`.

---

## Sources consolidées

Ce document fusionne et **remplace** l'ancien fichier suivant :
`docs/Intégration-AlertEngine.md`
