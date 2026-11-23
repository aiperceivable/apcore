/**
 * Canonical issue #123 drivers: D-133/134/136/139/140/141/146/148/149.
 * Every case uses the public SDK boundary specified by its driver_contract.
 * Fixtures are loaded in place; no implementation or expected result is mocked.
 */
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { Type } from '@sinclair/typebox';
import yaml from 'js-yaml';
import { describe, expect, it, vi } from 'vitest';
import { ACL, type ACLRule } from '../src/acl.js';
import { BindingLoader } from '../src/bindings.js';
import { CancelToken, ExecutionCancelledError } from '../src/cancel.js';
import { APCore } from '../src/client.js';
import { Config } from '../src/config.js';
import { Context, Identity } from '../src/context.js';
import { ModuleError } from '../src/errors.js';
import type { ApCoreEvent } from '../src/events/emitter.js';
import { Executor } from '../src/executor.js';
import { annotationsFromJSON, type Change, type PreviewResult } from '../src/module.js';
import { Registry } from '../src/registry/registry.js';
import { SchemaExporter } from '../src/schema/exporter.js';
import { jsonSchemaToTypeBox } from '../src/schema/loader-pure.js';
import type { ExportProfile } from '../src/schema/types.js';
import { findFixturesRoot } from './spec-repo.js';

type JsonObject = Record<string, unknown>;
interface Case<I> { id: string; input: I; expected: JsonObject }
interface Fixture<I> { test_cases: Case<I>[]; module_contract: JsonObject }

function fixture<I>(name: string): Fixture<I> {
  return JSON.parse(readFileSync(join(findFixturesRoot(), `${name}.json`), 'utf8')) as Fixture<I>;
}

function guardCases<I>(data: Fixture<I>, count: number, keys: string[]): void {
  expect(data.test_cases).toHaveLength(count);
  expect(new Set(data.test_cases.map((testCase) => testCase.id)).size).toBe(count);
  for (const testCase of data.test_cases) {
    for (const key of Object.keys(testCase.expected)) {
      expect(keys, `Unhandled expected field ${testCase.id}.${key}`).toContain(key);
    }
  }
}

function errorCode(error: unknown): string | undefined {
  // Namespace isolation reloads the error constructors too. Read the wire code,
  // not constructor identity, so a different module instance remains observable.
  if (error == null || typeof error !== 'object' || !('code' in error)) return undefined;
  return typeof error.code === 'string' ? error.code : undefined;
}

function trivialModule(): JsonObject {
  return {
    description: 'Canonical conformance module',
    inputSchema: Type.Object({}), outputSchema: Type.Object({}), execute: () => ({}),
  };
}

function contractedModule(contract: JsonObject): JsonObject {
  return {
    description: contract['description'] ?? 'Canonical conformance module',
    inputSchema: jsonSchemaToTypeBox(contract['input_schema'] as JsonObject),
    outputSchema: jsonSchemaToTypeBox(contract['output_schema'] as JsonObject),
    execute: () => ({ ok: true }),
  };
}

// D-139, PROTOCOL_SPEC §5.12.2/§5.12.5: real YAML and imported callables.
describe('binding_file_validation.json', () => {
  const data = fixture<{ file: { bindings: JsonObject[] } }>('binding_file_validation');
  it('handles every canonical case and expected field', () => guardCases(data, 9, ['error_code', 'module_ids']));
  for (const testCase of data.test_cases) {
    it(testCase.id, async () => {
      const root = mkdtempSync(join(tmpdir(), 'apcore-binding-validation-'));
      try {
        writeFileSync(join(root, 'typed.mjs'),
          'export function typed_greet(name) { return {greeting: `Hello ${name}`}; }\n' +
          'export const inputSchema = {type:"object",properties:{name:{type:"string"}},required:["name"]};\n' +
          'export const outputSchema = {type:"object",properties:{greeting:{type:"string"}}};\n');
        writeFileSync(join(root, 'untyped.mjs'), 'export function untyped_greet(name) { return {greeting:name}; }\n');
        const document = structuredClone(testCase.input.file);
        for (const binding of document.bindings) {
          const original = binding['target'] as string;
          if (original === 'fixture_targets:typed_greet') binding['target'] = `${join(root, 'typed.mjs')}:typed_greet`;
          else if (original === 'fixture_targets:untyped_greet') binding['target'] = `${join(root, 'untyped.mjs')}:untyped_greet`;
          else throw new Error(`Unhandled binding target ${original}`);
        }
        const file = join(root, 'fixture.binding.yaml');
        writeFileSync(file, yaml.dump(document));
        const registry = new Registry();
        let thrown: unknown;
        try { await new BindingLoader().loadBindings(file, registry); } catch (error) { thrown = error; }
        if ('error_code' in testCase.expected) expect(errorCode(thrown)).toBe(testCase.expected['error_code']);
        if ('module_ids' in testCase.expected) {
          expect(thrown).toBeUndefined();
          expect(registry.list()).toEqual(testCase.expected['module_ids']);
        }
      } finally { rmSync(root, { recursive: true, force: true }); }
    });
  }
});

// D-146, PROTOCOL_SPEC §9.8.2: isolate process-wide registrations via a fresh module.
describe('env_prefix_dispatch.json', () => {
  interface Input { namespaces: { name: string; env_prefix: string }[]; config_file?: JsonObject; env?: Record<string, string> }
  const data = fixture<Input>('env_prefix_dispatch');
  it('handles every canonical case and expected field', () => guardCases(data, 4, ['register_error_code', 'values', 'absent']));
  for (const testCase of data.test_cases) {
    it(testCase.id, async () => {
      const saved = Object.fromEntries(Object.entries(process.env).filter(([key]) => key.startsWith('APCORE')));
      const root = mkdtempSync(join(tmpdir(), 'apcore-env-prefix-'));
      try {
        for (const key of Object.keys(process.env)) if (key.startsWith('APCORE')) delete process.env[key];
        Object.assign(process.env, testCase.input.env ?? {});
        vi.resetModules();
        const { Config: FreshConfig } = await import('../src/config.js');
        let thrown: unknown;
        try {
          for (const registration of testCase.input.namespaces) {
            FreshConfig.registerNamespace({ name: registration.name, envPrefix: registration.env_prefix });
          }
        } catch (error) { thrown = error; }
        if ('register_error_code' in testCase.expected) {
          expect(errorCode(thrown)).toBe(testCase.expected['register_error_code']);
          return;
        }
        expect(thrown).toBeUndefined();
        const file = join(root, 'apcore.yaml');
        writeFileSync(file, yaml.dump(testCase.input.config_file));
        const config = FreshConfig.load(file);
        for (const [key, value] of Object.entries((testCase.expected['values'] ?? {}) as JsonObject)) expect(config.get(key)).toEqual(value);
        for (const key of (testCase.expected['absent'] ?? []) as string[]) expect(config.get(key) == null).toBe(true);
      } finally {
        for (const key of Object.keys(process.env)) if (key.startsWith('APCORE')) delete process.env[key];
        Object.assign(process.env, saved);
        rmSync(root, { recursive: true, force: true });
      }
    });
  }
});

// D-148, PROTOCOL_SPEC §2.5.1: standard bootstrap, never hand-wire the emitter.
describe('ephemeral_modules.json', () => {
  interface Operation {
    op: string; module_id?: string; file?: string;
    context?: { caller_id: string; identity: { id: string; type: string; roles: string[]; attrs: JsonObject } };
  }
  interface ExpectedEvent {
    event_type: string; module_id: string; payload?: JsonObject; payload_absent_keys?: string[]; identity_id?: string;
  }
  const data = fixture<{ operations: Operation[] }>('ephemeral_modules');
  it('handles every canonical case and expected field', () => guardCases(data, 7, ['events', 'secret_absent', 'error_code']));
  for (const testCase of data.test_cases) {
    it(testCase.id, async () => {
      const root = mkdtempSync(join(tmpdir(), 'apcore-ephemeral-'));
      const client = new APCore({ config: new Config({ sys_modules: { enabled: true, events: { enabled: true } } }), registry: new Registry({ extensionsDir: root }) });
      const events: ApCoreEvent[] = [];
      const ids = new Set(testCase.input.operations.map((operation) => operation.module_id ?? operation.file?.replace(/\//g, '.')).filter(Boolean));
      expect(client.events).not.toBeNull();
      client.events!.subscribe({ onEvent(event) {
        if (ids.has(event.moduleId ?? '') && ['apcore.registry.module_registered', 'apcore.registry.module_unregistered'].includes(event.eventType)) events.push(event);
      } });
      let thrown: unknown;
      try {
        for (const operation of testCase.input.operations) {
          let context: Context | undefined;
          if (operation.context) {
            const identity = operation.context.identity;
            context = Context.create(new Identity(identity.id, identity.type, identity.roles, identity.attrs)).child(operation.context.caller_id).child('fixture.registry');
          }
          try {
            switch (operation.op) {
              case 'register': await client.registry.register(operation.module_id!, trivialModule(), null, null, { context }); break;
              case 'unregister': client.registry.unregister(operation.module_id!, { context }); break;
              case 'register_internal': client.registry.registerInternal(operation.module_id!, trivialModule()); break;
              case 'discover': {
                const file = join(root, `${operation.file}.ts`);
                mkdirSync(dirname(file), { recursive: true });
                writeFileSync(file, 'export default {description:"Ephemeral discovery",inputSchema:{type:"object"},outputSchema:{type:"object"},execute(){return {};}};\n');
                await client.discover();
                break;
              }
              default: throw new Error(`Unhandled ephemeral operation ${operation.op}`);
            }
          } catch (error) { thrown = error; break; }
        }
        await client.events!.flush();
        if ('error_code' in testCase.expected) expect(errorCode(thrown)).toBe(testCase.expected['error_code']);
        else expect(thrown).toBeUndefined();
        const expectedEvents = testCase.expected['events'] as ExpectedEvent[];
        expect(events).toHaveLength(expectedEvents.length);
        for (const [index, expectedEvent] of expectedEvents.entries()) {
          const event = events[index];
          expect(event.eventType).toBe(expectedEvent.event_type);
          expect(event.moduleId).toBe(expectedEvent.module_id);
          for (const [key, value] of Object.entries(expectedEvent.payload ?? {})) expect(event.data[key]).toEqual(value);
          for (const key of expectedEvent.payload_absent_keys ?? []) expect(event.data).not.toHaveProperty(key);
          if (expectedEvent.identity_id) expect((event.data['identity'] as JsonObject)['id']).toBe(expectedEvent.identity_id);
        }
        if ('secret_absent' in testCase.expected) expect(JSON.stringify(events.map((event) => event.data))).not.toContain(testCase.expected['secret_absent']);
      } finally { rmSync(root, { recursive: true, force: true }); }
    });
  }
});

// D-149, PROTOCOL_SPEC §8.1: assertions use the canonical wire serializer.
describe('error_details_shape.json', () => {
  const data = fixture<{ inputs: JsonObject; call_module_id?: string }>('error_details_shape');
  it('handles every canonical case and expected field', () => guardCases(data, 5, ['error_code', 'errors', 'error_count', 'detail_keys_present', 'detail_keys_absent']));
  for (const testCase of data.test_cases) {
    it(testCase.id, async () => {
      const registry = new Registry();
      await registry.register(data.module_contract['module_id'] as string, contractedModule(data.module_contract));
      const executor = new Executor({ registry });
      let thrown: unknown;
      try { await executor.call(testCase.input.call_module_id ?? data.module_contract['module_id'] as string, testCase.input.inputs); } catch (error) { thrown = error; }
      expect(thrown).toBeInstanceOf(ModuleError);
      const wire = JSON.parse(JSON.stringify((thrown as ModuleError).toJSON())) as JsonObject;
      expect(wire['code']).toBe(testCase.expected['error_code']);
      const details = wire['details'] as JsonObject;
      if ('errors' in testCase.expected) {
        const errors = details['errors'] as JsonObject[];
        for (const item of errors) {
          expect(Object.keys(item).sort()).toEqual(['keyword', 'message', 'path']);
          expect(item['message']).toEqual(expect.any(String));
          expect((item['message'] as string).length).toBeGreaterThan(0);
        }
        for (const item of testCase.expected['errors'] as JsonObject[]) expect(errors).toContainEqual(expect.objectContaining(item));
      }
      if ('error_count' in testCase.expected) expect(details['errors']).toHaveLength(testCase.expected['error_count'] as number);
      for (const key of (testCase.expected['detail_keys_present'] ?? []) as string[]) expect(details).toHaveProperty(key);
      for (const key of (testCase.expected['detail_keys_absent'] ?? []) as string[]) expect(details).not.toHaveProperty(key);
    });
  }
});

function pointer(value: unknown, path: string): unknown {
  let current = value;
  for (const key of path.slice(1).split('/').map((part) => part.replace(/~1/g, '/').replace(/~0/g, '~'))) {
    if (current == null || typeof current !== 'object' || !(key in current)) return undefined;
    current = (current as JsonObject)[key];
  }
  return current;
}

function assertNoExtensionKeywords(value: unknown, propertyNames = false): void {
  if (Array.isArray(value)) { for (const item of value) assertNoExtensionKeywords(item); return; }
  if (value == null || typeof value !== 'object') return;
  for (const [key, child] of Object.entries(value)) {
    if (!propertyNames) expect(key.startsWith('x-'), `Unexpected extension keyword ${key}`).toBe(false);
    assertNoExtensionKeywords(child, key === 'properties');
  }
}

// D-140, PROTOCOL_SPEC §4.17/Appendix D.1: exercise the public profile exporter.
describe('export_profiles.json', () => {
  const data = fixture<{ profile: ExportProfile; annotations: JsonObject }>('export_profiles');
  it('handles every canonical case and expected field', () => guardCases(data, 5, ['paths', 'absent_paths', 'no_x_keywords_under']));
  for (const testCase of data.test_cases) {
    it(testCase.id, async () => {
      const contract = data.module_contract;
      const registry = new Registry();
      const module = contractedModule(contract);
      module['annotations'] = annotationsFromJSON(testCase.input.annotations);
      // Streaming declarations must represent real streaming modules.
      if (testCase.input.annotations['streaming']) module['stream'] = async function* () { yield { sent: true }; };
      await registry.register(contract['module_id'] as string, module);
      const output = new SchemaExporter().export({
        moduleId: contract['module_id'] as string, description: contract['description'] as string,
        inputSchema: contract['input_schema'] as JsonObject, outputSchema: contract['output_schema'] as JsonObject,
        definitions: {}, version: '1.0.0',
      }, testCase.input.profile, annotationsFromJSON(testCase.input.annotations));
      for (const [path, value] of Object.entries((testCase.expected['paths'] ?? {}) as JsonObject)) expect(pointer(output, path), path).toEqual(value);
      for (const path of (testCase.expected['absent_paths'] ?? []) as string[]) expect(pointer(output, path), path).toBeUndefined();
      if ('no_x_keywords_under' in testCase.expected) assertNoExtensionKeywords(pointer(output, testCase.expected['no_x_keywords_under'] as string));
    });
  }
});

// D-136, type-mapping §17.3 R5/R6: native TypeBox schemas, not JSON-Schema loading.
describe('json_input_native_types.json', () => {
  const data = fixture<{ inputs: JsonObject }>('json_input_native_types');
  it('handles every canonical case and expected field', () => guardCases(data, 4, ['output', 'error_code']));
  for (const testCase of data.test_cases) {
    it(testCase.id, async () => {
      const registry = new Registry();
      await registry.register(data.module_contract['module_id'] as string, {
        description: 'Native typed scheduling module',
        inputSchema: Type.Object({ when: Type.String({ format: 'date-time' }), request_id: Type.String({ format: 'uuid' }), level: Type.Union([Type.Literal('low'), Type.Literal('high')]) }),
        outputSchema: Type.Object({ accepted: Type.Boolean() }), execute: () => ({ accepted: true }),
      });
      let thrown: unknown;
      let output: unknown;
      try { output = await new Executor({ registry }).call(data.module_contract['module_id'] as string, testCase.input.inputs); } catch (error) { thrown = error; }
      if ('error_code' in testCase.expected) expect(errorCode(thrown)).toBe(testCase.expected['error_code']);
      if ('output' in testCase.expected) { expect(thrown).toBeUndefined(); expect(output).toEqual(testCase.expected['output']); }
    });
  }
});

// D-134/D-141, PROTOCOL_SPEC §12.8: collect checks using real Executor.validate().
describe('preflight_check_reporting.json', () => {
  interface Input { module_id: string; register?: boolean; acl_rules: ACLRule[]; default_effect: string; caller_id: string; inputs: JsonObject; implements_preflight?: boolean; implements_preview?: boolean; preview_returns_null?: boolean }
  const data = fixture<Input>('preflight_check_reporting');
  it('handles every canonical case and expected field', () => guardCases(data, 7, ['valid', 'failed_checks', 'passed_checks', 'checks_absent', 'optional_passed_checks', 'predicted_changes_count', 'predicted_changes_present']));
  for (const testCase of data.test_cases) {
    it(testCase.id, async () => {
      const registry = new Registry();
      const module = contractedModule(data.module_contract);
      module['execute'] = () => { throw new Error('Preflight must not execute the module'); };
      if (testCase.input.implements_preflight) module['preflight'] = () => data.module_contract['preflight_returns'];
      if (testCase.input.implements_preview) module['preview'] = (): PreviewResult | null => testCase.input.preview_returns_null ? null : { changes: [data.module_contract['preview_change'] as Change] };
      if (testCase.input.register !== false) await registry.register(testCase.input.module_id, module);
      const executor = new Executor({ registry, acl: new ACL(testCase.input.acl_rules, testCase.input.default_effect) });
      const context = Context.create(new Identity(testCase.input.caller_id, 'module')).child(testCase.input.caller_id);
      const result = await executor.validate(testCase.input.module_id, testCase.input.inputs, context);
      const names = result.checks.map((check) => check.check);
      expect(result.valid).toBe(testCase.expected['valid']);
      expect(result.checks.filter((check) => !check.passed).map((check) => check.check).sort()).toEqual([...(testCase.expected['failed_checks'] as string[])].sort());
      for (const name of (testCase.expected['passed_checks'] ?? []) as string[]) expect(result.checks).toContainEqual(expect.objectContaining({ check: name, passed: true }));
      for (const name of (testCase.expected['checks_absent'] ?? []) as string[]) expect(names).not.toContain(name);
      for (const name of (testCase.expected['optional_passed_checks'] ?? []) as string[]) {
        const check = result.checks.find((item) => item.check === name);
        if (check) { expect(check.passed).toBe(true); expect(check.warnings ?? []).toEqual([]); }
      }
      const serialized = JSON.parse(JSON.stringify(result)) as JsonObject;
      if (testCase.expected['predicted_changes_present']) expect(Array.isArray(serialized['predictedChanges'])).toBe(true);
      if ('predicted_changes_count' in testCase.expected) expect(serialized['predictedChanges']).toHaveLength(testCase.expected['predicted_changes_count'] as number);
    });
  }
});

// D-133, PROTOCOL_SPEC §12.7.5/A22: capture tokens inside actual module calls.
describe('timeout_cancellation.json', () => {
  interface ModuleSpec { kind: string; sleep_ms?: number; checks_token?: boolean; calls?: string; catches?: string | null; module_timeout_ms: number }
  interface Input { modules: Record<string, ModuleSpec>; call: string; config?: JsonObject; application_token: boolean; cancel_application_token_after_ms?: number }
  const data = fixture<Input>('timeout_cancellation');
  it('handles every canonical case and expected field', () => guardCases(data, 6, ['error_code', 'output', 'token_cancelled', 'returns_within_ms']));
  for (const testCase of data.test_cases) {
    it(testCase.id, async () => {
      const registry = new Registry();
      const tokens = new Map<string, CancelToken | null>();
      for (const [id, definition] of Object.entries(testCase.input.modules)) {
        if (!['sleeper', 'caller'].includes(definition.kind)) throw new Error(`Unhandled module kind ${definition.kind}`);
        await registry.register(id, {
          description: 'Cancellation conformance module', inputSchema: Type.Object({}), outputSchema: Type.Object({}, { additionalProperties: true }),
          resources: { timeout: definition.module_timeout_ms },
          async execute(_inputs: JsonObject, context: Context): Promise<JsonObject> {
            tokens.set(id, context.cancelToken);
            if (definition.kind === 'caller') {
              try { return await (context.executor as Executor).call(definition.calls!, {}, context); }
              catch (error) { if (definition.catches && errorCode(error) === definition.catches) return { caught: definition.catches }; throw error; }
            }
            const deadline = Date.now() + definition.sleep_ms!;
            while (Date.now() < deadline) {
              if (definition.checks_token && context.cancelToken?.isCancelled) throw new ExecutionCancelledError();
              await new Promise((resolve) => setTimeout(resolve, Math.min(10, Math.max(0, deadline - Date.now()))));
            }
            return { slept: true };
          },
        });
      }
      const executor = new Executor({ registry, config: new Config(testCase.input.config ?? {}) });
      const applicationToken = testCase.input.application_token ? new CancelToken() : null;
      if (applicationToken) tokens.set('application', applicationToken);
      const context = Context.create(null, null, applicationToken);
      const cancelTimer = testCase.input.cancel_application_token_after_ms === undefined ? undefined : setTimeout(() => applicationToken!.cancel(), testCase.input.cancel_application_token_after_ms);
      const started = performance.now();
      let output: unknown;
      let thrown: unknown;
      try { output = await executor.call(testCase.input.call, {}, context); } catch (error) { thrown = error; }
      finally { if (cancelTimer) clearTimeout(cancelTimer); }
      const elapsed = performance.now() - started;
      if ('error_code' in testCase.expected) expect(errorCode(thrown)).toBe(testCase.expected['error_code']);
      if ('output' in testCase.expected) { expect(thrown).toBeUndefined(); expect(output).toEqual(testCase.expected['output']); }
      for (const [id, cancelled] of Object.entries(testCase.expected['token_cancelled'] as JsonObject)) expect(tokens.get(id)?.isCancelled ?? false, id).toBe(cancelled);
      expect(elapsed).toBeLessThan(testCase.expected['returns_within_ms'] as number);
    });
  }
});
