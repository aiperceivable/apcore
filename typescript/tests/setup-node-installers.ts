/**
 * Vitest setup: install the Node-only file readers that the package's
 * Node entry (`src/index.ts`) wires up via side-effect imports.
 *
 * Tests typically import individual source files directly
 * (`from '../src/acl.js'` etc.), which skips the Node entry and so
 * leaves `ACL.load`, `registerSysModules` overrides loading, etc. in
 * their browser-no-op state. Importing the side-effect modules here
 * runs them once before any test executes.
 */

import '../src/acl-file.js';
import '../src/sys-modules/overrides-file.js';
import '../src/sys-modules/install.js';
