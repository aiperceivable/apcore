/** Installs filesystem registry operations for the Node package entry point. */
import { accessSync, realpathSync, watch } from 'node:fs';
import * as path from 'node:path';
import { loadIdMap, loadMetadata } from './metadata.js';
import { _setRegistryFilesystem } from './registry.js';
import { scanExtensions, scanMultiRoot } from './scanner.js';

export const registryNodeFilesystem = {
  fs: {
    accessSync,
    realpathSync,
    watch(directory: string, options: { recursive: boolean }, listener: (event: string, filename: string | null) => void) {
      return watch(directory, options, listener);
    },
  },
  path,
  loadIdMap,
  loadMetadata,
  scanExtensions,
  scanMultiRoot,
};

_setRegistryFilesystem(registryNodeFilesystem);
