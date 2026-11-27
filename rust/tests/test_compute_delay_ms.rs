//! Sync alignment (D-08): the canonical retry-delay method is named
//! `compute_delay_ms`.

use apcore::async_task::RetryConfig;

#[test]
fn compute_delay_ms_applies_backoff_and_cap() {
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
