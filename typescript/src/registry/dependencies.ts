/**
 * Dependency resolution via Kahn's topological sort.
 */

import {
  CircularDependencyError,
  DependencyNotFoundError,
  DependencyVersionMismatchError,
  ModuleLoadError,
} from '../errors.js';
import type { DependencyInfo } from './types.js';
import { matchesVersionHint } from './version.js';

export function resolveDependencies(
  modules: Array<[string, DependencyInfo[]]>,
  knownIds?: Set<string> | null,
  moduleVersions?: Map<string, string> | Record<string, string> | null,
): string[] {
  if (modules.length === 0) return [];

  const ids = knownIds ?? new Set(modules.map(([id]) => id));

  const versionLookup: Map<string, string> | null = (() => {
    if (moduleVersions == null) return null;
    if (moduleVersions instanceof Map) return moduleVersions;
    return new Map(Object.entries(moduleVersions));
  })();

  // Build graph and in-degree
  const graph = new Map<string, Set<string>>();
  const inDegree = new Map<string, number>();

  for (const [modId] of modules) {
    inDegree.set(modId, 0);
  }

  for (const [moduleId, deps] of modules) {
    for (const dep of deps) {
      if (!ids.has(dep.moduleId)) {
        if (dep.optional) continue;
        throw new DependencyNotFoundError(moduleId, dep.moduleId);
      }
      if (dep.version && versionLookup !== null) {
        const actual = versionLookup.get(dep.moduleId);
        if (actual === undefined) {
          // Version constraint cannot be evaluated — surface this so a silent
          // bypass does not masquerade as a satisfied constraint. Typical
          // cause: the target module is registered but has no `version`
          // string on its class and no YAML metadata declared one.
          console.warn(
            `[apcore:registry] Cannot enforce version constraint '${dep.version}' on '${dep.moduleId}' for '${moduleId}': no version information available for the target module`,
          );
        } else if (!matchesVersionHint(actual, dep.version)) {
          if (dep.optional) {
            console.warn(
              `[apcore:registry] Optional dependency '${dep.moduleId}' for module '${moduleId}' has version '${actual}' which does not satisfy constraint '${dep.version}', skipping`,
            );
            continue;
          }
          throw new DependencyVersionMismatchError(moduleId, dep.moduleId, dep.version, actual);
        }
      }
      if (!graph.has(dep.moduleId)) graph.set(dep.moduleId, new Set());
      graph.get(dep.moduleId)!.add(moduleId);
      inDegree.set(moduleId, (inDegree.get(moduleId) ?? 0) + 1);
    }
  }

  // Initialize queue with zero-in-degree nodes (sorted for determinism)
  const queue: string[] = [...inDegree.entries()]
    .filter(([, deg]) => deg === 0)
    .map(([id]) => id)
    .sort();

  const loadOrder: string[] = [];
  while (queue.length > 0) {
    const modId = queue.shift()!;
    loadOrder.push(modId);
    const dependents = graph.get(modId);
    if (dependents) {
      for (const dependent of [...dependents].sort()) {
        const newDeg = (inDegree.get(dependent) ?? 1) - 1;
        inDegree.set(dependent, newDeg);
        if (newDeg === 0) {
          queue.push(dependent);
        }
      }
    }
  }

  // The sort stalled. Kahn's algorithm terminating with nodes remaining has two
  // distinct causes and only one of them is a cycle (spec §5.15.2, D-79): a real
  // cycle means the author must break a dependency edge, while a stall with no
  // back edge means a required dependency is simply missing from this batch.
  // Reporting CIRCULAR_DEPENDENCY for the latter — with a cyclePath synthesised
  // from the leftover set, often a one-element "cycle" — sends the author
  // looking for a loop that does not exist.
  if (loadOrder.length < modules.length) {
    const ordered = new Set(loadOrder);
    const remaining = new Set(modules.filter(([id]) => !ordered.has(id)).map(([id]) => id));
    const cyclePath = findBackEdgeCycle(modules, remaining);
    if (cyclePath !== null) {
      throw new CircularDependencyError(cyclePath);
    }
    const blocked = [...remaining].sort();
    throw new ModuleLoadError(
      blocked.join(','),
      `${blocked.length} module(s) could not be loaded — ` +
        `blocked by unresolved required dependencies: ${JSON.stringify(blocked)}`,
    );
  }

  return loadOrder;
}

/**
 * Return a back-edge cycle `[n0, ..., nk, n0]` if one exists in `remaining`,
 * else `null`.
 *
 * The earlier `extractCycle` fell back to `sorted(remaining)` when it found no
 * back edge, which made a non-cycle blockage indistinguishable from a real
 * cycle. Returning `null` lets the caller tell them apart (D-79).
 */
function findBackEdgeCycle(
  modules: Array<[string, DependencyInfo[]]>,
  remaining: Set<string>,
): string[] | null {
  const depMap = new Map<string, string[]>();
  for (const [modId, deps] of modules) {
    if (remaining.has(modId)) {
      const uniq = new Set(deps.filter((d) => remaining.has(d.moduleId)).map((d) => d.moduleId));
      depMap.set(modId, [...uniq].sort());
    }
  }

  for (const start of [...remaining].sort()) {
    const cycle = dfsFindCycle(depMap, start);
    if (cycle !== null) return cycle;
  }

  return null;
}

function dfsFindCycle(depMap: Map<string, string[]>, start: string): string[] | null {
  const path: string[] = [];
  const onPath = new Set<string>();
  const visited = new Set<string>();
  const stack: Array<[string, number]> = [[start, 0]];

  while (stack.length > 0) {
    const frame = stack[stack.length - 1];
    const [node, idx] = frame;
    if (idx === 0) {
      if (onPath.has(node)) {
        const startIdx = path.indexOf(node);
        return [...path.slice(startIdx), node];
      }
      if (visited.has(node)) {
        stack.pop();
        continue;
      }
      visited.add(node);
      onPath.add(node);
      path.push(node);
    }

    const neighbors = depMap.get(node) ?? [];
    if (idx < neighbors.length) {
      frame[1] = idx + 1;
      stack.push([neighbors[idx], 0]);
    } else {
      onPath.delete(node);
      path.pop();
      stack.pop();
    }
  }

  return null;
}
