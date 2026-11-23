/**
 * Registry types: ModuleDescriptor, DiscoveredModule, DependencyInfo.
 */

import type { ModuleAnnotations, ModuleExample } from '../module.js';

export interface ModuleDescriptor {
  moduleId: string;
  name: string | null;
  description: string;
  documentation: string | null;
  inputSchema: Record<string, unknown>;
  outputSchema: Record<string, unknown>;
  version: string;
  tags: string[];
  annotations: ModuleAnnotations | null;
  examples: ModuleExample[];
  metadata: Record<string, unknown>;
  sunsetDate: string | null;
  /**
   * Declared module dependencies, parsed from `metadata.dependencies`.
   *
   * Promoted to a typed field rather than left as raw JSON under `metadata`.
   * `metadata` is specified as "arbitrary extension metadata" — the `x-` layer
   * of the three-layer model — while dependencies are structural data the
   * framework itself consumes for load and reload ordering.
   *
   * PROTOCOL_SPEC §12.2 requires a `dependencies` entry in `metadata` to reach
   * the registered module's descriptor. apcore-rust carried a typed
   * `Vec<DependencyInfo>`; this SDK surfaced it on neither `getDefinition()`
   * nor `descriptor.metadata` — only through `getModuleMetadata()`
   * (sync finding A-D-004).
   */
  dependencies: DependencyInfo[];
}

export interface DiscoveredModule {
  filePath: string;
  canonicalId: string;
  metaPath: string | null;
  namespace: string | null;
}

export interface DependencyInfo {
  moduleId: string;
  version: string | null;
  optional: boolean;
}
