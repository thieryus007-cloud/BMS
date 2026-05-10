/// Push rule-engine evaluation metrics to VictoriaMetrics (Prometheus format).
/// Call this after each rule evaluation to track which rules fire and how often.
use tracing::warn;

pub async fn record_rule_eval(vm_url: &str, rule_name: &str) {
    let ts_ms = chrono::Utc::now().timestamp_millis();
    let line  = format!("rule_eval_total{{rule=\"{rule_name}\"}} 1 {ts_ms}");
    let url   = format!("{vm_url}/api/v1/import/prometheus");
    if let Err(e) = reqwest::Client::new()
        .post(&url)
        .body(line)
        .send()
        .await
    {
        warn!("rule_metrics: VM write failed for {rule_name}: {e}");
    }
}
