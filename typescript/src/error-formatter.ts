/**
 * ErrorFormatterRegistry -- per-adapter error formatting (§8.8).
 *
 * Allows adapters to register custom formatters that transform ModuleErrors
 * into adapter-specific output shapes. Falls back to error.toJSON() when no
 * formatter is registered for the given adapter.
 */

import { ModuleError } from './errors.js';
import { ErrorFormatterDuplicateError } from './errors.js';

export interface ErrorFormatter {
  format(error: ModuleError, context?: unknown): Record<string, unknown>;
}

export class ErrorFormatterRegistry {
  private static readonly _registry = new Map<string, ErrorFormatter>();

  /**
   * Register a formatter for the given adapter name.
   * Throws ErrorFormatterDuplicateError if a formatter is already registered.
   */
  static register(adapterName: string, formatter: ErrorFormatter): void {
    if (ErrorFormatterRegistry._registry.has(adapterName)) {
      throw new ErrorFormatterDuplicateError(adapterName);
    }
    ErrorFormatterRegistry._registry.set(adapterName, formatter);
  }

  /**
   * Retrieve the registered formatter for an adapter, or undefined if none.
   */
  static get(adapterName: string): ErrorFormatter | undefined {
    return ErrorFormatterRegistry._registry.get(adapterName);
  }

  /**
   * Format an error using the registered formatter for the given adapter.
   * Falls back to error.toJSON() if no formatter is registered.
   */
  static format(adapterName: string, error: ModuleError, context?: unknown): Record<string, unknown> {
    const formatter = ErrorFormatterRegistry._registry.get(adapterName);
    if (formatter !== undefined) {
      return formatter.format(error, context);
    }
    return error.toJSON();
  }

  /**
   * Remove a registered formatter. Used primarily in tests.
   */
  static unregister(adapterName: string): void {
    ErrorFormatterRegistry._registry.delete(adapterName);
  }

  /**
   * Clear all registered formatters. Used primarily in tests.
   */
  static clear(): void {
    ErrorFormatterRegistry._registry.clear();
  }
}
