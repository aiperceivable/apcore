/** Installs filesystem registry operations for the Node package entry point. */
import { accessSync, realpathSync, watch } from 'node:fs';
import * as path from 'node:path';
import { pathToFileURL } from 'node:url';
import { loadIdMap, loadMetadata } from './metadata.js';
import { _setRegistryFilesystem } from './registry.js';
import { scanExtensions, scanMultiRoot } from './scanner.js';

let reloadGeneration = 0;

export const registryNodeFilesystem = {
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
      url.searchParams.set('apcore-reload', String(++reloadGeneration));
    }
    return url.href;
  },
  loadIdMap,
  loadMetadata,
  scanExtensions,
  scanMultiRoot,
};

_setRegistryFilesystem(registryNodeFilesystem);
