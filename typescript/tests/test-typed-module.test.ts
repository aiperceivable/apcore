import { describe, expect, expectTypeOf, it, vi } from 'vitest';
import {
  APCore,
  CallbackApprovalHandler,
  Executor,
  Registry,
  createApprovalResult,
  type BoundModule,
  type StandardSchemaV1,
} from '../src/index.js';

const id = 'orders.quote';
const draft = 'https://json-schema.org/draft/2020-12/schema';
const wireInput = {
  $schema: draft,
  type: 'object',
  properties: { units: { type: 'string' } },
  required: ['units'],
  additionalProperties: false,
};
const wireOutput = {
  $schema: draft,
  type: 'object',
  properties: { total: { type: 'number' } },
  required: ['total'],
};

interface RawInput { units: string }
interface ParsedInput { units: number }
interface RawOutput { total: string }
interface ParsedOutput { total: number }

function schemas() {
  const nativeInput = vi.fn(() => Object.defineProperty({ ...wireInput }, '~standard', {
    value: { validate: () => ({ issues: [] }) },
  }));
  const nativeOutput = vi.fn(() => wireOutput);
  const input: StandardSchemaV1<RawInput, ParsedInput> = {
    '~standard': {
      version: 1,
      vendor: 'test-validator',
      async validate(value) {
        const units = Number((value as RawInput).units);
        return units > 0 ? { value: { units } } : { issues: [
          { message: 'Units must be positive', path: [{ key: 'units' }] },
          { message: 'Invalid allocation', path: ['options/~names', 0] },
        ] };
      },
      jsonSchema: {
        input: nativeInput,
        output: () => { throw new Error('An input declaration must use the input schema'); },
      },
    },
  };
  const output: StandardSchemaV1<RawOutput, ParsedOutput> = {
    '~standard': {
      version: 1,
      vendor: 'test-validator',
      async validate(value) {
        const total = Number((value as RawOutput).total);
        return total >= 0 ? { value: { total } } : { issues: [{ message: 'Total must be nonnegative', path: ['total'] }] };
      },
      jsonSchema: {
        input: () => { throw new Error('An output declaration must use the output schema'); },
        output: nativeOutput,
      },
    },
  };
  return { input, output, nativeInput, nativeOutput };
}

describe('APCore.define', () => {
  it('infers the types on both sides of validation and registers canonical schemas for both entry points', async () => {
    const client = new APCore();
    const { input, output, nativeInput, nativeOutput } = schemas();
    const module = await client.define({ id, description: 'Quote an order', readonly: true, input, output, execute(args) {
      expectTypeOf(args).toEqualTypeOf<ParsedInput>();
      return { total: String(args.units * 2) };
    } });
    expectTypeOf(module).toEqualTypeOf<BoundModule<RawInput, ParsedOutput>>();
    expectTypeOf(module.invoke).parameter(0).not.toEqualTypeOf<ParsedInput>();
    expect(module.id).toBe(id);
    expect(nativeInput).toHaveBeenCalledExactlyOnceWith({ target: 'draft-2020-12' });
    expect(nativeOutput).toHaveBeenCalledExactlyOnceWith({ target: 'draft-2020-12' });
    await expect(module.invoke({ units: '2.5' })).resolves.toEqual({ total: 5 });
    await expect(client.call(id, { units: '2.5' })).resolves.toEqual({ total: 5 });
    await expect(client.stream(id, { units: '2.5' }).next()).resolves.toMatchObject({ value: { total: 5 } });
    expect(client.getCallableDefinition(id)).toMatchObject({ inputSchema: wireInput, outputSchema: wireOutput });
    expect(client.registry.getDefinition(id)).toMatchObject({ annotations: { readonly: true } });
  });

  it('awaits registration and uses an explicit schema in place of native conversion', async () => {
    const client = new APCore();
    const { input, output, nativeInput, nativeOutput } = schemas();
    let finish!: (errors: string[]) => void;
    client.registry.setValidator({ validate: () => new Promise(resolve => { finish = resolve; }) });
    const typedInput = { '~standard': { ...input['~standard'], jsonSchema: undefined } };
    const handle = client.define({
      id, description: 'Quote an order', input: typedInput, output, inputSchema: wireInput, outputSchema: wireOutput,
      execute: ({ units }) => ({ total: String(units) }),
    });
    expect(client.registry.get(id)).toBeNull();
    finish([]);
    await expect(handle).resolves.toMatchObject({ id });
    expect(client.registry.get(id)).not.toBeNull();
    expect(nativeInput).not.toHaveBeenCalled();
    expect(nativeOutput).not.toHaveBeenCalled();
  });

  it('rejects declarations without compatible validation and export contracts', async () => {
    const client = new APCore();
    const { input, output } = schemas();
    const options = { id, description: 'Quote an order', input, output, execute: () => ({ total: '0' }) };
    const validationOnly = { '~standard': { ...input['~standard'], jsonSchema: undefined } };
    await expect(client.define({ ...options, input: validationOnly })).rejects.toThrow('Supply inputSchema explicitly');
    await expect(client.define({ ...options, inputSchema: { $schema: 'http://json-schema.org/draft-07/schema#' } }))
      .rejects.toThrow('must use Draft 2020-12');
    const wrongVersion = { '~standard': { ...input['~standard'], version: 2 } } as unknown as typeof input;
    await expect(client.define({ ...options, input: wrongVersion })).rejects.toThrow('must implement Standard Schema v1');
    await expect(client.define({ ...options, execute: null as unknown as typeof options.execute }))
      .rejects.toThrow('requires an execute function');
    expect(client.registry.get(id)).toBeNull();
  });

  it('preserves detailed Standard Schema issues in preflight and runtime failures', async () => {
    const client = new APCore();
    const { input, output } = schemas();
    const execute = vi.fn(() => ({ total: '-1' }));
    const module = await client.define({ id, description: 'Quote an order', input, output, execute });
    const errors = [
      { path: '/units', message: 'Units must be positive', keyword: 'standardSchema' },
      { path: '/options~1~0names/0', message: 'Invalid allocation', keyword: 'standardSchema' },
    ];
    const preflight = await module.validate({ units: '0' });
    expect(preflight.valid).toBe(false);
    expect(preflight.errors).toContainEqual(expect.objectContaining({ code: 'SCHEMA_VALIDATION_ERROR', details: { errors } }));
    await expect(module.invoke({ units: '0' })).rejects.toMatchObject({ code: 'SCHEMA_VALIDATION_ERROR', details: { errors } });
    await expect(client.call(id, { units: 1 })).rejects.toMatchObject({ code: 'SCHEMA_VALIDATION_ERROR' });
    expect(execute).not.toHaveBeenCalled();
    await expect(module.invoke({ units: '1' })).rejects.toMatchObject({
      code: 'SCHEMA_VALIDATION_ERROR', details: { errors: [{ path: '/total', message: 'Total must be nonnegative' }] },
    });
  });

  it('uses detached normalized input for final evaluation, approval and execution', async () => {
    const registry = new Registry();
    const approved: unknown[] = [];
    const evaluate = vi.fn(async ({ arguments: args }: { arguments: Readonly<Record<string, unknown>> }) => {
      const units = args['units'];
      Reflect.set(args, 'units', 100);
      return { decision: units === 6 ? 'require_approval' : 'deny' } as const;
    });
    const executor = new Executor({ registry, policyEvaluator: { evaluate }, approvalHandler: {
      supportsFinalScope: true,
      async requestApproval(request) {
        approved.push(request.arguments['units']);
        Reflect.set(request.arguments, 'units', 200);
        return createApprovalResult({ status: 'approved' });
      },
      async checkApproval() { return createApprovalResult({ status: 'rejected' }); },
    } });
    executor.useBefore((_name, args) => ({ units: String(Number(args['units']) * 2) }));
    executor.addStepMiddleware({ beforeStep(name, state) {
      if (name === 'execute') state.context.inputs = { units: 300 };
    } });
    const client = new APCore({ registry, executor });
    const { input, output } = schemas();
    const execute = vi.fn(({ units }: ParsedInput) => ({ total: String(units) }));
    const module = await client.define({ id, description: 'Quote an order', input, output, execute });
    await expect(module.invoke({ units: '3' })).resolves.toEqual({ total: 6 });
    expect(evaluate).toHaveBeenCalledOnce();
    expect(approved).toEqual([6]);
    expect(execute).toHaveBeenCalledExactlyOnceWith({ units: 6 }, expect.anything());
    await expect(module.invoke({ units: '4' })).rejects.toMatchObject({ code: 'POLICY_DENIED' });
    expect(approved).toHaveLength(1);
    expect(execute).toHaveBeenCalledOnce();
  });

  it('keeps default approval before middleware and checks Standard Schema before dispatch', async () => {
    const registry = new Registry();
    const observations: unknown[] = [];
    const approvalHandler = new CallbackApprovalHandler(async request => {
      observations.push(['approval', request.arguments['units']]);
      return createApprovalResult({ status: 'approved' });
    });
    const executor = new Executor({ registry, approvalHandler });
    executor.useBefore((_name, args) => {
      observations.push(['middleware', args['units']]);
      return { units: '4' };
    });
    const client = new APCore({ registry, executor });
    const { input, output } = schemas();
    const module = await client.define({
      id, description: 'Quote an order', input, output, annotations: { requiresApproval: true },
      execute: ({ units }) => {
        observations.push(['execute', units]);
        return { total: String(units) };
      },
    });
    await expect(module.invoke({ units: '2' })).resolves.toEqual({ total: 4 });
    expect(observations).toEqual([['approval', '2'], ['middleware', '2'], ['execute', 4]]);
  });

  it('rejects non-wire results and skips execution when the validation step is unavailable', async () => {
    const registry = new Registry();
    const client = new APCore({ registry, executor: new Executor({ registry, strategy: 'minimal' }) });
    const { input, output } = schemas();
    const execute = vi.fn(() => ({ total: '1' }));
    const module = await client.define({ id, description: 'Quote an order', input, output, execute });
    await expect(module.invoke({ units: '1' })).rejects.toThrow('requires built-in input validation');
    expect(execute).not.toHaveBeenCalled();

    const normalClient = new APCore();
    const invalidOutput: StandardSchemaV1<RawOutput, { time: Date }> = {
      '~standard': { version: 1, vendor: 'test-validator', validate: () => ({ value: { time: new Date() } }) },
    };
    const nonWire = await normalClient.define({
      id, description: 'Quote an order', input, output: invalidOutput, outputSchema: wireOutput, execute,
    });
    await expect(nonWire.invoke({ units: '1' })).rejects.toMatchObject({ code: 'SCHEMA_VALIDATION_ERROR' });
  });
});
