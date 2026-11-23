//! Sync alignment (D-08): the canonical retry-delay method is named
//! `compute_delay_ms`. The legacy `delay_for_attempt` is retained as a
//! `#[deprecated]` alias for one minor version.

use apcore::async_task::RetryConfig;

#[test]
fn compute_delay_ms_method_exists_and_matches_legacy() {
    let mut cfg = RetryConfig::default();
    cfg.max_retries = 5;
    cfg.retry_delay_ms = 1000;
    cfg.backoff_multiplier = 2.0;
    cfg.max_retry_delay_ms = 30_000;
    assert_eq!(cfg.compute_delay_ms(0), 1000);
    assert_eq!(cfg.compute_delay_ms(1), 2000);
    assert_eq!(cfg.compute_delay_ms(2), 4000);
    assert_eq!(cfg.compute_delay_ms(3), 8000);
    assert_eq!(cfg.compute_delay_ms(4), 16_000);
    assert_eq!(cfg.compute_delay_ms(5), 30_000); // cap
}

#[test]
#[allow(deprecated)]
fn delay_for_attempt_legacy_alias_still_works() {
    let mut cfg = RetryConfig::default();
    cfg.max_retries = 3;
    cfg.retry_delay_ms = 500;
    cfg.backoff_multiplier = 2.0;
    cfg.max_retry_delay_ms = 10_000;
    assert_eq!(cfg.delay_for_attempt(0), cfg.compute_delay_ms(0));
    assert_eq!(cfg.delay_for_attempt(2), cfg.compute_delay_ms(2));
}
