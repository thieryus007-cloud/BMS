# Architecture du bus RS485 unifié

> Différence entre les crates `rs485-bus` et `daly-bms-core` et fonctionnement du
> bus partagé `/dev/ttyUSB0` côté Pi5.

---

## Vue d'ensemble

Sur le Pi5, **un seul port série** (`/dev/ttyUSB0`, 9600 bauds, half-duplex)
porte simultanément le trafic de plusieurs protocoles différents :

| Protocole | Adresses | Driver |
|-----------|----------|--------|
| Daly UART V1.21 (BMS) | 0x01, 0x02 | `daly-bms-core` |
| Modbus RTU (ET112) | 0x07, 0x08, 0x09 | `daly-bms-server::et112` |
| Modbus RTU (PRALRAN irradiance) | 0x05 | `daly-bms-server::irradiance` |
| Modbus RTU (ATS CHINT) | (configurable) | `daly-bms-server::ats` |

L'arbitrage se fait via un **mutex tokio** placé dans le crate `rs485-bus`.
Aucune collision possible sur le bus half-duplex : un seul driver émet à la fois.

---

## Les deux couches

### `rs485-bus` — couche physique (générique, agnostique)

| Aspect | Détail |
|---|---|
| Rôle | Possède le port série et garantit l'accès exclusif au bus |
| Contenu | `SharedBus` (mutex tokio autour de `SerialStream`) + `modbus_rtu` (CRC16, FC03/04/06) |
| Sait parler | **Aucun** protocole spécifique — c'est juste un "tube" + des helpers Modbus génériques |
| Taille | ~440 lignes |
| Dépendances | `tokio-serial`, `tokio` |
| Utilisé par | `daly-bms-core` (BMS Daly), `et112` (compteurs), `irradiance` (PRALRAN), `ats` (CHINT) |

C'est l'arbitre du bus. Un seul `Arc<SharedBus>` est créé pour `/dev/ttyUSB0`.
Tous les drivers acquièrent le mutex à tour de rôle pour transmettre leur trame.

### `daly-bms-core` — protocole Daly (spécifique)

| Aspect | Détail |
|---|---|
| Rôle | Implémente le **protocole UART Daly V1.21** par-dessus `SharedBus` |
| Contenu | `protocol.rs` (framing 13 octets + checksum), `commands.rs` (DataIds 0x90–0x98), `write.rs` (MOS/SOC/reset), `poll.rs` (boucle async + backoff), `types.rs` (`BmsSnapshot`, `Alarms`, …) |
| Sait parler | Uniquement au BMS Daly |
| Taille | ~2 330 lignes |
| Dépendances | `rs485-bus` (re-export `SharedBus`), `serde`, `chrono` |
| Utilisé par | `daly-bms-server` |

`DalyPort` encapsule un `SharedBus` et y ajoute le framing Daly (start byte 0xA5,
adresse 0x40+board, checksum 1 octet, etc.).

---

## Schéma d'empilement

```
┌─────────────────────────────────────────────────────┐
│ daly-bms-server                                     │
│  ├── DalyBusManager  (daly-bms-core)  ──┐           │
│  ├── ET112 driver     (rs485-bus::modbus_rtu) ──┐   │
│  ├── Irradiance       (rs485-bus::modbus_rtu) ──┤   │
│  └── ATS CHINT        (rs485-bus::modbus_rtu) ──┤   │
└──────────────────────────────────────────────┬──┴───┘
                                               ▼
                                  ┌────────────────────┐
                                  │ Arc<SharedBus>     │  ← rs485-bus
                                  │ Mutex<SerialStream>│
                                  └─────────┬──────────┘
                                            ▼
                                     /dev/ttyUSB0
                                     (RS485 half-duplex)
```

---

## Pourquoi cette séparation ?

1. **Réutilisabilité** — `rs485-bus` est protocole-agnostique. On peut y brancher
   un nouveau capteur Modbus demain sans toucher à `daly-bms-core`.
2. **Concurrence sécurisée** — un seul mutex sur le port série évite les
   collisions sur le bus half-duplex (impossible que BMS et ET112 émettent en
   même temps).
3. **Tests isolés** — `daly-bms-core` se teste sans port série réel ;
   `rs485-bus::modbus_rtu` se teste sur des trames CRC en mémoire.
4. **Légèreté** — `daly-bms-core` n'embarque pas la complexité Modbus,
   `rs485-bus` n'embarque pas la complexité Daly.

---

## Le code clé qui les relie

Dans `daly-bms-core/src/bus.rs` :

```rust
let port = DalyPort::open("/dev/ttyUSB0", 9600, 500)?;
let bus  = port.shared_bus();   // ← extrait le Arc<SharedBus> sous-jacent
// passe `bus` à run_et112_poll_loop, run_irradiance_poll_loop, ats…
```

Côté `daly-bms-server/src/main.rs` (mode hardware) :

```rust
let dal_port = DalyPort::open(&resolved_port, config.serial.baud, 500)?;
let shared_bus = dal_port.shared_bus();

// 1. ET112 — bus partagé
tokio::spawn(et112::run_et112_poll_loop(shared_bus.clone(), …));

// 2. PRALRAN irradiance — bus partagé
tokio::spawn(irradiance::run_irradiance_poll_loop(shared_bus.clone(), …));

// 3. ATS CHINT — bus partagé
tokio::spawn(ats::run_ats_poll_loop(shared_bus.clone(), …));

// 4. BMS Daly — utilise dal_port directement (qui possède le même SharedBus)
let manager = Arc::new(DalyBusManager::new(dal_port, devices));
tokio::spawn(poll_loop(manager, …));
```

**Un port, un mutex, plusieurs protocoles.**

---

## Trames Modbus RTU générées par `rs485-bus::modbus_rtu`

| Function code | Usage projet | Helper |
|---------------|--------------|--------|
| FC03 (Read Holding Registers) | ATS CHINT (futur) | `build_fc03(addr, reg, count)` |
| FC04 (Read Input Registers) | ET112, PRALRAN | `build_fc04(addr, reg, count)` |
| FC06 (Write Single Register) | ATS CHINT (futur) | `build_fc06(addr, reg, value)` |

Toutes les trames sont closes par CRC-16/Modbus (polynôme 0xA001, init 0xFFFF,
LSB first) — implémentation pure Rust dans `crc16()`.

---

## Trames Daly UART générées par `daly-bms-core::protocol`

Format fixe 13 octets :

```text
[0xA5][PC_ADDR][DATA_ID][0x08][8 octets data][CHECKSUM]
```

- `PC_ADDR` = `0x40 + board_number` (0x40 pour BMS 1, 0x41 pour BMS 2…)
- `DATA_ID` = 0x90–0x98 (lecture) ou 0x10/0x21/etc. (écriture)
- `CHECKSUM` = somme modulo 256 des 12 octets précédents

Détails complets : `docs/Daly-UART_485-Communications-Protocol-V1.21-1.pdf`.

---

## Voir aussi

- `crates/rs485-bus/src/lib.rs` — implémentation `SharedBus`
- `crates/rs485-bus/src/modbus_rtu.rs` — CRC16, builders FC03/04/06
- `crates/daly-bms-core/src/bus.rs` — `DalyPort` + `DalyBusManager`
- `crates/daly-bms-core/src/protocol.rs` — framing Daly + checksum
- `crates/daly-bms-server/src/main.rs` — branchement de tous les drivers sur le bus partagé
