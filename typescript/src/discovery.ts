import { createNodeLifecycleCallbacks } from './lifecycle-node.js';
import type { APCorePlugin } from './client.js';
import { createRegistryFilesystem } from './registry/node-filesystem.js';
import { Registry, type RegistryOptions } from './registry/registry.js';

export type DiscoveryRegistryOptions = Omit<RegistryOptions, 'filesystem'>;

/** Create a registry with its own filesystem discovery and watch capability. */
export function createDiscoveryRegistry(options?: DiscoveryRegistryOptions): Registry {
  return new Registry({
    ...options,
    filesystem: createRegistryFilesystem(),
    lifecycleCallbacks: options?.lifecycleCallbacks ?? createNodeLifecycleCallbacks(),
  });
}

/** Install filesystem discovery on the registry this client constructs. */
export function createFilesystemDiscoveryPlugin(options?: Omit<DiscoveryRegistryOptions, 'config'>): APCorePlugin {
  return {
    createRegistry: (config, lifecycleCallbacks) => createDiscoveryRegistry({ ...options, config, lifecycleCallbacks }),
  };
}
