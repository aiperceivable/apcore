/** Filesystem capability for a Node discovery instance. */
import { randomUUID } from 'node:crypto';
import { accessSync, realpathSync, watch } from 'node:fs';
import * as path from 'node:path';
import { pathToFileURL } from 'node:url';
import { loadIdMap, loadMetadata } from './metadata.js';
import type { RegistryFilesystem } from './registry.js';
import { scanExtensions, scanMultiRoot } from './scanner.js';

export function createRegistryFilesystem(): RegistryFilesystem {
  const cacheNamespace = randomUUID();
  let reloadGeneration = 0;
  return {
    fs: {
      accessSync,
      realpathSync,
      watch(directory: string, options: { recursive: boolean }, listener: (event: string, filename: string | null) => void) {
        return watch(directory, options, listener);
      },
    },
    path,
    entryPointSpecifier(filePath: string, reload: boolean): string {
      const url = pathToFileURL(filePath);
      if (reload) {
        url.searchParams.set('apcore-reload', `${cacheNamespace}-${++reloadGeneration}`);
      }
      return url.href;
    },
    loadIdMap,
    loadMetadata,
    scanExtensions,
    scanMultiRoot,
  };
}
