import { describe, it, expect } from 'vitest';
import { detectIdConflicts } from '../../src/registry/conflicts.js';

describe('detectIdConflicts', () => {
  const reserved = new Set(['system', 'internal', 'core']);

  it('returns null when there are no conflicts', () => {
    const existing = new Set(['executor.email.send']);
    const result = detectIdConflicts('executor.slack.post', existing, reserved);
    expect(result).toBeNull();
  });

  it('detects exact duplicate IDs', () => {
    const existing = new Set(['executor.email.send']);
    const result = detectIdConflicts('executor.email.send', existing, reserved);
    expect(result).not.toBeNull();
    expect(result!.type).toBe('duplicate_id');
    expect(result!.severity).toBe('error');
    expect(result!.message).toContain('executor.email.send');
  });

  it('detects reserved word as first segment', () => {
    const existing = new Set<string>();
    const result = detectIdConflicts('system.check', existing, reserved);
    expect(result).not.toBeNull();
    expect(result!.type).toBe('reserved_word');
    expect(result!.severity).toBe('error');
    expect(result!.message).toContain('system');
  });

  it('allows reserved word in non-first segments', () => {
    const existing = new Set<string>();
    const result = detectIdConflicts('executor.system.check', existing, reserved);
    expect(result).toBeNull();
  });

  it('allows reserved word in last segment', () => {
    const existing = new Set<string>();
    const result = detectIdConflicts('api.foo.internal', existing, reserved);
    expect(result).toBeNull();
  });

  it('detects case collision without lowercaseMap', () => {
    const existing = new Set(['Executor.Email.Send']);
    const result = detectIdConflicts('executor.email.send', existing, reserved);
    expect(result).not.toBeNull();
    expect(result!.type).toBe('case_collision');
    expect(result!.severity).toBe('warning');
    expect(result!.message).toContain('Executor.Email.Send');
  });

  it('detects case collision with lowercaseMap', () => {
    const existing = new Set(['Executor.Email.Send']);
    const lcMap = new Map([['executor.email.send', 'Executor.Email.Send']]);
    const result = detectIdConflicts('executor.email.send', existing, reserved, lcMap);
    expect(result).not.toBeNull();
    expect(result!.type).toBe('case_collision');
    expect(result!.severity).toBe('warning');
  });

  it('returns null when lowercaseMap has no collision', () => {
    const existing = new Set(['executor.email.send']);
    const lcMap = new Map([['executor.email.send', 'executor.email.send']]);
    const result = detectIdConflicts('executor.email.send', existing, reserved, lcMap);
    // This is a duplicate_id, not a case collision — duplicate check runs first
    expect(result).not.toBeNull();
    expect(result!.type).toBe('duplicate_id');
  });

  it('returns null for lowercaseMap with same ID (not a collision)', () => {
    const existing = new Set(['executor.slack.post']);
    const lcMap = new Map([['executor.slack.post', 'executor.slack.post']]);
    const result = detectIdConflicts('executor.email.send', existing, reserved, lcMap);
    expect(result).toBeNull();
  });

  it('prioritises duplicate over reserved word', () => {
    // If the ID is both a duplicate and has a reserved first segment, duplicate wins
    const existing = new Set(['system.check']);
    const result = detectIdConflicts('system.check', existing, reserved);
    expect(result!.type).toBe('duplicate_id');
  });

  it('detects reserved word when first segment is reserved (non-duplicate)', () => {
    const existing = new Set<string>();
    const result = detectIdConflicts('core.check', existing, reserved);
    expect(result).not.toBeNull();
    expect(result!.type).toBe('reserved_word');
    expect(result!.message).toContain('core');
  });

  it('prioritises reserved word over case collision', () => {
    const existing = new Set(['System.Check']);
    const result = detectIdConflicts('system.check', existing, reserved);
    expect(result!.type).toBe('reserved_word');
  });
});
