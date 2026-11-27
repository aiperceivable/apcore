"""Measure a fixed installed-package workload on one machine."""

import argparse
import json
import os
import statistics
import subprocess
import time
from pathlib import Path

PYTHON = """
import asyncio
import json
import time
from concurrent.futures import ThreadPoolExecutor

from apcore import APCore

client = APCore()


@client.module("bench.echo", description="Echo an integer")
def echo(value: int) -> dict[str, int]:
    return {"value": value}


async def main():
    for _ in range(100):
        await client.call_async("bench.echo", {"value": 1})
    samples = []
    for i in range(1000):
        start = time.perf_counter_ns()
        assert (await client.call_async("bench.echo", {"value": i}))["value"] == i
        samples.append(time.perf_counter_ns() - start)
    result = {}
    result["pipeline_p50_us"] = sorted(samples)[499] / 1000
    result["pipeline_p95_us"] = sorted(samples)[949] / 1000
    return result


def sync_call(value):
    assert client.call("bench.echo", {"value": value})["value"] == value


result = asyncio.run(main())
start = time.perf_counter()
with ThreadPoolExecutor(4) as workers:
    list(workers.map(sync_call, range(1000)))
result["sync_four_threads_calls_per_second"] = 1000 / (time.perf_counter() - start)
print(json.dumps(result))
"""

JAVASCRIPT = """
import { APCore } from 'apcore-js';
const client = new APCore();
client.module({
  id: 'bench.echo',
  description: 'Echo an integer',
  inputSchema: { type: 'object', properties: { value: { type: 'integer' } }, required: ['value'] },
  outputSchema: { type: 'object', properties: { value: { type: 'integer' } }, required: ['value'] },
  execute: ({ value }) => ({ value }),
});
for (let i = 0; i < 100; ++i) await client.call('bench.echo', { value: i });
const samples = [];
for (let i = 0; i < 1000; ++i) {
  const start = process.hrtime.bigint();
  const result = await client.call('bench.echo', { value: i });
  if (result.value !== i) throw new Error('unexpected result');
  samples.push(Number(process.hrtime.bigint() - start));
}
samples.sort((a, b) => a - b);
console.log(JSON.stringify({ pipeline_p50_us: samples[499] / 1000, pipeline_p95_us: samples[949] / 1000 }));
"""


def measure(label, command, cold, program, cwd):
    env = dict(os.environ)
    env.pop("PYTHONPATH", None)
    starts = []
    for _ in range(15):
        start = time.perf_counter_ns()
        subprocess.run([*command, cold], cwd=cwd, env=env, check=True, capture_output=True)
        starts.append(time.perf_counter_ns() - start)
    samples = [
        json.loads(subprocess.check_output([*command, program], cwd=cwd, env=env, text=True))
        for _ in range(3)
    ]
    result = {key: statistics.median(sample[key] for sample in samples) for key in samples[0]}
    result["cold_process_and_import_median_ms"] = statistics.median(starts) / 1_000_000
    return label, result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--python", type=Path)
    parser.add_argument("--node", type=Path)
    parser.add_argument("--npm-directory", type=Path)
    parser.add_argument("--rust-binary", type=Path)
    args = parser.parse_args()
    results = {}
    if args.python is not None:
        label, result = measure("python", [args.python, "-I", "-c"], "import apcore", PYTHON, None)
        results[label] = result
    if args.node is not None and args.npm_directory is not None:
        label, result = measure(
            "typescript", [args.node, "--input-type=module", "-e"],
            "await import('apcore-js')", JAVASCRIPT, args.npm_directory,
        )
        results[label] = result
    if args.rust_binary is not None:
        label, result = measure("rust", [args.rust_binary], "--startup", "--pipeline", None)
        result["cold_process_and_assembly_median_ms"] = result.pop("cold_process_and_import_median_ms")
        results[label] = result
    if not results:
        parser.error("pass --python, --rust-binary, or both --node and --npm-directory")
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
