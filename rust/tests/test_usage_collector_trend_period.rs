// D-27 alignment: UsageCollector trend MUST be computed from samples (current
// vs previous period counts) rather than hardcoded "stable", record() MUST
// honor an optional explicit timestamp, and the summary API MUST accept a
// period filter so callers can scope the aggregation window.

use apcore::observability::UsageCollector;
use chrono::{Duration, Utc};

#[test]
fn record_at_honors_explicit_timestamp() {
    let collector = UsageCollector::new();
    let old = Utc::now() - Duration::hours(48);
    collector.record_at("executor.m", Some("@a"), 10.0, true, old);

    // 24h window should NOT include a 48-hour-old record.
    let summaries_24h = collector.get_summary_for_period(Some(Duration::hours(24)));
    let m = summaries_24h.iter().find(|s| s.module_id == "executor.m");
    assert!(
        m.is_none_or(|s| s.call_count == 0),
        "48h-old record must not appear in a 24h window"
    );

    // No-period (all-time) view MUST include it.
    let all = collector.get_summary_for_period(None);
    let m_all = all
        .iter()
        .find(|s| s.module_id == "executor.m")
        .expect("module must appear in all-time view");
    assert_eq!(m_all.call_count, 1);
}

#[test]
fn trend_is_computed_from_samples_not_hardcoded() {
    let collector = UsageCollector::new();
    let now = Utc::now();
    // Previous 24h-period (24-48h ago): 1 call.
    collector.record_at(
        "executor.m",
        Some("@a"),
        5.0,
        true,
        now - Duration::hours(36),
    );
    // Current 24h-period: 5 calls — should produce "rising" (ratio > 1.2).
    for i in 0..5 {
        collector.record_at(
            "executor.m",
            Some("@a"),
            5.0,
            true,
            now - Duration::minutes(i64::from(i) * 10),
        );
    }
    let summaries = collector.get_summary_for_period(Some(Duration::hours(24)));
    let m = summaries
        .iter()
        .find(|s| s.module_id == "executor.m")
        .expect("module must be present");
    assert_eq!(
        m.trend, "rising",
        "trend must reflect sample ratio (got {})",
        m.trend
    );
}

#[test]
fn trend_new_when_no_previous_samples() {
    let collector = UsageCollector::new();
    let now = Utc::now();
    collector.record_at("executor.m", Some("@a"), 5.0, true, now);
    let summaries = collector.get_summary_for_period(Some(Duration::hours(1)));
    let m = summaries
        .iter()
        .find(|s| s.module_id == "executor.m")
        .expect("module must be present");
    assert_eq!(m.trend, "new", "no previous samples must yield 'new'");
}

#[test]
fn period_filter_restricts_results() {
    let collector = UsageCollector::new();
    let now = Utc::now();
    collector.record_at(
        "executor.m",
        Some("@a"),
        5.0,
        true,
        now - Duration::days(10),
    );
    collector.record_at("executor.m", Some("@a"), 5.0, true, now);

    let summaries_1h = collector.get_summary_for_period(Some(Duration::hours(1)));
    let m_1h = summaries_1h
        .iter()
        .find(|s| s.module_id == "executor.m")
        .expect("module present");
    assert_eq!(
        m_1h.call_count, 1,
        "1h window must only include the recent record"
    );
}
