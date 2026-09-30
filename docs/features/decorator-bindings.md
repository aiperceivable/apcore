---
description: "Two ways to turn existing code into modules: function wrapping (Python @module, TypeScript module(), Rust FunctionModule) and language-neutral YAML binding files loaded by BindingLoader."
---

# Decorator and YAML Bindings

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §5.11 Function-based Module Definition / §5.12 External Schema Binding.


## Overview

Besides implementing the Module interface directly, apcore offers two ways to turn existing code into modules:

- **Function wrapping** (§5.11) — wrap a function as a module in code. Python has the `@module` decorator (schemas inferred from type hints); TypeScript has the `module({...})` function (explicit TypeBox schemas); Rust has `FunctionModule` and the `APCore::module(...)` shorthand (explicit JSON schemas, closure handler).
- **Binding files** (§5.12) — declare modules in a YAML file that maps a `module_id` to a `target` callable, with no change to the target's code. Every SDK ships a `BindingLoader`.

Both paths produce `FunctionModule` instances that run through the full execution pipeline (ACL, approval, middleware, validation).

## Function wrapping

=== "Python"
    ```python
    # myapp/handlers.py
    from apcore import APCore, Context
    from apcore.decorator import module


    # Decorator with arguments: registers into the given registry.
    client = APCore()


    @client.module(id="text.upper", description="Convert text to uppercase", tags=["text"])
    def to_upper(text: str) -> dict:
        return {"result": text.upper()}


    # Bare decorator: auto-generates the ID from __module__ + __qualname__
    # ("myapp.handlers.greet"), takes the description from the docstring, and
    # attaches the module as greet.apcore_module (it does not register it).
    @module
    def greet(name: str, context: Context) -> dict:
        """Greet a user by name."""
        return {"message": f"Hello, {name}!", "caller": context.caller_id}


    client.register(greet.apcore_module.module_id, greet.apcore_module)

    # Function-call form: returns the FunctionModule itself.
    def shout(text: str) -> str:
        return text.upper() + "!"


    shout_module = module(shout, id="text.shout")
    client.register("text.shout", shout_module)

    print(client.call("text.upper", {"text": "hi"}))  # {'result': 'HI'}
    ```
=== "TypeScript"
    ```typescript
    import { Type } from "@sinclair/typebox";
    import { APCore } from "apcore-js";

    const client = new APCore();

    // module() needs an explicit id and TypeBox schemas: JavaScript cannot
    // derive either from a function at runtime.
    client.module({
      id: "text.upper",
      description: "Convert text to uppercase",
      tags: ["text"],
      inputSchema: Type.Object({ text: Type.String() }),
      outputSchema: Type.Object({ result: Type.String() }),
      execute: (inputs) => ({ result: String(inputs.text).toUpperCase() }),
    });

    console.log(await client.call("text.upper", { text: "hi" })); // { result: 'HI' }
    ```
=== "Rust"
    ```rust
    use std::collections::HashMap;

    use apcore::{APCore, FunctionModule, ModuleAnnotations, ModuleError};
    use serde_json::json;

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let client = APCore::new();

        let upper = FunctionModule::with_description(
            ModuleAnnotations::default(),
            json!({ "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"] }),
            json!({ "type": "object", "properties": { "result": { "type": "string" } } }),
            "Convert text to uppercase",
            None,
            vec!["text".to_string()],
            "1.0.0",
            HashMap::new(),
            Vec::new(),
            |inputs, _ctx| {
                Box::pin(async move {
                    let text = inputs["text"].as_str().unwrap_or("").to_uppercase();
                    Ok(json!({ "result": text }))
                })
            },
        );
        client.register("text.upper", Box::new(upper))?;

        let out = client.call("text.upper", json!({ "text": "hi" }), None, None).await?;
        println!("{out}"); // {"result":"HI"}
        Ok(())
    }
    ```

### Python: `@module` and schema inference

`apcore.decorator.module()` works as a bare decorator (`@module`), a decorator with arguments (`@module(id="x", ...)`), and a function call (`module(func, id="x")`). The decorator forms return the original function with the module attached as `func.apcore_module`; the call form (any call that passes the function together with `id=` or `registry=`) returns the `FunctionModule`. Passing `registry=` registers the module immediately.

Two registering shortcuts take arguments only (always call them, even with no arguments): `client.module(...)` registers on that `APCore` client, and the package-level `apcore.module(...)` registers on a default process-wide client (or on `registry=` if given).

- **Input schema** — a Pydantic model built from the parameters' type hints. `self` / `cls`, `*args` and any `Context`-typed parameter (detected by type, not name) are skipped; the `Context` is injected at call time. `**kwargs` makes the model accept extra fields. A parameter with no type hint raises `FUNC_MISSING_TYPE_HINT`.
- **Output schema** — from the return annotation: `dict` gives a permissive model, a `BaseModel` subclass is used as is, `None` gives an empty permissive model, any other type gives a model with a single `result` field. A missing return annotation raises `FUNC_MISSING_RETURN_TYPE`.
- **Result normalisation** — `None` → `{}`, `dict` unchanged, `BaseModel` → `model_dump()`, anything else → `{"result": value}`.
- **Description** — the `description` argument, else the first docstring line, else `"Module <name>"`.
- **Auto ID** — from `__module__` and `__qualname__`, lowercased, non-alphanumerics replaced with `_`, digit-leading segments prefixed with `_` (§5.11.6).
- **Async** — an `async def` function produces a module whose `execute` is a coroutine function.

## Binding files

A binding file is YAML with a `bindings` list (schema: `schemas/binding.schema.json`):

```yaml
# bindings/email.binding.yaml
bindings:
  - module_id: "email.send"
    target: "myapp.services.email:send_email"
    description: "Send an email"
    input_schema:
      type: object
      properties:
        to: { type: string, description: "Recipient address" }
        subject: { type: string, description: "Subject line" }
      required: [to, subject]
    output_schema:
      type: object
      properties:
        message_id: { type: string }
    annotations:
      destructive: false
      idempotent: false
    tags: [email]

  - module_id: "email.send_template"
    target: "myapp.services.email:EmailService.send_template"
    description: "Send an email from a template"
    auto_schema: true

  - module_id: "email.lookup"
    target: "myapp.services.email:lookup"
    schema_ref: "../schemas/email.lookup.schema.yaml"
```

| Field | Required | Meaning |
|---|---|---|
| `module_id` | yes | Canonical module ID (`^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$`, at most 192 chars) |
| `target` | yes | The callable, `module.path:callable` or `module.path:ClassName.method` (the class is instantiated with no arguments) |
| `description` | recommended | Plain-text description |
| `input_schema` / `output_schema` | one schema mode | Inline JSON Schema |
| `auto_schema` | one schema mode | `true` / `"permissive"` or `"strict"` — infer schemas from the target |
| `schema_ref` | one schema mode | Path to an external schema file, relative to the binding file |
| `annotations`, `tags`, `version`, `metadata`, `documentation`, `display` | no | As for any module; `display` is the surface overlay of §5.13 |

The file may also carry a top-level `spec_version` (default `"1.0"`).

### Schema modes

Exactly one mode applies per entry; combining them raises `BINDING_SCHEMA_MODE_CONFLICT`.

1. **Inline** — `input_schema` and `output_schema`.
2. **`schema_ref`** — load both from an external schema file.
3. **`auto_schema: true`** (or `"permissive"`) — infer from the target. Python reads the function's type hints; TypeScript reads the target module's exported `inputSchema` / `outputSchema` (or `<name>InputSchema` / `<name>OutputSchema`); Rust takes the schemas carried by a `typed_handler` (derived with `schemars`). If nothing can be inferred, `BINDING_SCHEMA_INFERENCE_FAILED`.
4. **`auto_schema: "strict"`** — as 3, and the inferred schema must be OpenAI/Anthropic strict-compatible, otherwise `BINDING_STRICT_SCHEMA_INCOMPATIBLE`.
5. **No schema key** — treated as implicit `auto_schema: true`. When inference finds nothing, Python raises `BINDING_SCHEMA_INFERENCE_FAILED`, TypeScript and Rust fall back to a permissive object schema.

`auto_schema: false` with no other mode leaves no schema and raises `BINDING_SCHEMA_INFERENCE_FAILED`.

### Target resolution

| SDK | How `target` is resolved |
|---|---|
| Python | `importlib` import of the module path, then attribute lookup. `BindingLoader(trusted_package_prefixes={...})` restricts which module paths may be imported. |
| TypeScript | Dynamic `import()` of the module specifier (a package name or path; `..` segments and `file:` URLs are rejected), then export lookup. `new BindingLoader({ trustedPackagePrefixes: [...] })` restricts specifiers. |
| Rust | No dynamic loading: `target` is a key into a handler map you pass to `register_into_with_handlers` / `register_into_with_typed_handlers`. |

### Loading binding files

Loading is always an explicit call; no SDK scans a bindings directory at start-up (§5.12.6).

| SDK | Single file | Directory |
|---|---|---|
| Python | `BindingLoader().load_bindings(file_path, registry, config=None)` | `load_binding_dir(dir_path=None, registry=None, pattern=None, *, config=None)` |
| TypeScript | `await loader.loadBindings(filePath, registry, config?)` | `await loader.loadBindingDir(dirPath, registry, pattern?, config?)` |
| Rust | `loader.load_from_yaml(path)` (or `load_from_file` for JSON), then `register_into_with_handlers` / `register_into_with_typed_handlers` | `loader.load_binding_dir(dir, pattern)` / `load_binding_dir_with_config(dir, pattern, config)` |

The directory and pattern resolve as **explicit argument > config > default**:

| Setting | Config key | Default |
|---|---|---|
| directory | `bindings.dir` (env `APCORE_BINDINGS_DIR`) | `./bindings` |
| filename pattern | `bindings.pattern` (env `APCORE_BINDINGS_PATTERN`) | `*.binding.yaml` |

The pattern is matched against each file name (not recursively) with Algorithm A25 (`*` and `?` only). A missing directory raises `BINDING_FILE_INVALID`; a pattern that matches nothing yields no modules. Passing a `Config` also applies the `validation.binding.*` limits (§9.1.2), which are unconstrained by default. Loading stops at the first error.

=== "Python"
    ```python
    from apcore import BindingLoader, Config, Registry

    config = Config.load("apcore.yaml")  # may set bindings.dir / bindings.pattern
    registry = Registry()

    modules = BindingLoader().load_binding_dir(registry=registry, config=config)
    print([m.module_id for m in modules])
    ```
=== "TypeScript"
    ```typescript
    import { BindingLoader, Config, Registry } from "apcore-js";

    const config = Config.load("apcore.yaml"); // may set bindings.dir / bindings.pattern
    const registry = new Registry();

    const modules = await new BindingLoader().loadBindingDir(undefined, registry, undefined, config);
    console.log(modules.map((m) => m.moduleId));
    ```
=== "Rust"
    ```rust
    use std::collections::HashMap;
    use std::path::Path;

    use apcore::{typed_handler, BindingLoader, ModuleError, Registry};
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    #[derive(Deserialize, JsonSchema)]
    struct UpperInput {
        text: String,
    }

    #[derive(Serialize, JsonSchema)]
    struct UpperOutput {
        result: String,
    }

    fn main() -> Result<(), ModuleError> {
        // bindings/text.binding.yaml:
        //   bindings:
        //     - module_id: "text.upper"
        //       target: "text:to_upper"
        //       auto_schema: true
        let mut loader = BindingLoader::new();
        loader.load_from_yaml(Path::new("bindings/text.binding.yaml"))?;

        // `target` strings are keys into this map; typed_handler derives the
        // input/output schemas that auto_schema uses.
        let mut handlers = HashMap::new();
        handlers.insert(
            "text:to_upper".to_string(),
            typed_handler(|input: UpperInput| Ok(UpperOutput { result: input.text.to_uppercase() })),
        );

        let registry = Registry::new();
        let count = loader.register_into_with_typed_handlers(&registry, handlers)?;
        println!("registered {count} module(s)");
        Ok(())
    }
    ```

## Errors

| Code | Python / TypeScript class | Cause |
|---|---|---|
| `FUNC_MISSING_TYPE_HINT` | `FuncMissingTypeHintError` | A wrapped function's parameter has no type hint |
| `FUNC_MISSING_RETURN_TYPE` | `FuncMissingReturnTypeError` | A wrapped function has no return annotation |
| `BINDING_FILE_INVALID` | `BindingFileInvalidError` | Binding file or directory missing, empty, unparseable, or structurally invalid |
| `BINDING_INVALID_TARGET` | `BindingInvalidTargetError` | `target` lacks `:`, or is outside the trusted prefixes |
| `BINDING_MODULE_NOT_FOUND` | `BindingModuleNotFoundError` | The module path cannot be imported (Rust: no handler for the target) |
| `BINDING_CALLABLE_NOT_FOUND` | `BindingCallableNotFoundError` | The callable is not in the module |
| `BINDING_NOT_CALLABLE` | `BindingNotCallableError` | The resolved attribute is not callable |
| `BINDING_SCHEMA_INFERENCE_FAILED` | `BindingSchemaInferenceFailedError` | No schema mode yields a schema |
| `BINDING_SCHEMA_MODE_CONFLICT` | `BindingSchemaModeConflictError` | More than one schema mode on one entry |
| `BINDING_STRICT_SCHEMA_INCOMPATIBLE` | `BindingStrictSchemaIncompatibleError` | `auto_schema: "strict"` and the schema cannot be made strict |

Rust returns these as `ModuleError` with the corresponding `ErrorCode`. A `module_id` that is already registered fails with the registry's `DUPLICATE_MODULE_ID`.

??? info "Python SDK reference"
    Not a protocol requirement — the relevant `apcore-python` source files.

    | File | Purpose |
    |------|---------|
    | `src/apcore/decorator.py` | `module()`, `FunctionModule`, type-hint schema inference, auto-ID generation |
    | `src/apcore/bindings.py` | `BindingLoader`: YAML parsing, target resolution, schema modes, directory loading |

## Contract: module (Python)

### Inputs
- `func_or_none` (callable/None, positional-only) — the function to wrap; `None` in the `@module(...)` form
- `id` (str, optional) — module ID; auto-generated from `__module__` / `__qualname__` when absent
- `description`, `documentation` (str, optional) — description falls back to the first docstring line, then `"Module <name>"`
- `annotations` (dict, optional), `tags` (list[str], optional), `version` (str, default `"1.0.0"`), `metadata` (dict, optional), `display` (dict, optional), `examples` (list, optional)
- `registry` (Registry, optional) — register the resulting `FunctionModule` immediately

### Errors
- `FuncMissingTypeHintError` (`FUNC_MISSING_TYPE_HINT`) — a parameter lacks a type hint
- `FuncMissingReturnTypeError` (`FUNC_MISSING_RETURN_TYPE`) — the function lacks a return annotation
- Registry errors (e.g. `DUPLICATE_MODULE_ID`) when `registry` is given

### Returns
- `@module` / `@module(...)`: the original function, with `.apcore_module` attached
- `module(func, id=...)` or `module(func, registry=...)`: the `FunctionModule`

### Properties
- async: false (the wrapper's `execute` is async when the function is)
- thread_safe: true
- pure: false when `registry` is given

## Contract: BindingLoader.load_bindings

### Inputs
- `file_path` (required) — a binding YAML file
- `registry` (required) — the registry to register into
- `config` (optional) — supplies the `validation.binding.*` limits

### Errors
- `BINDING_FILE_INVALID`, `BINDING_INVALID_TARGET`, `BINDING_MODULE_NOT_FOUND`, `BINDING_CALLABLE_NOT_FOUND`, `BINDING_NOT_CALLABLE`, `BINDING_SCHEMA_INFERENCE_FAILED`, `BINDING_SCHEMA_MODE_CONFLICT`, `BINDING_STRICT_SCHEMA_INCOMPATIBLE` — see [Errors](#errors)
- Registry errors such as `DUPLICATE_MODULE_ID`

### Returns
- The list of `FunctionModule`s registered (Python, TypeScript). Rust splits loading (`load_from_yaml`) from registration (`register_into_with_*`, which returns the count).

### Properties
- async: false (TypeScript: returns a promise, because targets are loaded with dynamic `import()`)
- thread_safe: false (mutates the registry)
- pure: false (reads files, imports code, registers modules)
- idempotent: false (a second load re-registers and fails with `DUPLICATE_MODULE_ID`)

## Contract: BindingLoader.load_binding_dir

### Inputs
- `dir_path` (optional) — directory to scan; falls back to `bindings.dir`, then `./bindings`
- `registry` (required)
- `pattern` (optional) — file-name pattern (A25); falls back to `bindings.pattern`, then `*.binding.yaml`
- `config` (optional) — source for `bindings.dir`, `bindings.pattern` and `validation.binding.*`

### Errors
- `BINDING_FILE_INVALID` — the resolved directory does not exist; any error from loading a matched file (fail-fast)

### Returns
- All modules from all matched files (Rust: the count), in file-name order; empty when nothing matches

### Properties
- async: false (TypeScript: returns a promise)
- thread_safe: false
- pure: false
- idempotent: false

## Testing strategy

- **Function wrapping** — schema inference for primitives, defaults, `Optional`, unions, containers, `Literal`, `Annotated` constraints, nested models; Context detection by type; `*args` / `**kwargs`; missing-hint errors; result normalisation; sync vs. async `execute`; the three `module()` forms; auto-ID sanitisation.
- **Binding files** — parsing and structural errors; target resolution for functions and `Class.method`; each schema mode and the mode-conflict error; `schema_ref` relative resolution; `auto_schema: "strict"` rejection; directory loading with `bindings.dir` / `bindings.pattern` precedence; missing-directory error; fail-fast; an end-to-end call through the executor.
