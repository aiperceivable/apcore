/** Project supported module values onto their JSON representation. */

function isPlainRecord(value: object): boolean {
  const prototype = Object.getPrototypeOf(value);
  return prototype === null || prototype === Object.prototype;
}

function isRecordLike(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object'
    && (isPlainRecord(value) || Object.prototype.toString.call(value) === '[object Object]');
}

function projectValue(value: unknown, ancestors: Set<object>): unknown {
  if (value instanceof Date) return Date.prototype.toJSON.call(value);
  if (!Array.isArray(value) && !isRecordLike(value)) return value;
  if (ancestors.has(value)) return value;

  ancestors.add(value);
  try {
    if (Array.isArray(value)) {
      const elements = value.map((element: unknown) => projectValue(element, ancestors));
      return elements.every((element, index) => Object.is(element, value[index])) ? value : elements;
    }

    let changed = !isPlainRecord(value);
    const entries = Object.entries(value).map(([key, original]) => {
      const projected = projectValue(original, ancestors);
      changed ||= !Object.is(original, projected);
      return [key, projected] as const;
    });
    return changed ? Object.fromEntries(entries) : value;
  } finally {
    ancestors.delete(value);
  }
}

/** Return the module's data fields as an object, or null for a non-map root. */
export function normalizeModuleObject(value: unknown): Record<string, unknown> | null {
  if (!isRecordLike(value)) return null;
  return projectValue(value, new Set()) as Record<string, unknown>;
}
