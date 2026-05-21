/// Build a Victron N/ topic for reading.
/// portal_id: e.g. "c0619ab9929a"
pub fn n(portal_id: &str, path: &str) -> String {
    format!("N/{portal_id}/{path}")
}

/// Build a Victron W/ topic for writing a command.
pub fn w(portal_id: &str, path: &str) -> String {
    format!("W/{portal_id}/{path}")
}

// ---------------------------------------------------------------------------
// All topics the energy-manager subscribes to
// ---------------------------------------------------------------------------

pub fn all_subscriptions(portal_id: &str, vebus: u32, mppt1: u32, mppt2: u32, pvinv: u32, shunt: u32) -> Vec<String> {
    let pid = portal_id;
    vec![
        // --- VEBus ---
        n(pid, &format!("vebus/{vebus}/Ac/State/IgnoreAcIn1")),
        n(pid, &format!("vebus/{vebus}/Ac/Out/L1/F")),
        n(pid, &format!("vebus/{vebus}/Ac/ActiveIn/Connected")),
        n(pid, &format!("vebus/{vebus}/Dc/0/Voltage")),
        n(pid, &format!("vebus/{vebus}/Dc/0/Current")),
        n(pid, &format!("vebus/{vebus}/Dc/0/Power")),
        n(pid, &format!("vebus/{vebus}/Ac/Out/L1/V")),
        n(pid, &format!("vebus/{vebus}/Ac/Out/L1/I")),
        n(pid, &format!("vebus/{vebus}/Ac/Out/L1/P")),
        n(pid, &format!("vebus/{vebus}/State")),
        n(pid, &format!("vebus/{vebus}/Energy/InverterToAcOut")),
        n(pid, &format!("vebus/{vebus}/Energy/OutToInverter")),

        // --- System aggregates ---
        n(pid, "system/0/Dc/Battery/Soc"),
        n(pid, "system/0/Dc/Battery/Current"),
        n(pid, "system/0/Dc/Battery/Power"),
        n(pid, "system/0/Dc/Battery/State"),
        n(pid, "system/0/Dc/Battery/TimeToGo"),
        n(pid, "system/0/Dc/Pv/Power"),
        n(pid, "system/0/Ac/PvOnOutput/L1/Power"),
        n(pid, "system/0/Ac/ConsumptionOnOutput/L1/Power"),

        // --- SmartShunt (battery/{shunt}) — native energy counters ---
        n(pid, &format!("battery/{shunt}/Dc/0/Voltage")),
        n(pid, &format!("battery/{shunt}/Dc/0/Current")),
        n(pid, &format!("battery/{shunt}/Dc/0/Power")),
        n(pid, &format!("battery/{shunt}/Soc")),
        n(pid, &format!("battery/{shunt}/TimeToGo")),
        n(pid, &format!("battery/{shunt}/State")),
        // Wildcards: match any battery instance so we don't need to know the
        // exact SmartShunt instance number (varies per installation).
        n(pid, "battery/+/History/ChargedEnergy"),
        n(pid, "battery/+/History/DischargedEnergy"),

        // --- MPPT 1 ---
        n(pid, &format!("solarcharger/{mppt1}/Yield/Power")),
        n(pid, &format!("solarcharger/{mppt1}/History/Daily/0/Yield")),
        n(pid, &format!("solarcharger/{mppt1}/History/Daily/0/MaxPower")),
        n(pid, &format!("solarcharger/{mppt1}/State")),
        n(pid, &format!("solarcharger/{mppt1}/Pv/V")),
        n(pid, &format!("solarcharger/{mppt1}/Dc/0/Current")),

        // --- MPPT 2 ---
        n(pid, &format!("solarcharger/{mppt2}/Yield/Power")),
        n(pid, &format!("solarcharger/{mppt2}/History/Daily/0/Yield")),
        n(pid, &format!("solarcharger/{mppt2}/History/Daily/0/MaxPower")),
        n(pid, &format!("solarcharger/{mppt2}/State")),
        n(pid, &format!("solarcharger/{mppt2}/Pv/V")),
        n(pid, &format!("solarcharger/{mppt2}/Dc/0/Current")),

        // --- PVInverter (ET112) ---
        n(pid, &format!("pvinverter/{pvinv}/Ac/Power")),
        n(pid, &format!("pvinverter/{pvinv}/Ac/Energy/Forward")),

        // --- Irradiance ---
        "santuario/irradiance/raw".to_string(),

        // --- Shelly (DEYE relay events) ---
        "shellypro2pm-ec62608840a4/events/rpc".to_string(),

        // --- Tasmota (water heater relay) ---
        "stat/tongou_3BC764/POWER".to_string(),
        "tele/tongou_3BC764/SENSOR".to_string(),

        // --- Persist (retained baselines + DEYE state) ---
        "santuario/persist/pvinv_baseline".to_string(),
        "santuario/persist/yield_yesterday".to_string(),
        "santuario/persist/deye_state".to_string(),
    ]
}

// ---------------------------------------------------------------------------
// Publish topics (outputs)
// ---------------------------------------------------------------------------

pub mod publish {
    use super::w;

    pub fn vebus_max_charge_current(portal_id: &str, vebus: u32) -> String {
        w(portal_id, &format!("vebus/{vebus}/Dc/0/MaxChargeCurrent"))
    }

    pub fn vebus_power_assist(portal_id: &str, vebus: u32) -> String {
        w(portal_id, &format!("vebus/{vebus}/Settings/PowerAssistEnabled"))
    }

    pub fn cgwacs_max_feed_in(portal_id: &str) -> String {
        w(portal_id, "settings/0/Settings/CGwacs/MaxFeedInPower")
    }

    pub fn shelly_rpc(shelly_id: &str) -> String {
        format!("{shelly_id}/rpc")
    }

    #[allow(dead_code)]
    pub fn tasmota_cmd(tasmota_id: &str) -> String {
        format!("cmnd/{tasmota_id}/Power")
    }

    pub const HEATPUMP_VENUS: &str = "santuario/heatpump/1/venus";
    pub const SWITCH_VENUS:   &str = "santuario/switch/1/venus";
    pub const PLATFORM_VENUS: &str = "santuario/platform/venus";
    pub const HEAT_VENUS:     &str = "santuario/heat/1/venus";
    pub const METEO_VENUS:    &str = "santuario/meteo/venus";
    pub const INVERTER_VENUS: &str = "santuario/inverter/venus";
    pub const SYSTEM_VENUS:   &str = "santuario/system/venus";
    pub const PVINV_BASELINE:  &str = "santuario/persist/pvinv_baseline";
    pub const YIELD_YESTERDAY: &str = "santuario/persist/yield_yesterday";
    pub const DEYE_STATE:      &str = "santuario/persist/deye_state";
}
