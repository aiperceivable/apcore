"""Opt-in call-tree budget and per-attempt accounting."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass
from threading import Lock
from types import MappingProxyType
from typing import Literal

from apcore.errors import CallBudgetExceededError, InvalidInputError

_MAX_SAFE_INTEGER = (1 << 53) - 1


def _validate_cost(value: int, name: str) -> None:
    if type(value) is not int or not 0 <= value <= _MAX_SAFE_INTEGER:
        raise InvalidInputError(f"{name} must be a non-negative safe integer")


@dataclass(frozen=True, slots=True)
class CallBudgetSnapshot:
    """An instantaneous view of the budget's accounting."""

    calls_used: int
    calls_reserved: int
    cost_used: int


class CallBudget:
    """Share an optional call count and cost budget across a Context tree."""

    def __init__(
        self,
        max_calls: int | None = None,
        max_cost: int | None = None,
        estimated_costs: Mapping[str, int] | None = None,
    ) -> None:
        if max_calls is not None:
            _validate_cost(max_calls, "max_calls")
        if max_cost is not None:
            _validate_cost(max_cost, "max_cost")
        costs = dict(estimated_costs) if estimated_costs is not None else {}
        for module_id, cost in costs.items():
            if not isinstance(module_id, str) or not module_id:
                raise InvalidInputError("Estimated cost keys must be module IDs")
            _validate_cost(cost, "estimated cost")
        self._max_calls = max_calls
        self._max_cost = max_cost
        self._estimated_costs = MappingProxyType(costs)
        self._lock = Lock()
        self._calls_used = 0
        self._calls_reserved = 0
        self._cost_used = 0

    def snapshot(self) -> CallBudgetSnapshot:
        """Report calls and cost, clamped to the portable safe integer range."""
        with self._lock:
            return CallBudgetSnapshot(
                min(self._calls_used, _MAX_SAFE_INTEGER),
                min(self._calls_reserved, _MAX_SAFE_INTEGER),
                min(self._cost_used, _MAX_SAFE_INTEGER),
            )

    def _check_allowance(self, estimate: int) -> None:
        if self._max_calls is not None and self._calls_used + self._calls_reserved >= self._max_calls:
            raise CallBudgetExceededError("calls")
        self._check_cost(self._cost_used + estimate)

    def _check_cost(self, cost: int) -> None:
        if self._max_cost is not None and cost > self._max_cost:
            raise CallBudgetExceededError("cost")

    def _check(self, module_id: str) -> None:
        with self._lock:
            self._check_allowance(self._estimated_costs.get(module_id, 0))

    def _reserve(self, module_id: str) -> _CallReservation:
        estimate = self._estimated_costs.get(module_id, 0)
        with self._lock:
            self._check_allowance(estimate)
            self._calls_reserved += 1
            return _CallReservation(self, estimate)


class _CallReservation:
    """One guarded attempt, held until it starts or exits."""

    def __init__(self, budget: CallBudget, estimate: int) -> None:
        self._budget = budget
        self._estimate = estimate
        self._state: Literal["reserved", "active", "closed"] = "reserved"
        self._actual_reported = False

    def start(self) -> None:
        with self._budget._lock:
            if self._state != "reserved":
                raise InvalidInputError("Call budget reservation is no longer available")
            cost = self._budget._cost_used + self._estimate
            self._budget._check_cost(cost)
            self._budget._calls_reserved -= 1
            self._budget._calls_used += 1
            self._budget._cost_used = cost
            self._state = "active"

    def record_cost(self, actual: int) -> None:
        _validate_cost(actual, "actual cost")
        with self._budget._lock:
            if self._state != "active" or self._actual_reported:
                raise InvalidInputError("Actual cost can be recorded once during an active budgeted call")
            self._actual_reported = True
            self._budget._cost_used += actual - self._estimate
            self._budget._check_cost(self._budget._cost_used)

    def release(self) -> None:
        with self._budget._lock:
            if self._state == "reserved":
                self._budget._calls_reserved -= 1
                self._state = "closed"

    def finish(self) -> None:
        with self._budget._lock:
            if self._state == "active":
                self._state = "closed"
