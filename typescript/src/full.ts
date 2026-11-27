import { ACL as RuntimeACL, type AclConfigLike, type AuditLogger } from './acl.js';
import { discoverAcl, loadAclFile } from './acl-yaml.js';
import { APCore as RuntimeAPCore, type APCoreOptions } from './client.js';
import { createNodeLifecycleCallbacks } from './lifecycle-node.js';
import { buildTracingMiddleware } from './observability/tracing-config.js';
import { createRegistryFilesystem } from './registry/node-filesystem.js';
import { Registry as RuntimeRegistry, type RegistryOptions } from './registry/registry.js';
import { registerSysModules } from './sys-modules/entry.js';

export * from './node-api.js';
export { EventEmitter } from './events/entry.js';
export { createNodeLifecyclePlugin } from './lifecycle-node.js';

export class Registry extends RuntimeRegistry {
  constructor(options?: RegistryOptions) {
    super({
      ...options,
      filesystem: options?.filesystem === undefined ? createRegistryFilesystem() : options.filesystem,
      lifecycleCallbacks: options?.lifecycleCallbacks ?? createNodeLifecycleCallbacks(),
    });
  }
}

export class ACL extends RuntimeACL {
  static override load(yamlPath: string, auditLogger?: AuditLogger | null): ACL {
    return loadAclFile(yamlPath, auditLogger, this);
  }

  static override discover(config: AclConfigLike): ACL | null {
    return discoverAcl(config, path => this.load(path));
  }
}

/** Assemble the supported Node capabilities for this client. */
export class APCore extends RuntimeAPCore {
  constructor(options?: APCoreOptions) {
    super(options, {
      createRegistry: (config, lifecycleCallbacks) => new Registry({ config, lifecycleCallbacks }),
      discoverAcl: config => ACL.discover(config),
      registerSysModules,
      createTracingMiddleware: buildTracingMiddleware,
    });
  }
}
