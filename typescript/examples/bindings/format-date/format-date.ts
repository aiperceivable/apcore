/**
 * Target function for the format_date YAML binding.
 *
 * Two contracts matter here:
 *
 * 1. `BindingLoader` invokes the resolved target as `func(inputs, context)` —
 *    a single inputs object plus the execution `Context`. (apcore-python
 *    unpacks `**inputs` instead, because it can introspect the signature;
 *    TypeScript cannot, so the object form is the cross-language-safe shape.)
 * 2. `auto_schema: true` infers the module's schemas from the exported
 *    `inputSchema` / `outputSchema` bindings below (or, per-symbol,
 *    `formatDateStringInputSchema` / `formatDateStringOutputSchema`).
 *    Without them, loading fails with `BindingSchemaInferenceFailedError`.
 */

import { Type } from '@sinclair/typebox';
import type { Context } from 'apcore-js';

export const inputSchema = Type.Object({
  dateString: Type.String({ description: 'Input date, ISO 8601 (e.g. "2024-01-15")' }),
  outputFormat: Type.String({ description: 'strftime-style format (e.g. "%B %d, %Y")' }),
});

export const outputSchema = Type.Object({
  formatted: Type.String({ description: 'The reformatted date string' }),
});

export function formatDateString(
  inputs: Record<string, unknown>,
  _context: Context,
): { formatted: string } {
  const dateString = inputs['dateString'] as string;
  const outputFormat = inputs['outputFormat'] as string;
  const dt = new Date(dateString);

  // Simple format substitution (subset of strftime)
  const formatted = outputFormat
    .replace('%Y', String(dt.getUTCFullYear()))
    .replace('%m', String(dt.getUTCMonth() + 1).padStart(2, '0'))
    .replace('%d', String(dt.getUTCDate()).padStart(2, '0'))
    .replace(
      '%B',
      dt.toLocaleString('en-US', { month: 'long', timeZone: 'UTC' }),
    );

  return { formatted };
}
