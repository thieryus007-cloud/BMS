# Problemes à Resoudre:

# 1/
```


```


# 2/
```
Daly-BMS-Rust/crates/energy-manager/src/logic/meteo/mod.rs
ligne 58

 // santuario/meteo/venus — irradiance + solar + wind
    let meteo_payload = json!({
        "Irradiance":    s.irradiance_wm2,
        "TodaysYield":   s.total_yield_today_kwh,
        "YieldYesterday": s.yield_yesterday_kwh,
        "WindSpeed":     s.wind_speed_ms,
        "MpptPower":     s.mppt_273.power_w.unwrap_or(0.0) + s.mppt_289.power_w.unwrap_or(0.0),
        "SolarTotal":    s.solar_total_w,
        "Mppts": [
            {
```

# 3/
```
2026-05-05T13:51:59.747391Z  INFO energy_manager::logic::water_heater: Water heater: irradiance=10.0 W/m², min=300, irradiance_low=true

```

# 4/ 
```




```
