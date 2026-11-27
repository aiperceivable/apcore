import { discoverAcl } from './acl-yaml.js';
import type { APCorePlugin } from './client.js';

export { Config, discoverConfigFile, RESERVED_NAMESPACES } from './config.js';
export { discoverAcl, loadAclFile } from './acl-yaml.js';
export { buildStrategyFromConfig, buildStrategyFromConfigSync } from './pipeline-config.js';

/** Discover an ACL for the executor this client constructs. */
export function createAclDiscoveryPlugin(): APCorePlugin {
  return { discoverAcl };
}
