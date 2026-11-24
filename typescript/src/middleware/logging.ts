/**
 * LoggingMiddleware for structured module call logging.
 *
 * @deprecated Use `ObsLoggingMiddleware` from `apcore/observability` instead.
 * `ObsLoggingMiddleware` provides the same functionality with structured JSON
 * output, configurable log levels, and `_secret_`-key redaction via
 * `ContextLogger`. `LoggingMiddleware` will be removed in a future major version.
 */

import type { Context } from '../context.js';
import { Middleware } from './base.js';

export interface Logger {
  info(message: string, extra?: Record<string, unknown>): void;
  error(message: string, extra?: Record<string, unknown>): void;
}

const defaultLogger: Logger = {
  info(message: string, extra?: Record<string, unknown>) {
    console.info(`[apcore:middleware.logging] ${message}`, extra ?? '');
  },
  error(message: string, extra?: Record<string, unknown>) {
    console.error(`[apcore:middleware.logging] ${message}`, extra ?? '');
  },
};

/**
 * The `inputs` field of a log record: the executor's captured
 * `context.redactedInputs`, or nothing when no capture exists (the middleware
 * was invoked outside the pipeline). Never the raw inputs — D-131.
 */
function capturedInputs(context: Context): { inputs?: Record<string, unknown> } {
  return context.redactedInputs != null ? { inputs: context.redactedInputs } : {};
}

/** @deprecated Use `ObsLoggingMiddleware` from `apcore/observability`. */
export class LoggingMiddleware extends Middleware {
  private _logger: Logger;
  private _logInputs: boolean;
  private _logOutputs: boolean;
  private _logErrors: boolean;

  constructor(options?: {
    logger?: Logger;
    logInputs?: boolean;
    logOutputs?: boolean;
    logErrors?: boolean;
  }) {
    super(700);
    this._logger = options?.logger ?? defaultLogger;
    this._logInputs = options?.logInputs ?? true;
    this._logOutputs = options?.logOutputs ?? true;
    this._logErrors = options?.logErrors ?? true;
  }

  override before(
    moduleId: string,
    _inputs: Record<string, unknown>,
    context: Context,
  ): null {
    context.data['_apcore.mw.logging.start_time'] = performance.now();

    if (this._logInputs) {
      // PROTOCOL_SPEC 10.6.1 requirement 5 (D-131): the CAPTURED inputs, never
      // the raw argument; omitted when nothing was captured.
      this._logger.info(`[${context.traceId}] START ${moduleId}`, {
        traceId: context.traceId,
        moduleId,
        callerId: context.callerId,
        ...capturedInputs(context),
      });
    }

    return null;
  }

  override after(
    moduleId: string,
    _inputs: Record<string, unknown>,
    _output: Record<string, unknown>,
    context: Context,
  ): null {
    const startTime = (context.data['_apcore.mw.logging.start_time'] as number) ?? performance.now();
    const durationMs = performance.now() - startTime;

    if (this._logOutputs) {
      // D-131: the captured output only — the raw `output` argument carries
      // `x-sensitive` fields in clear text and is never logged.
      this._logger.info(
        `[${context.traceId}] END ${moduleId} (${durationMs.toFixed(2)}ms)`,
        {
          traceId: context.traceId,
          moduleId,
          durationMs,
          ...(context.redactedOutput != null ? { output: context.redactedOutput } : {}),
        },
      );
    }

    return null;
  }

  override onError(
    moduleId: string,
    _inputs: Record<string, unknown>,
    error: Error,
    context: Context,
  ): null {
    if (this._logErrors) {
      this._logger.error(`[${context.traceId}] ERROR ${moduleId}: ${error}`, {
        traceId: context.traceId,
        moduleId,
        error: String(error),
        ...capturedInputs(context),
      });
    }

    return null;
  }
}
