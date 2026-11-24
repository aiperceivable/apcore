/**
 * Extension point system for apcore.
 *
 * Provides a centralized mechanism to register, query, and wire custom
 * extensions (discoverers, middleware, ACL providers, span exporters, and
 * module validators) into the apcore runtime.
 */

import type { ACL } from './acl.js';
import type { ApprovalHandler } from './approval.js';
import { InvalidInputError } from './errors.js';
import type { Executor } from './executor.js';
import { Middleware } from './middleware/index.js';
import { TracingMiddleware } from './observability/tracing.js';
import type { Span, SpanExporter } from './observability/tracing.js';
import type { Discoverer, ModuleValidator, Registry } from './registry/registry.js';

/**
 * Delegates span export to multiple underlying exporters.
 */
class CompositeExporter implements SpanExporter {
  private _exporters: SpanExporter[];

  constructor(exporters: SpanExporter[]) {
    this._exporters = exporters;
  }

  export(span: Span): void {
    for (const exp of this._exporters) {
      try {
        exp.export(span);
      } catch (e) {
        console.warn('[apcore:extensions] Span exporter failed:', e);
      }
    }
  }
}

/**
 * Describes a named slot where extensions can be registered.
 */
export interface ExtensionPoint {
  readonly name: string;
  readonly description: string;
  readonly multiple: boolean;
}

/** Type guard: checks if value has a `discover` method. */
function isDiscoverer(value: unknown): value is Discoverer {
  return typeof value === 'object' && value !== null && typeof (value as Record<string, unknown>)['discover'] === 'function';
}

/** Type guard: checks if value has a `validate` method. */
function isModuleValidator(value: unknown): value is ModuleValidator {
  return typeof value === 'object' && value !== null && typeof (value as Record<string, unknown>)['validate'] === 'function';
}

/** Type guard: checks if value has at least one middleware method (before, after, onError). */
function isMiddleware(value: unknown): value is Middleware {
  if (typeof value !== 'object' || value === null) return false;
  const obj = value as Record<string, unknown>;
  return typeof obj['before'] === 'function' || typeof obj['after'] === 'function' || typeof obj['onError'] === 'function';
}

/** Type guard: checks if value has a `check` method (ACL). */
function isACL(value: unknown): value is ACL {
  return typeof value === 'object' && value !== null && typeof (value as Record<string, unknown>)['check'] === 'function';
}

/** Type guard: checks if value has an `export` method (SpanExporter). */
function isSpanExporter(value: unknown): value is SpanExporter {
  return typeof value === 'object' && value !== null && typeof (value as Record<string, unknown>)['export'] === 'function';
}

/** Type guard: checks if value has requestApproval and checkApproval methods. */
function isApprovalHandler(value: unknown): value is ApprovalHandler {
  if (typeof value !== 'object' || value === null) return false;
  const obj = value as Record<string, unknown>;
  return typeof obj['requestApproval'] === 'function' && typeof obj['checkApproval'] === 'function';
}

type TypeChecker = (value: unknown) => boolean;

interface InternalExtensionPoint extends ExtensionPoint {
  readonly typeCheck: TypeChecker;
  readonly typeName: string;
}

function builtInPoints(): Map<string, InternalExtensionPoint> {
  return new Map<string, InternalExtensionPoint>([
    ['discoverer', {
      name: 'discoverer',
      description: 'Custom module discovery strategy',
      multiple: false,
      typeCheck: isDiscoverer,
      typeName: 'Discoverer',
    }],
    ['middleware', {
      name: 'middleware',
      description: 'Execution middleware',
      multiple: true,
      typeCheck: isMiddleware,
      typeName: 'Middleware',
    }],
    ['acl', {
      name: 'acl',
      description: 'Access control provider',
      multiple: false,
      typeCheck: isACL,
      typeName: 'ACL',
    }],
    ['span_exporter', {
      name: 'span_exporter',
      description: 'Tracing span exporter',
      multiple: true,
      typeCheck: isSpanExporter,
      typeName: 'SpanExporter',
    }],
    ['module_validator', {
      name: 'module_validator',
      description: 'Custom module validation',
      multiple: false,
      typeCheck: isModuleValidator,
      typeName: 'ModuleValidator',
    }],
    ['approval_handler', {
      name: 'approval_handler',
      description: 'Approval handler for Step 4.5 gate',
      multiple: false,
      typeCheck: isApprovalHandler,
      typeName: 'ApprovalHandler',
    }],
  ]);
}

/**
 * Manages extension points and their registered implementations.
 *
 * Pre-registers six built-in extension points: discoverer, middleware,
 * acl, span_exporter, module_validator, and approval_handler.
 */
export class ExtensionManager {
  private _points: Map<string, InternalExtensionPoint>;
  private _extensions: Map<string, unknown[]>;

  constructor() {
    this._points = builtInPoints();
    this._extensions = new Map<string, unknown[]>();
    for (const name of this._points.keys()) {
      this._extensions.set(name, []);
    }
  }

  /**
   * Return the named extension point, or throw if it is not registered.
   *
   * D-108: an UNKNOWN point name is an error; an EMPTY point is not. The
   * distinction used to be blurred by the contract's `### Errors: No errors
   * raised` row, which was written about the empty case — one SDK read it as
   * covering the unknown case too and answered a misspelled name with a
   * silent null/[]/false, so `get('middlewares')` wired nothing and first
   * surfaced as a bug at `apply()`. A bare `Error` carries no `code`, so a
   * caller could not tell this apart from any other failure either.
   */
  private requirePoint(pointName: string): InternalExtensionPoint {
    const point = this._points.get(pointName);
    if (point === undefined) {
      throw new InvalidInputError(
        `Unknown extension point: '${pointName}'. Available: ${[...this._points.keys()].sort().join(', ')}`,
      );
    }
    return point;
  }

  /**
   * Register an extension for the given extension point.
   *
   * @throws InvalidInputError if point_name is unknown.
   * @throws TypeError if extension does not satisfy the type constraint.
   */
  register(pointName: string, extension: unknown): void {
    const point = this.requirePoint(pointName);

    if (!point.typeCheck(extension)) {
      throw new TypeError(
        `Extension for '${pointName}' must satisfy the ${point.typeName} interface`,
      );
    }

    if (point.multiple) {
      this._extensions.get(pointName)!.push(extension);
    } else {
      this._extensions.set(pointName, [extension]);
    }
  }

  /**
   * Return the single extension for a non-multiple point, or null.
   *
   * @throws InvalidInputError if pointName is not a registered point. A point
   * that exists but holds nothing returns null (D-108).
   */
  get(pointName: string): unknown | null {
    this.requirePoint(pointName);
    const exts = this._extensions.get(pointName)!;
    return exts.length > 0 ? exts[0] : null;
  }

  /**
   * Return all extensions for a multiple-type point.
   *
   * @throws InvalidInputError if pointName is not a registered point. A point
   * that exists but holds nothing returns [] (D-108).
   */
  getAll(pointName: string): unknown[] {
    this.requirePoint(pointName);
    return [...this._extensions.get(pointName)!];
  }

  /**
   * Remove a specific extension from an extension point.
   * Returns true if found and removed, false otherwise.
   *
   * @throws InvalidInputError if pointName is not a registered point. An
   * extension the point does not hold returns false (D-108).
   */
  unregister(pointName: string, extension: unknown): boolean {
    this.requirePoint(pointName);
    const exts = this._extensions.get(pointName)!;
    const idx = exts.indexOf(extension);
    if (idx === -1) return false;
    exts.splice(idx, 1);
    return true;
  }

  /**
   * Return all registered extension points.
   */
  listPoints(): ExtensionPoint[] {
    return [...this._points.values()].map(({ name, description, multiple }) => ({
      name,
      description,
      multiple,
    }));
  }

  /**
   * Wire all registered extensions into the given registry and executor.
   */
  apply(registry: Registry, executor: Executor): void {
    // Discoverer
    const discoverer = this.get('discoverer') as Discoverer | null;
    if (discoverer !== null) {
      registry.setDiscoverer(discoverer);
    }

    // Module validator
    const validator = this.get('module_validator') as ModuleValidator | null;
    if (validator !== null) {
      registry.setValidator(validator);
    }

    // ACL
    const acl = this.get('acl') as ACL | null;
    if (acl !== null) {
      executor.setAcl(acl);
    }

    // Approval handler
    const approvalHandler = this.get('approval_handler') as ApprovalHandler | null;
    if (approvalHandler !== null) {
      executor.setApprovalHandler(approvalHandler);
    }

    // Middleware
    for (const mw of this.getAll('middleware')) {
      executor.use(mw as Middleware);
    }

    // Span exporters: find existing TracingMiddleware and set exporter(s)
    const exporters = this.getAll('span_exporter') as SpanExporter[];
    if (exporters.length > 0) {
      const tracingMw = this._findTracingMiddleware(executor);
      if (tracingMw !== null) {
        if (exporters.length === 1) {
          tracingMw.setExporter(exporters[0]);
        } else {
          tracingMw.setExporter(new CompositeExporter(exporters));
        }
      } else {
        console.warn(
          '[apcore:extensions] span_exporter extensions registered but no TracingMiddleware found in executor middleware chain',
        );
      }
    }
  }

  private _findTracingMiddleware(executor: Executor): TracingMiddleware | null {
    for (const mw of executor.middlewares) {
      if (mw instanceof TracingMiddleware) {
        return mw;
      }
    }
    return null;
  }
}
