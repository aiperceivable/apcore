use std::fmt::Write as _;

use super::metrics::{
    MetricsCollector, METRIC_CALLS_TOTAL, METRIC_DURATION_SECONDS, METRIC_ERRORS_TOTAL,
};
use super::usage::UsageCollector;

/// Render the collectors' current data in Prometheus text exposition format.
///
/// The standard metric families are present before the first observation.
/// Usage metrics are appended when a usage collector is supplied.
#[must_use]
pub fn render_prometheus_metrics(
    collector: &MetricsCollector,
    usage: Option<&UsageCollector>,
) -> String {
    let body = collector.export_prometheus();
    let mut output = String::new();
    ensure_metric_present(&mut output, &body, METRIC_CALLS_TOTAL, "counter");
    ensure_metric_present(&mut output, &body, METRIC_ERRORS_TOTAL, "counter");
    ensure_metric_present(&mut output, &body, METRIC_DURATION_SECONDS, "histogram");
    output.push_str(&body);
    if let Some(usage) = usage {
        output.push_str(&usage.export_prometheus());
    }
    output
}

fn ensure_metric_present(output: &mut String, body: &str, name: &str, metric_type: &str) {
    if body.contains(&format!("# TYPE {name} "))
        || body.contains(&format!("{name} "))
        || body.contains(&format!("{name}{{"))
        || body.contains(&format!("{name}_bucket"))
    {
        return;
    }
    let _ = writeln!(output, "# HELP {name} apcore standard metric");
    let _ = writeln!(output, "# TYPE {name} {metric_type}");
}
