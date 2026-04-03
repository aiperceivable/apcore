# Task 2: Handler Registry

Add `_condition_handlers` class-level registry and `register_condition()` API.

## Python
- `_condition_handlers: ClassVar[dict[str, ACLConditionHandler]] = {}`
- `@classmethod register_condition(cls, key, handler)`

## TypeScript
- `private static conditionHandlers = new Map<string, ACLConditionHandler>()`
- `static registerCondition(key, handler)`

## Rust
- `static CONDITION_HANDLERS: LazyLock<RwLock<HashMap<String, Box<dyn ACLConditionHandler>>>>`
- `pub fn register_condition(key, handler)`

## Rules
- Same key replaces previous handler
- Thread-safe
