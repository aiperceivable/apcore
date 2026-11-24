"""EventRetryConfig — per-subscriber retry policy for the apcore event bus."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class EventRetryConfig:
    """Retry policy for event delivery to a single subscriber.

    Attributes:
        max_attempts: Total delivery attempts (including the initial one).
        initial_backoff_ms: Delay after the first failure, in milliseconds.
        max_backoff_ms: Upper bound on computed delays, in milliseconds.
        backoff_multiplier: Exponential factor applied per retry.
    """

    max_attempts: int = 3
    initial_backoff_ms: int = 100
    max_backoff_ms: int = 30_000
    backoff_multiplier: float = 2.0

    def compute_delay_ms(self, attempt: int) -> int:
        """Return the backoff delay for a given zero-based attempt index.

        Args:
            attempt: Zero-based retry counter (0 = first retry after initial fail).
        """
        return min(
            self.max_backoff_ms,
            int(self.initial_backoff_ms * (self.backoff_multiplier**attempt)),
        )
