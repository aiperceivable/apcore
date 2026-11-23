/**
 * Side-effect module: wires the Node-only `registerSysModules` into
 * `APCore` so that `new APCore({ config })` auto-registers sys modules.
 *
 * Imported by the package's Node entry (`src/index.ts`). The browser
 * entry intentionally does NOT import this file, which keeps the
 * `sys-modules/registration.ts` chain (and its transitive `node:fs` /
 * `node:path` dependencies via `events/subscribers.ts`,
 * `sys-modules/control.ts`) out of the browser bundle.
 */

import { _setSysModulesInstaller } from '../client.js';
import { registerSysModules } from './registration.js';

_setSysModulesInstaller(registerSysModules);
