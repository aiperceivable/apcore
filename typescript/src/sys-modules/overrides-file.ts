/**
 * Side-effect module: installs the Node-side overrides-file loader on
 * `registerSysModules`.
 *
 * This file is imported by the package's Node entry (`src/index.ts`). It is
 * intentionally NOT re-exported from the browser entry — browser bundles
 * therefore see `_overridesLoader === null` and skip the file-loading
 * branch in `registerSysModules`.
 *
 * Pulled out of `registration.ts` so that the `node:fs` static import
 * lives on a leaf module the browser entry never reaches.
 */

import { existsSync, readFileSync } from 'node:fs';
import yaml from 'js-yaml';
import { _setOverridesLoader } from './registration.js';

_setOverridesLoader((path: string): Record<string, unknown> | null => {
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
});
