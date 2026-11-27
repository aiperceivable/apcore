import { existsSync, readFileSync } from 'node:fs';
import yaml from 'js-yaml';

export function readFileOverrides(path: string): Record<string, unknown> | null {
  if (!existsSync(path)) return null;
  try {
    const content = readFileSync(path, 'utf-8');
    const parsed = yaml.load(content);
    return typeof parsed === 'object' && parsed !== null
      ? (parsed as Record<string, unknown>)
      : null;
  } catch (err) {
    console.warn('[apcore:sys-modules] Failed to load overrides file:', err);
    return null;
  }
}
