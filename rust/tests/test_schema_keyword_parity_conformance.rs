//! Cross-language conformance test for JSON Schema keyword parity at the
//! validation boundary (PROTOCOL_SPEC §4.15, JSON Schema 2020-12 §6 and §10.2).
//!
//! Consumes the canonical `schema_keyword_parity.json` fixture shipped by the
//! `apcore` spec repo (sibling directory or `CONFORMANCE_SPEC_REPO`).
//!
//! The fixture's `driver_contract` requires driving the code path a module
//! invocation actually takes. In this SDK that is
//! [`apcore::executor::validate_against_schema`] — the function
//! `src/builtin_steps.rs` calls for input and output validation. Driving
//! `SchemaValidator` instead would defeat the fixture's purpose.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use async_trait::async_trait;
use serde_json::{json, Value};

use apcore::executor::validate_against_schema;
use apcore::{Config, Context, ErrorCode, Executor, Module, ModuleError, Registry};

use crate::conformance_env::find_fixtures_root;

fn load_fixture(name: &str) -> Value {
    let path = find_fixtures_root().join(format!("{name}.json"));
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to read fixture: {}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("Invalid JSON in {name}: {e}"))
}

#[test]
fn conformance_schema_keyword_parity() {
    let fixture = load_fixture("schema_keyword_parity");
    let cases = fixture["test_cases"]
        .as_array()
        .expect("fixture must carry a test_cases array");
    assert!(!cases.is_empty(), "fixture must carry at least one case");

    let mut failures: Vec<String> = Vec::new();

    for tc in cases {
        let id = tc["id"].as_str().expect("every case needs an id");
        let description = tc["description"].as_str().unwrap_or("");
        let schema = &tc["schema"];
        let input = &tc["input"];
        let expected_valid = tc["expected_valid"]
            .as_bool()
            .unwrap_or_else(|| panic!("FAIL [{id}]: fixture missing expected_valid"));

        // The executor boundary: exactly what builtin_steps.rs runs for a
        // module's `input_schema` / `output_schema`.
        let outcome = validate_against_schema(input, schema, "Input");
        let actual_valid = outcome.is_ok();

        if actual_valid != expected_valid {
            let actual = match &outcome {
                Ok(()) => "valid".to_string(),
                Err(e) => format!("invalid ({:?}: {})", e.code, e.message),
            };
            failures.push(format!(
                "  [{id}] {description}\n    \
                 schema:   {schema}\n    \
                 input:    {input}\n    \
                 expected: {}\n    \
                 actual:   {actual}",
                if expected_valid { "valid" } else { "invalid" },
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "schema_keyword_parity: {}/{} case(s) diverge from the spec fixture \
         (cross-language divergence in validate_against_schema):\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

struct Echo {
    input: Value,
    output: Value,
    executions: Arc<AtomicUsize>,
}

#[async_trait]
impl Module for Echo {
    fn description(&self) -> &'static str {
        "Return the supplied object"
    }

    fn input_schema(&self) -> Value {
        self.input.clone()
    }

    fn output_schema(&self) -> Value {
        self.output.clone()
    }

    async fn execute(&self, inputs: Value, _: &Context<Value>) -> Result<Value, ModuleError> {
        self.executions.fetch_add(1, Ordering::Relaxed);
        Ok(inputs)
    }
}

#[tokio::test]
async fn conformance_schema_keyword_parity_through_public_module_boundary() {
    let fixture = load_fixture("schema_keyword_parity");
    let cases = fixture["test_cases"].as_array().expect("fixture cases");
    for case in cases
        .iter()
        .filter(|case| case["exercise_module_boundary"] == true)
    {
        let registry = Arc::new(Registry::new());
        let open = json!({"type": "object"});
        let schema = case["schema"].clone();
        let input = &case["input"];
        let executions = Arc::new(AtomicUsize::new(0));
        registry
            .register_module(
                "parity.input",
                Box::new(Echo {
                    input: schema.clone(),
                    output: open.clone(),
                    executions: Arc::clone(&executions),
                }),
            )
            .unwrap();
        registry
            .register_module(
                "parity.output",
                Box::new(Echo {
                    input: open,
                    output: schema,
                    executions: Arc::clone(&executions),
                }),
            )
            .unwrap();
        let executor = Executor::new(registry, Config::default());

        for (id, executions_on_rejection) in [("parity.input", 0), ("parity.output", 1)] {
            let result = executor.call(id, input.clone(), None, None).await;
            if case["expected_valid"] == true {
                assert_eq!(result.unwrap(), *input, "{}", case["id"]);
            } else {
                let error = result.unwrap_err();
                assert_eq!(
                    error.code,
                    ErrorCode::SchemaValidationError,
                    "{}",
                    case["id"]
                );
                if let Some(keyword) = case["expected_keyword"].as_str() {
                    let errors = error.details["errors"]
                        .as_array()
                        .expect("validation errors");
                    assert!(
                        errors.iter().any(|entry| entry["keyword"] == keyword),
                        "{} did not report {keyword}: {errors:?}",
                        case["id"]
                    );
                }
                assert_eq!(executions.load(Ordering::Relaxed), executions_on_rejection);
            }
        }
    }
}
