 Je vais examiner le code source de ce projet Rust pour identifier les fuites mémoire potentielles.
 L'URL retournée est la page HTML de GitHub. Je vais chercher les fichiers source directement
  Je vais essayer d'accéder directement aux fichiers source du repository.  
   Je dois examiner les modules critiques qui pourraient causer des fuites mémoire : `state.rs`, `api.rs`, `bridges/mqtt.rs`, `monitor.rs`, et le module `daly-bms-core`.    
     Je dois examiner les modules `console.rs`, `vm_client.rs`, et le crate `daly-bms-core` pour identifier d'autres sources potentielles de fuites.    
      Maintenant je vais examiner les fichiers API pour voir les handlers WebSocket et les routes HTTP qui pourraient accumuler des données.      
        Je dois examiner le fichier WebSocket BMS pour voir comment les connexions sont gérées.         
J'ai maintenant suffisamment d'informations pour analyser la fuite mémoire. Voici mon diagnostic :

---

## 🔴 Diagnostic : Fuite mémoire dans `daly-bms-server`

### Cause principale identifiée : **Accumulation de tâches `tokio::spawn` orphelines**

#### Localisation : `main.rs`, lignes ~320-450

Le problème majeur se situe dans la boucle de polling et les callbacks de snapshot. Voici le pattern problématique :

```rust
// Dans main.rs — BMS polling
tokio::spawn(async move {
    poll_loop(
        manager,
        poll_cfg,
        move |snap| {
            let s = state_poll.clone();
            let addr = snap.address;
            let name = snap.name.clone();
            tokio::spawn(async move {   // ← 🚨 SPAWN ORPHELIN #1
                s.record_rs485_success(addr, "BMS", &name).await;
                s.on_snapshot(snap).await;  // ← appelle console_bus.emit() + ws_tx.send()
            });
        },
        move |addr, kind, msg| {
            let s = state_err.clone();
            let name = bms_names.get(&addr).cloned()...;
            tokio::spawn(async move {   // ← 🚨 SPAWN ORPHELIN #2
                s.record_rs485_error(addr, "BMS", &name, &err_msg).await;
            });
        },
    ).await;
});
```

**Le même pattern se répète pour ET112, PRALRAN, ATS, Tasmota, Shelly** — à chaque snapshot ou erreur, une nouvelle tâche est spawnée sans aucune limite.

### Pourquoi c'est une fuite :

1. **Pas de `JoinHandle` conservé** — les `tokio::spawn` retournent des `JoinHandle` qui sont immédiatement dropées. Tokio garde la tâche en mémoire jusqu'à completion, mais sans handle, rien ne la nettoie si elle bloque.

2. **Back-pressure nulle** — Si VM est lent ou indisponible, `vm.write_rows(rows).await` peut bloquer longtemps. Pendant ce temps, de nouvelles tâches s'accumulent dans la queue d'exécution Tokio.

3. **`console_bus.emit()` avec broadcast saturé** — dans `console.rs` :
   ```rust
   pub fn emit(&self, ev: ConsoleEvent) {
       let _ = self.tx.send(Arc::new(ev));  // ← drop silencieux si buffer plein
   }
   ```
   Le `let _ =` masque les erreurs `Lagged` du broadcast. Si aucun subscriber WebSocket console n'est connecté, les 512 messages s'accumulent... mais le `Arc` des événements reste en mémoire jusqu'à ce que le channel soit drainé.

4. **WebSocket BMS (`ws_tx.send`)** — dans `state.rs` :
   ```rust
   let _ = self.ws_tx.send(Arc::new(latest));  // ← drop silencieux si pas de subscriber
   ```
   Idem : si aucun client WebSocket n'est connecté, le broadcast channel de 128 slots se remplit, et les `Arc<BmsSnapshot>` (qui contiennent des `VecDeque`, des `BTreeMap`, etc.) restent en mémoire.

---

### 🔶 Cause secondaire : `metrics_store` dual-write

Dans `vm_client.rs` :
```rust
if let Some(w) = &self.metrics_store {
    let mut dropped = 0_usize;
    for row in &rows {
        if w.try_write(row.to_sample()).is_err() {
            dropped += 1;  // ← samples perdus mais allocations déjà faites
        }
    }
}
```
Le `try_write` échoue si la queue est pleine, mais les `Sample` (qui clonent les strings de labels) sont alloués avant le test. Avec le polling à 1s et 10+ métriques par BMS, ça fait des milliers d'allocations string/jour.

---

### 🔶 Cause tertiaire : `reqwest::Client` sans pool limité

Dans `vm_client.rs` :
```rust
let http = Client::builder()
    .timeout(Duration::from_secs(config.timeout_secs))
    .build()?;
```
Pas de `.pool_max_idle_per_host()` ni de `.pool_idle_timeout()`. Si VM est instable, les connexions HTTP peuvent s'accumuler dans le pool idle de reqwest.

---

## ✅ Plan de correction

### 1. **Limiter les spawns avec un `Semaphore` ou `tokio::sync::mpsc`**

Remplacer les `tokio::spawn` sauvages par un channel bounded avec un seul worker :

```rust
// Dans main.rs — remplacer les callbacks par un channel
let (snapshot_tx, mut snapshot_rx) = tokio::sync::mpsc::channel::<BmsSnapshot>(32);

tokio::spawn(async move {
    while let Some(snap) = snapshot_rx.recv().await {
        state.on_snapshot(snap).await;
    }
});

// Dans le callback du poll_loop :
move |snap| {
    let _ = snapshot_tx.try_send(snap);  // back-pressure : drop si plein
}
```

### 2. **Limiter la capacité du broadcast console**

```rust
// console.rs
pub const CONSOLE_CAPACITY: usize = 512;  // ← réduire à 64 si pas de subscriber WS
```
Ou mieux : ne pas émettre si aucun subscriber (vérifier `tx.receiver_count()`).

### 3. **Limiter le broadcast WebSocket BMS**

```rust
// state.rs
const WS_BROADCAST_CAPACITY: usize = 128;  // ← réduire à 16 + vérifier receiver_count
```
Et dans `on_snapshot` :
```rust
if self.ws_tx.receiver_count() > 0 {
    let _ = self.ws_tx.send(Arc::new(latest));
}
```

### 4. **Limiter le pool reqwest**

```rust
let http = Client::builder()
    .timeout(Duration::from_secs(config.timeout_secs))
    .pool_max_idle_per_host(2)
    .pool_idle_timeout(Duration::from_secs(30))
    .build()?;
```

### 5. **Éviter les allocations dans le hot path VM**

Pré-allouer le buffer Prometheus dans `VmRow` avec `String::with_capacity` (déjà fait ✓) mais aussi éviter le `.clone()` des labels dans `to_sample()` :

```rust
// vm_client.rs — to_sample() actuel
fn to_sample(&self) -> Sample {
    Sample {
        metric: self.metric.clone(),      // ← clone inutile si on consume pas le VmRow
        labels: self.labels.iter().cloned().collect(),  // ← clone tous les labels
        ...
    }
}
```

---

## 📊 Recommandation immédiate

Pour confirmer la fuite, lance avec :
```bash
# Observer la croissance du nombre de tâches Tokio
RUST_LOG=tokio=trace,daly_bms_server=info ./daly-bms-server

# Ou avec tokio-metrics (déjà intégré !)
# Voir les métriques : tokio_task_polls_total, tokio_task_mean_poll_us
```

Et vérifie si `tokio_task_polls_total` augmente continuellement sans se stabiliser — ce serait la confirmation que des tâches s'accumulent.

**Le fix le plus urgent** : remplacer les 6-7 `tokio::spawn` dans les callbacks de `main.rs` par un `mpsc::channel` avec un seul worker par type de donnée (BMS, ET112, etc.). Cela éliminera la fuite principale.
