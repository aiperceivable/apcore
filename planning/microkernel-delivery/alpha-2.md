# APCore alpha.2 delivery

The starting points are Core `301c90131712f1a42a31c2efed6c19b9744b936c` and Surfaces `7aea980b6f4de3876d5edbbc247235f450fc25c8`. Both revisions passed their post-merge workflows. Both candidate source trees match their merged trees. Verify local artifacts throughout the refactor and obtain separate approval before publishing packages.

## Implementation order

1. Expose additive assembly APIs and package entry points. The in-memory profile provides registration, schema validation, governance, lifecycle and streaming completion. Make discovery, YAML configuration, system modules, observability integrations and event sinks explicit. Provide a full assembly for applications using these capabilities. TypeScript exposes a pure Prometheus renderer. Python updates the synchronous bridge while retaining the same asynchronous pipeline for every call.
2. Move Toolkit HTTP listener and A2A subscriber callers and tests to their Surfaces-owned interfaces. Connect the A2A subscriber through an instance-owned factory so multiple hosts can configure their subscriptions independently. Move MCP, OpenAI Chat and Responses, and Anthropic projections into Surfaces. Migrate consumers to the split SDK entry points and record the supported Core range for each resulting package.
3. Verify the migrated consumers against the replacement paths. Then change the default SDK entry points, remove the superseded Core integrations and SDK aliases, and apply the new approval defaults. Retain the deprecation window for configuration keys protected by protocol 1.x. Review the normative profile and default changes against the protocol's governance and versioning requirements.

## Workstreams

| Workstream | Acceptance evidence |
| --- | --- |
| Python packaging and assembly | A wheel installed into an empty environment can register and execute a module without YAML or a watcher. Explicit discovery, configuration and the full profile load their declared extras. The full suite passes. |
| Python synchronous bridge | Concurrent caller threads reuse independent loops. Nested calls are bounded and honor deadlines. Shutdown retires loop and worker resources while preserving lifecycle and stream cleanup. Calls share the asynchronous pipeline. |
| TypeScript packaging and assembly | The installed root works in Node and a browser bundle without Node or YAML in its reachable graph. Mixed minimal and full imports leave capabilities scoped to the instance. Full Node assembly preserves async lifecycle reentrancy protection. |
| Rust packaging and assembly | An external crate built with default features exercises registration, governance, cancellation, streaming completion and shutdown. The normal dependency graph excludes filesystem watching, YAML, socket listening and HTTP. Full assembly enables the extensions explicitly and passes complete conformance. Retain the runtime capabilities needed for native lifecycle and stream cleanup. |
| Surfaces | MCP, A2A, CLI and Toolkit pass their supported Core matrix using built SDK distributions. Test HTTP text and readiness, one subscriber delivery and two-host isolation, protocol tool schemas, and cancellation and final results through public entry points. |
| Governance | Exercise module, ACL and policy approval requirements with no handler and record zero module executions. Cover default clients, injected executors and configured strategies. Test explicit policy overrides and preflight alongside ordinary and streaming calls. |
| Shared fixtures and documentation | Keep protocol anchors, align the profile, schema and fixture metadata with the actual dependency boundary, and provide runnable migration examples. Record cold start, package size, execution latency and Python synchronous concurrency with reproducible commands. |

The approved alpha.1 build hashes remain the comparison baseline. Validate alpha.2 from its own built wheel, npm package and crate, with the Golden Path covering direct calls and MCP on the same distributions.

## Local acceptance

| Gate | Evidence |
| --- | --- |
| Python Core | 6,653 tests passed, with 40 skips and 5 expected failures. Clean base and `[all]` wheel installations passed dependency and execution checks. File discovery and native FSEvents passed with the full installation. The nested timeout regression completed without leaving an unretrieved Future. |
| TypeScript Core | 5,862 tests passed, with 36 skips. The installed root worked under Node and browser conditions and in a browser bundle. The minimal import graph excluded Node modules and YAML. Full Node assembly passed instance isolation and async lifecycle tests. |
| Rust Core | The default-feature archive passed 495 unit tests and four Core Profile tests on Rust 1.86. The full profile passed 602 unit tests, 2,439 consolidated integration tests and the standalone suites. Default features exclude YAML, file watchers, reqwest and Tokio file and socket features. Tokio runtime and macros support lifecycle cleanup and streaming. |
| Surfaces Python | Toolkit 1,289, MCP 1,275, A2A 522 and CLI 1,098 tests passed. The final installed Core and MCP wheels passed the direct and MCP Golden Path with the same approval and ACL rules. |
| Surfaces TypeScript | Toolkit 1,079, MCP 993, A2A 491 and CLI 950 tests passed. A separate npm consumer installed all four candidate packages with one Core version. Toolkit also passed installation with Core 0.33.0. |
| Surfaces Rust | Toolkit 714, MCP 1,152, A2A 242 and CLI 1,052 tests passed with the locked Core and Toolkit crates. Toolkit, MCP and A2A used Rust 1.86. CLI used stable Rust to meet its dependency floor. |
| Shared contract | All three SDKs drive all 98 Core fixture files. Every top-level expected field has a reader. The Core Profile fixture covers annotation, ACL, policy and destructive approval requirements with zero module execution on rejection. Surfaces use one shared fixture for provider projections, including recursive references. |
| Migration | The [runtime profile guide](../../docs/guides/runtime-profiles.md), schema and protocol draft reflect the capabilities of the installed packages. MkDocs strict build passed. |

The local review identified an Anthropic recursive-schema regression, a missing
TypeScript public export, and an inconsistent custom-strategy diagnostic. The
revised converters, packaged imports and governance tests cover these cases.
Follow hosted CI and review in [Core PR #236](https://github.com/aiperceivable/apcore/pull/236)
and [Surfaces PR #125](https://github.com/aiperceivable/apcore-surfaces/pull/125).
Tags and package publication remain paused.

### Initial local Core candidate artifacts

| Distribution | SDK version | SHA-256 |
| --- | --- | --- |
| Python wheel | `1.0.0a2` | `ed01a45192ce3fff2bc02f76d80b86c38d8a98f60701dce87d8d6952e18239bc` |
| npm archive | `1.0.0-alpha.2` | `fbfc83403234056023d843c424de58d42d6056cb9cbd94d92c0642d2ea0832d9` |
| Rust crate | `1.0.0-alpha.2` | `54e630a6f3c82ee9e01aec1f58ad14a06e21d8082414a1f4d398bce2f45b4de3` |

Surfaces CI builds its Python wheel from Core commit `dc61dd575a3d76a113f55a84175b6f36ba35e894`. The TypeScript and Rust jobs use Core commit `0384a3d4e7688c47eaa1feea6032fb4739813f20`. The Rust lock records the archive built from that commit, including its Cargo VCS metadata.

## Benchmark

Measured on arm64 macOS 15.8.3 with Python 3.11.18, Node 22.23.5, and Rust
1.86.0. The same benchmark registers an in-memory echo module and executes
1,000 validated calls after a 100-call warmup. Pipeline figures are medians
of three runs. Startup is the median of 15 fresh processes. Python and
TypeScript startup include SDK import. Rust startup includes client assembly
in a release-optimized binary.

| Measurement | Python alpha.1 | Python alpha.2 | TS alpha.1 | TS alpha.2 | Rust alpha.1 | Rust alpha.2 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Compressed Core package, bytes | 465,997 | 461,991 | 657,377 | 646,218 | 1,545,995 | 1,530,159 |
| Fresh-process startup, ms | 121.48 | 110.71 | 88.42 | 62.89 | 3.41 | 2.90 |
| Warm pipeline p50, μs | 319.54 | 321.04 | 30.67 | 30.29 | 31.75 | 17.08 |
| Warm pipeline p95, μs | 375.63 | 366.25 | 50.54 | 51.58 | 36.08 | 17.42 |
| Python four-thread sync calls per second | 2,033 | 2,693 | | | | |

These figures describe one workload on the same host. Python synchronous
throughput includes a four-thread caller pool and 1,000 successful calls.

Install each wheel into an isolated environment, and install each npm archive
into its own project. For Rust, unpack each crate into a separate `library`
directory. Place [bench-alpha2.rs](bench-alpha2.rs) in `runner/src/main.rs`
beside `library` and use the following manifest in `runner/Cargo.toml`.

```toml
[package]
name = "apcore-benchmark"
version = "0.0.0"
edition = "2021"
rust-version = "1.86"

[dependencies]
apcore = { path = "../library" }
serde_json = "1"
tokio = { version = "1", features = ["rt", "macros"] }
```

Build each Rust runner with `cargo +1.86.0 build --release`. Run the harness
once per installed release with that release's paths.

```sh
python3 planning/microkernel-delivery/bench-alpha2.py \
    --python /path/to/venv/bin/python \
    --node /path/to/node \
    --npm-directory /path/to/consumer \
    --rust-binary /path/to/runner/target/release/apcore-benchmark
```
