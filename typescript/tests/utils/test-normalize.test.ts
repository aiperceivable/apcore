import { describe, it, expect } from 'vitest';
import { normalizeToCanonicalId } from '../../src/utils/normalize.js';

describe('normalizeToCanonicalId', () => {
  it('converts PascalCase to snake_case', () => {
    expect(normalizeToCanonicalId('HttpJsonParser', 'typescript')).toBe('http_json_parser');
  });

  it('converts camelCase to snake_case', () => {
    expect(normalizeToCanonicalId('getDBUrl', 'typescript')).toBe('get_db_url');
  });

  it('handles already snake_case', () => {
    expect(normalizeToCanonicalId('my_module', 'python')).toBe('my_module');
  });

  it('handles dot-separated IDs for python', () => {
    expect(normalizeToCanonicalId('api.handler.TaskSubmit', 'python')).toBe('api.handler.task_submit');
  });

  it('handles :: separator for rust', () => {
    expect(normalizeToCanonicalId('executor::validator::DbParams', 'rust')).toBe('executor.validator.db_params');
  });

  it('handles dot-separated IDs for go', () => {
    expect(normalizeToCanonicalId('api.Handler', 'go')).toBe('api.handler');
  });

  it('handles dot-separated IDs for java', () => {
    expect(normalizeToCanonicalId('com.example.MyModule', 'java')).toBe('com.example.my_module');
  });

  it('handles acronyms like HTML', () => {
    expect(normalizeToCanonicalId('HTMLParser', 'typescript')).toBe('html_parser');
  });

  it('throws on empty localId', () => {
    expect(() => normalizeToCanonicalId('', 'python')).toThrow('non-empty string');
  });

  it('throws on unsupported language', () => {
    expect(() => normalizeToCanonicalId('Foo', 'ruby')).toThrow("Unsupported language 'ruby'");
  });

  it('throws when result is not a valid canonical ID', () => {
    expect(() => normalizeToCanonicalId('123invalid', 'python')).toThrow('does not conform');
  });

  it('preserves lowercase segments', () => {
    expect(normalizeToCanonicalId('api.handler', 'typescript')).toBe('api.handler');
  });
});
