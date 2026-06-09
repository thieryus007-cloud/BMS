/// Push rule-engine evaluation metrics to daly-bms-server / metrics-store redb (Prometheus format).
/// Call this after each rule evaluation to track which rules fire and how often.
use tracing::warn;

pub async fn record_rule_eval(vm_url: &str, rule_name: &str) {
    let ts_ms = chrono::Utc::now().timestamp_millis();
    let line  = format!("rule_eval_total{{rule=\"{rule_name}\"}} 1 {ts_ms}");
    let url   = format!("{vm_url}/api/v1/import/prometheus");
    // Client partagé et borné (audit 2026-06 §6) : appelé à chaque évaluation
    // de règle — un client neuf sans timeout par appel pouvait suspendre la
    // logique métier indéfiniment si le serveur ne répondait plus.
    if let Err(e) = crate::http_clients::shared_client()
        .post(&url)
        .body(line)
        .send()
        .await
    {
        warn!("rule_metrics: VM write failed for {rule_name}: {e}");
    }
}
