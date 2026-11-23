/**
 * Function adapter classes for the middleware system.
 */

import type { Context } from '../context.js';
import { Middleware } from './base.js';

export type BeforeCallback = (
  moduleId: string,
  inputs: Record<string, unknown>,
  context: Context,
) => Record<string, unknown> | null;

export type AfterCallback = (
  moduleId: string,
  inputs: Record<string, unknown>,
  output: Record<string, unknown>,
  context: Context,
) => Record<string, unknown> | null;

export class BeforeMiddleware extends Middleware {
  private _callback: BeforeCallback;

  constructor(callback: BeforeCallback, priority: number = 100) {
    super(priority);
    this._callback = callback;
  }

  override before(
    moduleId: string,
    inputs: Record<string, unknown>,
    context: Context,
  ): Record<string, unknown> | null {
    return this._callback(moduleId, inputs, context);
  }
}

export class AfterMiddleware extends Middleware {
  private _callback: AfterCallback;

  constructor(callback: AfterCallback, priority: number = 100) {
    super(priority);
    this._callback = callback;
  }

  override after(
    moduleId: string,
    inputs: Record<string, unknown>,
    output: Record<string, unknown>,
    context: Context,
  ): Record<string, unknown> | null {
    return this._callback(moduleId, inputs, output, context);
  }
}
