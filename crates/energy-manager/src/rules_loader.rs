/// Loads .grl rule sources from disk (if configured) with embedded fallback.
///
/// Usage:
///   let loader = Arc::new(RulesLoader::new(cfg.rules.dir.as_deref().map(Path::new)));
///   let src = loader.load("water_heater");
///
/// Hot-reload: trigger via POST /api/v1/em/rules/reload — the endpoint broadcasts
/// a signal via AppBus::rule_reload; each logic module recreates its engine.
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

// Embedded fallbacks compiled into the binary
const EMBEDDED: &[(&str, &str)] = &[
    ("charge_current", include_str!("../rules/charge_current.grl")),
    // NB : la décision DEYE n'utilise plus de moteur de règles — c'est une logique native
    // simple (logic/deye_command/mod.rs). Plus de deye_command.grl.
    ("irradiance",     include_str!("../rules/irradiance.grl")),
    ("smartshunt",     include_str!("../rules/smartshunt.grl")),
    ("solar_power",    include_str!("../rules/solar_power.grl")),
    ("water_heater",   include_str!("../rules/water_heater.grl")),
];

/// Metadata about a loaded rule, exposed via GET /api/v1/em/rules.
#[derive(Debug, Clone, Serialize)]
pub struct RuleInfo {
    pub name:       String,
    pub origin:     String,           // "disk" | "embedded"
    pub path:       Option<String>,   // disk path if origin == "disk"
    pub loaded_at:  DateTime<Utc>,
    pub size_bytes: usize,
}

pub struct RulesLoader {
    dir:  Option<PathBuf>,
    info: Arc<RwLock<HashMap<String, RuleInfo>>>,
}

impl RulesLoader {
    pub fn new(dir: Option<&Path>) -> Self {
        Self {
            dir:  dir.map(PathBuf::from),
            info: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Load a rule source by name. Tries disk first if a dir is configured,
    /// falls back to the embedded constant.
    pub fn load(&self, name: &str) -> String {
        if let Some(dir) = &self.dir {
            let path = dir.join(format!("{name}.grl"));
            if let Ok(src) = std::fs::read_to_string(&path) {
                let info = RuleInfo {
                    name:       name.to_string(),
                    origin:     "disk".to_string(),
                    path:       Some(path.to_string_lossy().into_owned()),
                    loaded_at:  Utc::now(),
                    size_bytes: src.len(),
                };
                if let Ok(mut map) = self.info.write() {
                    map.insert(name.to_string(), info);
                }
                tracing::info!("rules_loader: {name}.grl loaded from {:?} ({} bytes)", path, src.len());
                return src;
            }
            tracing::warn!("rules_loader: {name}.grl not found in {:?} — using embedded fallback", dir);
        }

        let src = Self::embedded(name);
        let info = RuleInfo {
            name:       name.to_string(),
            origin:     "embedded".to_string(),
            path:       None,
            loaded_at:  Utc::now(),
            size_bytes: src.len(),
        };
        if let Ok(mut map) = self.info.write() {
            map.insert(name.to_string(), info);
        }
        src.to_string()
    }

    /// Returns metadata for all rules that have been loaded at least once.
    pub fn info(&self) -> Vec<RuleInfo> {
        let map = self.info.read().unwrap_or_else(|e| e.into_inner());
        let mut list: Vec<RuleInfo> = map.values().cloned().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    fn embedded(name: &str) -> &'static str {
        EMBEDDED.iter()
            .find(|(n, _)| *n == name)
            .map(|(_, src)| *src)
            .unwrap_or("")
    }
}
