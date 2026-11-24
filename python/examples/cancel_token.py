"""Cooperative cancellation example — cancel a long-running module mid-flight."""

import threading
import time

from apcore import APCore
from apcore.cancel import CancelToken, ExecutionCancelledError
from apcore.context import Context


client = APCore()


@client.module(id="demo.slow_task", description="Simulates a long-running task")
def slow_task(steps: int, context: Context) -> dict:
    """Execute multiple steps, checking for cancellation before each."""
    completed = 0
    for i in range(steps):
        if context.cancel_token:
            context.cancel_token.check()
        time.sleep(0.05)  # Simulate work
        completed += 1
    return {"completed": completed}


if __name__ == "__main__":
    # Run 1: Normal completion — no cancel token, all steps finish
    print("--- Run 1: Normal completion ---")
    result = client.call("demo.slow_task", {"steps": 3})
    print(f"Completed: {result}")

    # Run 2: Cancel mid-flight — token fires after 80 ms, interrupting a 10-step task
    print("\n--- Run 2: Cancel after 80ms ---")
    token = CancelToken()

    # cancel_token is a first-class Context.create() parameter (v0.22.0,
    # Issue #66). Pass it at construction time — the post-hoc
    # ctx.cancel_token = token anti-pattern is no longer required.
    ctx = Context.create(cancel_token=token)

    # Fire cancellation from a background thread after 80 ms
    timer = threading.Timer(0.08, token.cancel)
    timer.start()

    try:
        client.call("demo.slow_task", {"steps": 10}, ctx)
        print("Should not reach here")
    except ExecutionCancelledError as e:
        print(f"Caught cancellation: {e}")
    finally:
        timer.cancel()  # clean up the timer if execution ended before it fired

    # Token state inspection
    print(f"\nToken cancelled: {token.is_cancelled}")  # True
    token.reset()
    print(f"After reset:     {token.is_cancelled}")  # False
