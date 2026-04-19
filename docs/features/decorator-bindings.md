# Decorator and YAML Bindings

!!! info "Scope: Python SDK"
    This page documents idiomatic Python SDK APIs (`@module` decorator, `BindingLoader`). The protocol itself defines no decorator semantics — each language SDK provides equivalent ergonomics in its own idiom (e.g., TypeScript decorators or `defineModule()`, Rust attribute macros). YAML binding files, however, are language-neutral and portable across SDKs.

## Overview

Two complementary approaches for module creation: the `@module` decorator for zero-boilerplate function wrapping, and YAML bindings via `BindingLoader` for declarative, code-free module registration. Both approaches produce `FunctionModule` instances that participate fully in the executor pipeline (ACL, middleware, validation, async support). The decorator system includes automatic runtime model generation from function signatures, while the binding system supports four distinct schema resolution modes.

## Requirements

### Decorator System
- Provide a `@module` decorator that works in three forms: bare (`@module`), with arguments (`@module(id="x")`), and as a function call (`module(func, id="x")`).
- Automatically generate Pydantic input and output models from function type annotations via `_generate_input_model()` and `_generate_output_model()`.
- Detect and skip `Context`-typed parameters in input model generation (type-based detection, not name-based).
- Skip `self`, `cls`, and `*args` parameters. When `**kwargs` is present, create the input model with `extra="allow"`.
- Handle multiple return types: `dict` produces a permissive model, `BaseModel` subclass is returned directly, `None` produces an empty permissive model, and other types produce a model with a single `result` field.
- Support async functions: `FunctionModule.execute` must be a coroutine function when the wrapped function is async.
- Provide three description sources with priority: explicit `description` parameter, first line of docstring, fallback to `"Module {func_name}"`.
- Auto-generate module IDs from `__module__` and `__qualname__`, with sanitization (lowercased, non-alphanumeric replaced, digit-leading segments prefixed with underscore).
- Support optional registry integration for immediate registration on decoration.

### Binding System
- Load YAML binding files containing a `bindings` list of module declarations.
- Resolve `module.path:callable` target strings to actual Python callables via dynamic import.
- Support class method binding: `module:ClassName.method` auto-instantiates the class (must have a no-arg constructor) and resolves the bound method.
- Support four schema modes:
  1. `auto_schema: true` -- Infer schemas from function type annotations.
  2. Inline `input_schema`/`output_schema` -- Build Pydantic models from JSON Schema properties.
  3. `schema_ref` -- Load schema from an external YAML file (relative path resolution from binding file directory).
  4. Default (no schema key) -- Falls back to auto-schema inference.
- Handle unsupported JSON Schema features (`oneOf`, `anyOf`, `allOf`, `$ref`, `format`) gracefully by creating permissive models with `extra="allow"`.
- Support directory scanning via `load_binding_dir()` with configurable glob pattern (default `*.binding.yaml`).
- Fail fast on first error during binding loading.

## Technical Design

### Decorator Architecture

The `module()` function uses a dual-purpose design pattern:

```python
# Bare decorator -- func_or_none receives the function
@module
def greet(name: str) -> str: ...

# With arguments -- func_or_none is None, returns a decorator
@module(id="custom.id", tags=["email"])
def greet(name: str) -> str: ...

# Function call form -- func_or_none receives the function, id is set
fm = module(greet, id="custom.id")
```

The internal `_wrap()` function:
1. Generates `module_id` (explicit or auto-generated via `_make_auto_id()`).
2. Creates a `FunctionModule` with inferred or provided schemas.
3. Optionally registers with the provided registry.
4. Either attaches `FunctionModule` as `func.apcore_module` (decorator forms) or returns it directly (function call form).

#### FunctionModule

`FunctionModule` wraps a Python function for the apcore executor pipeline:

- **Schemas**: `input_schema` and `output_schema` are Pydantic `BaseModel` subclasses, either auto-generated or explicitly provided.
- **Execute closures**: Two separate closures are created at construction time -- one for sync functions and one for async -- so that `inspect.iscoroutinefunction(fm.execute)` returns the correct value. Both closures unpack inputs as keyword arguments and inject the `Context` if a Context-typed parameter was detected.
- **Result normalization**: `_normalize_result()` converts return values: `None` -> `{}`, `dict` -> passthrough, `BaseModel` -> `model_dump()`, other -> `{"result": value}`.

#### Type Inference Engine

`_generate_input_model(func)`:
- Uses `typing.get_type_hints()` with `include_extras=True` to resolve annotations (handles `from __future__ import annotations`).
- Iterates parameters, skipping `self`/`cls`, `*args`, `**kwargs`, and `Context`-typed parameters.
- Raises `FuncMissingTypeHintError` for untyped parameters.
- Creates a dynamic Pydantic model via `create_model()`, with `extra="allow"` if `**kwargs` was present.

`_generate_output_model(func)`:
- Examines the return type annotation.
- Maps: `dict`/`dict[str, T]` -> permissive model, `BaseModel` subclass -> returned directly, `None` -> empty permissive model, other types -> model with `result` field.
- Raises `FuncMissingReturnTypeError` if no return annotation exists.

### Binding Architecture

```
YAML File
  |
  +--> BindingLoader.load_bindings()
         |
         +--> Parse YAML, validate structure
         |
         +--> For each binding entry:
         |      |
         |      +--> resolve_target("module.path:callable")
         |      |      Import module, getattr callable
         |      |      For "Class.method": instantiate class, get bound method
         |      |
         |      +--> Determine schema mode:
         |      |      auto_schema -> _generate_input/output_model()
         |      |      inline schema -> _build_model_from_json_schema()
         |      |      schema_ref -> load external YAML, then build
         |      |      default -> try auto_schema
         |      |
         |      +--> Create FunctionModule
         |      +--> Register with Registry
         |
         +--> Return list[FunctionModule]
```

#### JSON Schema to Pydantic Conversion

`_build_model_from_json_schema()` maps JSON Schema types to Python types:
- `string` -> `str`, `integer` -> `int`, `number` -> `float`, `boolean` -> `bool`, `array` -> `list`, `object` -> `dict`.
- Fields listed in `required` array are marked as required (default `...`), others default to `None`.
- Unsupported top-level features (`oneOf`, `anyOf`, `allOf`, `$ref`, `format`) result in a permissive model with `extra="allow"`.

### Error Hierarchy

All binding-related errors inherit from `ModuleError`:
- `FuncMissingTypeHintError` -- Parameter lacks type annotation (code: `FUNC_MISSING_TYPE_HINT`).
- `FuncMissingReturnTypeError` -- Function lacks return type annotation (code: `FUNC_MISSING_RETURN_TYPE`).
- `BindingInvalidTargetError` -- Target string missing `:` separator (code: `BINDING_INVALID_TARGET`).
- `BindingModuleNotFoundError` -- Python module cannot be imported (code: `BINDING_MODULE_NOT_FOUND`).
- `BindingCallableNotFoundError` -- Callable not found in module (code: `BINDING_CALLABLE_NOT_FOUND`).
- `BindingNotCallableError` -- Resolved attribute is not callable (code: `BINDING_NOT_CALLABLE`).
- `BindingSchemaMissingError` -- Auto-schema failed on untyped callable (code: `BINDING_SCHEMA_MISSING`).
- `BindingFileInvalidError` -- YAML file issues (missing, empty, parse error, structural) (code: `BINDING_FILE_INVALID`).

## Language Equivalents

The `@module` decorator and `BindingLoader` are Python SDK idioms. TypeScript and Rust provide equivalent ergonomics in their own idiom:

=== "Python"
    ```python
    from apcore import APCore

    client = APCore()

    # Decorator form
    @client.module(id="text.upper", description="Convert text to uppercase", tags=["text"])
    def to_upper(text: str) -> dict:
        return {"result": text.upper()}

    # Bare decorator (auto-generates ID from module path + function name)
    @client.module
    def greet(name: str) -> dict:
        """Greet a user by name."""
        return {"message": f"Hello, {name}!"}

    # YAML binding (declarative, no code changes needed)
    # bindings.yaml:
    #   bindings:
    #     - module_id: "text.upper"
    #       target: "myapp.handlers:to_upper"
    #       description: "Convert text to uppercase"
    from apcore.bindings import BindingLoader
    from apcore.registry import Registry
    registry = Registry()
    BindingLoader().load_bindings("bindings.yaml", registry)
    ```
=== "TypeScript"
    ```typescript
    import { APCore } from "apcore-js";

    const client = new APCore();

    // defineModule() call form (TypeScript primary idiom)
    client.module({
        id: "text.upper",
        description: "Convert text to uppercase",
        tags: ["text"],
        inputSchema: { type: "object", properties: { text: { type: "string" } } },
        outputSchema: { type: "object", properties: { result: { type: "string" } } },
        execute: ({ text }: { text: string }) => ({ result: text.toUpperCase() }),
    });

    // Decorator form (TypeScript decorators, requires experimentalDecorators)
    // @apmodule({ id: "text.upper", description: "..." })
    // class UpperModule implements Module { ... }
    ```
=== "Rust"
    ```rust
    use apcore::APCore;
    use apcore::module::Module;
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use async_trait::async_trait;
    use serde_json::{json, Value};

    // Rust primary idiom: implement the Module trait
    struct UpperModule;

    #[async_trait]
    impl Module for UpperModule {
        fn description(&self) -> &str { "Convert text to uppercase" }
        fn input_schema(&self) -> Value {
            json!({"type": "object", "properties": {"text": {"type": "string"}}})
        }
        fn output_schema(&self) -> Value {
            json!({"type": "object", "properties": {"result": {"type": "string"}}})
        }
        async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let text = inputs["text"].as_str().unwrap_or("").to_uppercase();
            Ok(json!({"result": text}))
        }
    }

    let mut client = APCore::new();
    client.register("text.upper", Box::new(UpperModule)).unwrap();
    ```

## Dependencies

- `apcore.context.Context` -- Injected into wrapped functions when a Context-typed parameter is detected.
- `apcore.registry.Registry` -- Module registration for both decorator and binding paths.
- `apcore.errors` -- 8 error classes for decorator and binding failure modes.

??? info "Python SDK reference"
    The following tables are **not protocol requirements** — they document the Python SDK's source layout and runtime dependencies for implementers/users of `apcore-python`.

    **Source files:**

    | File | Lines | Purpose |
    |------|-------|---------|
    | `src/apcore/decorator.py` | 264 | `@module` decorator, `FunctionModule`, type inference helpers, auto-ID generation |
    | `src/apcore/bindings.py` | 220 | `BindingLoader` with YAML parsing, target resolution, schema mode handling |

    **Runtime dependencies:**

    - `pydantic` -- `BaseModel`, `ConfigDict`, `create_model` for dynamic model generation.
    - `inspect` (stdlib) -- Function signature introspection, parameter kind detection, coroutine function detection.
    - `typing` (stdlib) -- `get_type_hints()` for annotation resolution with forward reference support.
    - `re` (stdlib) -- Regex for auto-ID sanitization.
    - `importlib` (stdlib) -- Dynamic module import for target resolution in bindings.
    - `pathlib` (stdlib) -- Path operations for binding file and schema_ref resolution.
    - `yaml` (PyYAML) -- YAML parsing for binding files and schema references.

## Contract: module

### Inputs
- `func_or_none` (callable/None, positional) — the function to wrap; `None` when used as `@module(id=...)` (argument form)
- `id` (str, optional) — explicit module ID; auto-generated from `__module__.__qualname__` when absent
- `description` (str, optional) — human-readable description; falls back to first line of docstring, then `"Module {func_name}"`
- `input_schema` (type/BaseModel subclass, optional) — explicit Pydantic input model; auto-generated when absent
- `output_schema` (type/BaseModel subclass, optional) — explicit Pydantic output model; auto-generated when absent
- `tags` (list[str], optional) — searchable tags
- `version` (str, optional) — semantic version string
- `registry` (Registry, optional) — if provided, the resulting `FunctionModule` is immediately registered

### Errors
- `FuncMissingTypeHintError(code=FUNC_MISSING_TYPE_HINT)` — a parameter lacks a type annotation and `input_schema` was not provided
- `FuncMissingReturnTypeError(code=FUNC_MISSING_RETURN_TYPE)` — function lacks a return type annotation and `output_schema` was not provided

### Returns
- Decorator forms (`@module`, `@module(...)`): returns the original function with `.apcore_module` attribute attached
- Function-call form (`module(func, ...)`): returns the `FunctionModule` instance directly

### Properties
- async: false (the decorator itself is synchronous; `FunctionModule.execute` is async if the wrapped function is async)
- thread_safe: true (module creation is idempotent; no shared state mutated unless `registry` is provided)
- pure: false when `registry` is provided (registration mutates registry state)

## Contract: BindingLoader.load_bindings

### Inputs
- `path` (str/Path, required) — path to a YAML binding file; must exist and contain a `bindings` list
- `registry` (Registry, required) — registry to register the loaded modules into

### Errors
- `BindingFileInvalidError(code=BINDING_FILE_INVALID)` — file not found, empty, invalid YAML, missing `bindings` key, non-list `bindings`, or missing `module_id`/`target` per entry
- `BindingInvalidTargetError(code=BINDING_INVALID_TARGET)` — target string missing `:` separator
- `BindingModuleNotFoundError(code=BINDING_MODULE_NOT_FOUND)` — Python module in target path cannot be imported
- `BindingCallableNotFoundError(code=BINDING_CALLABLE_NOT_FOUND)` — callable not found in the imported module
- `BindingNotCallableError(code=BINDING_NOT_CALLABLE)` — resolved attribute is not callable
- `BindingSchemaMissingError(code=BINDING_SCHEMA_MISSING)` — auto-schema inference failed because callable lacks type hints
- `FuncMissingTypeHintError(code=FUNC_MISSING_TYPE_HINT)` — parameter lacks annotation during auto-schema
- `FuncMissingReturnTypeError(code=FUNC_MISSING_RETURN_TYPE)` — return type absent during auto-schema

### Returns
- On success: `list[FunctionModule]` — all successfully loaded and registered modules

### Properties
- async: false
- thread_safe: false (mutates registry)
- pure: false (reads filesystem, imports Python modules, mutates registry)
- idempotent: false (calling twice re-registers modules, raising a duplicate error from the Registry)

## Contract: BindingLoader.load_binding_dir

### Inputs
- `directory` (str/Path, required) — directory to scan; must exist
- `registry` (Registry, required) — registry to register modules into
- `pattern` (str, optional) — glob pattern for binding files; default `"*.binding.yaml"`

### Errors
- `BindingFileInvalidError(code=BINDING_FILE_INVALID)` — nonexistent directory, or any file within fails to parse (fail-fast on first error)

### Returns
- On success: `list[FunctionModule]` — all modules from all matched files; empty list if no files match

### Properties
- async: false
- thread_safe: false (mutates registry)
- pure: false (reads filesystem)
- idempotent: false

## Testing Strategy

### Decorator Tests (`tests/test_decorator.py`)

- **Error classes**: All 8 error classes instantiate correctly, have correct codes, include expected details, and inherit from `ModuleError`. Cross-cutting parametrized test verifies inheritance and code attributes for all error classes.
- **_generate_input_model()**: Simple primitives, default values, `Optional[str]`, union types (`str | int`), `list[str]`, `dict[str, int]`, `Literal` with validation, `Annotated` with `Field` constraints, nested `BaseModel` parameters, Context parameter skipping (type-based, not name-based), `self` skipping, `*args` skipping, `**kwargs` producing `extra="allow"`, missing type hint error, forward reference `NameError` mapped to `FuncMissingTypeHintError`, `*args` + `**kwargs` only, empty function, multiple defaults, and `from __future__ import annotations` compatibility.
- **_generate_output_model()**: Bare `dict`, typed `dict[str, Any]`, `BaseModel` subclass returned directly, `str`/`int`/`list[str]` wrapped in result field, `None` return producing empty permissive model, missing return type error, and result field invariant.
- **_has_context_param()**: Function with Context detected (True, param_name), function without Context (False, None), detection is type-based (works with any parameter name), and non-Context named "context" not detected.
- **FunctionModule constructor**: Input/output schemas are BaseModel subclasses, module_id stored correctly, description priority chain (explicit > docstring > fallback), multiline docstring uses first line only, optional attributes stored.
- **Sync execute**: Correct function call, dict passthrough, None -> `{}`, BaseModel -> `model_dump()`, string/int -> `{"result": value}`, Context injection, no Context injection when absent, exception propagation, `iscoroutinefunction` returns False.
- **Async execute**: Correct await, dict passthrough, None/non-dict/BaseModel handling, Context injection, `iscoroutinefunction` returns True, exception propagation.
- **@module with args**: Returns original function, attaches `.apcore_module`, correct id, registry integration, function remains callable, tags/version stored.
- **Bare @module**: Returns original function, attaches `.apcore_module`, auto-generates id containing function name and module path.
- **module() function call form**: Returns `FunctionModule`, registry integration, correct schemas.
- **_make_auto_id()**: Combines `__module__` + `__qualname__`, replaces `<locals>.`, lowercased, non-alphanumeric replaced, digit-leading segments prefixed.
- **Integration**: Full pipeline through `Executor.call()` for sync, async, Context injection, non-dict returns, BaseModel params, bare decorator, and function call form.

### Binding Tests (`tests/test_bindings.py`)

- **YAML parsing**: Single and multiple binding entries, empty file error, missing `bindings` key, non-list `bindings`, missing `module_id`/`target`, YAML syntax errors.
- **Target resolution**: Function resolution (`os.path:join`), class method resolution with auto-instantiation, class requiring constructor args error, missing colon separator, nonexistent module, nonexistent callable, non-callable attribute.
- **Schema modes**: `auto_schema` using type inference, `auto_schema` with untyped callable error, inline schema model creation, inline schema with untyped callable, inline basic type mapping (string/integer/number/boolean), required array marking, unsupported features producing permissive model, `schema_ref` loading external file, `schema_ref` file not found error.
- **Registration and integration**: `load_bindings` registers all modules, returns FunctionModule list, directory scanning, nonexistent directory error, empty directory returns empty list, fail-fast on first error.
- **Public API exports**: `BindingLoader` importable from `apcore`.
- **End-to-end integration**: BindingLoader -> Registry -> Executor.call() producing correct output with a dynamically created Python module and YAML binding file.
