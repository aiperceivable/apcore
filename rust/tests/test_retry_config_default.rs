// D-14 alignment: RetryConfig::default().max_retries MUST be 0 (matching
// apcore-python and apcore-typescript). The previous Rust default of 3
// silently retried failed tasks 3 times — a behavior the spec explicitly
// requires to be opt-in.

use apcore::async_task::RetryConfig;

#[test]
fn retry_config_default_has_zero_retries() {
    let cfg = RetryConfig::default();
    assert_eq!(
        cfg.max_retries, 0,
        "RetryConfig::default().max_retries must be 0 to match Python/TS spec"
    );
}
