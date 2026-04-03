# Task 5: Handler Dispatch

Replace if/else chain in `_check_conditions` with handler dispatch.

## _evaluate_conditions (sync)
- Loop conditions.items()
- Lookup handler by key
- Unknown: WARN + return False (fail-closed)
- Call handler.evaluate()
- If result is awaitable: close coroutine, WARN, return False
- If False: return False
- All pass: return True

## _evaluate_conditions_async (async)
- Same but awaits async handlers
