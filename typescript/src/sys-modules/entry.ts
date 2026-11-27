import type { APCorePlugin } from '../client.js';
import { registerSysModules, type RegisterSysModulesOptions } from './registration.js';

export { createSubscriberFromConfig, registerSysModules } from './registration.js';
export { FileOverridesStore } from './overrides.js';
export type { OverridesStore } from './overrides.js';
export type { RegisterSysModulesOptions, SubscriberFactory, SysModulesContext } from './registration.js';

/** Attach the system-module extension to one APCore instance. */
export function createSysModulesPlugin(
  options?: Omit<RegisterSysModulesOptions, 'failOnError' | 'onRegistrationError' | 'toggleState'>,
): APCorePlugin {
  return {
    registerSysModules: (registry, executor, config, metrics, runtime) =>
      registerSysModules(registry, executor, config, metrics, { ...options, ...runtime }),
  };
}
