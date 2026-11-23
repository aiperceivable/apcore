/**
 * Entry point resolution for discovered module files.
 */

import { ModuleLoadError } from '../errors.js';

export function snakeToPascal(name: string): string {
  if (!name) return '';
  return name.split('_').map((part) => part.charAt(0).toUpperCase() + part.slice(1)).join('');
}

function isModuleClass(obj: unknown): boolean {
  if (typeof obj !== 'object' || obj === null) return false;
  const record = obj as Record<string, unknown>;
  return (
    record['inputSchema'] != null &&
    typeof record['inputSchema'] === 'object' &&
    record['outputSchema'] != null &&
    typeof record['outputSchema'] === 'object' &&
    typeof record['description'] === 'string' &&
    typeof record['execute'] === 'function'
  );
}

export async function resolveEntryPoint(
  filePath: string,
  meta?: Record<string, unknown> | null,
): Promise<unknown> {
  let loaded: Record<string, unknown>;
  try {
    loaded = await import(filePath);
  } catch (e) {
    throw new ModuleLoadError(filePath, `Failed to import module: ${e}`);
  }

  // Meta override mode
  if (meta && 'entry_point' in meta) {
    const className = (meta['entry_point'] as string).split(':').pop()!;
    const cls = loaded[className];
    if (cls == null) {
      throw new ModuleLoadError(filePath, `Entry point class '${className}' not found`);
    }
    return cls;
  }

  // Auto-infer: look for default export first, then named exports
  if (loaded['default'] && isModuleClass(loaded['default'])) {
    return loaded['default'];
  }

  const candidates: unknown[] = [];
  for (const [, value] of Object.entries(loaded)) {
    if (isModuleClass(value)) {
      candidates.push(value);
    }
  }

  if (candidates.length === 1) return candidates[0];
  if (candidates.length === 0) {
    throw new ModuleLoadError(filePath, 'No Module subclass found in file');
  }
  throw new ModuleLoadError(filePath, 'Ambiguous entry point: multiple Module subclasses found');
}
