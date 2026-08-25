# system.usage.* parity — issue #96

**Internal planning. Not published to the doc site.**

Spec side landed: PROTOCOL_SPEC §6.7.1, `schemas/sys-usage-summary.schema.json`,
`schemas/sys-usage-module.schema.json`, corrected examples in
`docs/features/system-modules.md` and `docs/features/observability.md`.

Held here: `fixtures/usage_contract.json` (11 cases).

## Why the fixture is held

`.github/workflows/conformance-integrity.yml` runs `check_driver_coverage.py --strict`
and `check_expected_keys_read.py --strict` against sibling checkouts of the three
SDKs. A fixture in `conformance/fixtures/` that no SDK drives, or whose `expected`
keys no driver reads, fails the job — and the workflow says so in its own comment:

> LANDING ORDER: this step checks out the three SDKs as siblings, so a fixture that
> gains a new `expected` key reds this job until every driver asserts it. That is
> the guard working, not a flake — land the SDK drivers with, or before, the fixture.

This already bit once: `preflight_disclosure.json` landed in the doc repo ahead of
the drivers and `main` was red from 019eaa4 until the SDK side caught up.

The schemas are a different case and did land now. Nothing in CI validates SDK output
against them yet, so they are inert until a driver wires them up — which is exactly
what makes them useful as the target the SDK work converges on.

## Landing order

1. **Spec** — §6.7.1 + the two schemas. *Done.*
2. **SDK behaviour**, three PRs, independently mergeable:
   - **apcore-rust** — the largest share. Honour `period` in both usage modules
     (`get_summary_for_period` already exists for `summary`; the detail module needs
     period-aware variants of `get_module_summary`, `get_p99_latency_ms`,
     `get_caller_breakdown`, `get_hourly_distribution`, none of which currently takes
     one). Drop `HOUR_KEY_FORMAT` and emit the collector's key unchanged — and delete
     the comment claiming the two already match, which is the reason nobody looked.
     Declare `properties` + `required` in both `output_schema()` bodies.
   - **apcore-python** — `_compute_p99` returns `sorted_lat[rank]`; return
     `sorted_lat[idx]`, the index it already computes. This is a **behaviour change**:
     the reported p99 drops by one rank. Replace `_parse_period` with the
     `input_schema` pattern so `"0h"` / `"-5d"` / `"+3h"` fail validation instead of
     silently producing an empty or negative window.
   - **apcore-typescript** — smallest. Add the `period` pattern to `input_schema` so
     rejection happens at the same boundary as the other two rather than in
     `parsePeriod`; p99 and bucket key already conform.
3. **Drivers** for `usage_contract.json` in all three SDKs, merged.
4. **Fixture** — move `fixtures/usage_contract.json` to `conformance/fixtures/`, add a
   row to `conformance/README.md`, add a row to `docs/spec/conformance.md` §8.1 **and
   bump the Total** (CI verifies the count and the total; it was wrong in both
   directions before). Wire schema validation of live SDK output into the drivers.

## Do not

- Relax `hour.pattern` to accept `:00:00Z`. Rejecting today's apcore-rust output is the
  assertion, not a bug in the schema.
- Relax `additionalProperties: false`. A field one SDK emits and the others do not is a
  parity gap; failing loudly is the intent. If validation fails after step 2, establish
  which side is right before amending either.
- Land the fixture before step 3.

## Open, tracked elsewhere

The `$id` authority. These two files use `https://apcore.dev/`, matching all fifteen
existing schemas — the issue's own constraint is that the set must never be split, and
16/16 on one authority satisfies it. If the project migrates to
`apcore.aiperceivable.com` or the GitHub Pages URL, all seventeen move together.
