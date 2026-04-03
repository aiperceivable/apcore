# Task 1: Handler Protocol

Define `ACLConditionHandler` protocol/interface/trait in all 3 SDKs.

## Python
- `SyncACLConditionHandler` — `@runtime_checkable` Protocol with `evaluate(value, context) -> bool`
- `AsyncACLConditionHandler` — `@runtime_checkable` Protocol with `async evaluate(value, context) -> bool`
- `ACLConditionHandler = SyncACLConditionHandler | AsyncACLConditionHandler`

## TypeScript
- `ACLConditionHandler` interface with `evaluate(value: unknown, context: Context): boolean | Promise<boolean>`

## Rust
- `#[async_trait] pub trait ACLConditionHandler: Send + Sync` with `async fn evaluate(&self, value: &Value, ctx: &Context<Value>) -> bool`

## Files
- NEW: `acl_handlers.py`, `acl-handlers.ts`, `acl_handlers.rs`
