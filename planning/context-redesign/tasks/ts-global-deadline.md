# Task: TypeScript globalDeadline Field

## Goal

Add `globalDeadline: number | null` field to the TypeScript `Context` class, aligned with Python and Rust SDKs. The field represents an absolute deadline as epoch seconds (float). It defaults to `null` and is NOT serialized.

## Files Involved

### TypeScript SDK
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-typescript/src/context.ts` (add field + constructor param)
- **Create or modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-typescript/tests/context-global-deadline.test.ts`

## Steps

### Step 1: Write failing test (AC-020)

Create `apcore-typescript/tests/context-global-deadline.test.ts`:

```typescript
import { describe, it, expect } from "vitest";
import { Context } from "../src/context";

describe("Context.globalDeadline", () => {
  it("AC-020: globalDeadline is accessible on Context", () => {
    const ctx = Context.create({
      executor: null as any,
      globalDeadline: 1234.5,
    });
    expect(ctx.globalDeadline).toBe(1234.5);
  });

  it("globalDeadline defaults to null", () => {
    const ctx = Context.create({ executor: null as any });
    expect(ctx.globalDeadline).toBeNull();
  });

  it("globalDeadline accepts null explicitly", () => {
    const ctx = Context.create({
      executor: null as any,
      globalDeadline: null,
    });
    expect(ctx.globalDeadline).toBeNull();
  });

  it("globalDeadline is number type (epoch seconds)", () => {
    const deadline = Date.now() / 1000 + 30; // 30 seconds from now
    const ctx = Context.create({
      executor: null as any,
      globalDeadline: deadline,
    });
    expect(typeof ctx.globalDeadline).toBe("number");
    expect(ctx.globalDeadline).toBeGreaterThan(0);
  });
});
```

### Step 2: Add field to Context class

Modify `apcore-typescript/src/context.ts`:

```typescript
// Add to the Context class field declarations:
readonly globalDeadline: number | null;

// Add to the constructor parameter list:
constructor(
    // ...existing params...
    globalDeadline: number | null = null,
) {
    // ...existing assignments...
    this.globalDeadline = globalDeadline;
}
```

If Context uses a config/options object pattern in the constructor, add to the options interface:

```typescript
interface ContextOptions {
    // ...existing fields...
    globalDeadline?: number | null;
}
```

And in the constructor body:

```typescript
this.globalDeadline = options.globalDeadline ?? null;
```

### Step 3: Update Context.create() factory if applicable

If `Context.create()` accepts options, add `globalDeadline` to the accepted parameters and pass through to the constructor.

### Step 4: Ensure child contexts inherit globalDeadline

If `Context.child()` exists, verify that `globalDeadline` is propagated from parent:

```typescript
child(/* ... */): Context {
    return new Context({
        // ...existing fields...
        globalDeadline: this.globalDeadline,
    });
}
```

### Step 5: Run TypeScript tests

```bash
cd apcore-typescript && npx vitest run tests/context-global-deadline.test.ts
```

### Step 6: Run full test suite to verify no regressions

```bash
cd apcore-typescript && npx vitest run
```

## Acceptance Criteria

- [x] **AC-020**: TypeScript Context has `globalDeadline` field (unit test: create with `1234.5`, assert accessible)
- [ ] Field defaults to `null` when not provided
- [ ] Field type is `number | null`
- [ ] Child contexts inherit `globalDeadline` from parent
- [ ] All existing tests continue to pass

## Dependencies

- **Depends on:** none
- **Required by:** serialization

## Estimated Time

1 hour
