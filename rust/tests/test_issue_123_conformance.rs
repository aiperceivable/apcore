//! Public-path drivers for apcore#123: §2.5.1, §4.17, §5.12, §9.8.2,
//! §12.7.5 and §12.8. Canonical fixtures remain the source of expectations.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use apcore::bindings::{typed_handler, BindingLoader, TypedBindingHandler};
use apcore::cancel::CancelToken;
use apcore::config::{Config, EnvStyle, NamespaceRegistration};
use apcore::errors::ModuleError;
use apcore::module::{Change, ChunkStream, ModuleAnnotations, PreviewResult, StreamingModule};
use apcore::registry::{DefaultDiscoverer, ModuleDescriptor};
use apcore::schema::exporter::{ExportProfile, SchemaExporter};
use apcore::schema::SchemaDefinition;
use apcore::{ACLRule, Context, Executor, Module, Registry, ACL};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[path = "conformance_env.rs"]
mod conformance_env;

fn fixture(name: &str) -> Value {
    let path = conformance_env::find_fixtures_root().join(format!("{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(path).expect("read canonical fixture"))
        .expect("parse canonical fixture")
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .expect("array")
        .iter()
        .map(|v| v.as_str().expect("string").to_owned())
        .collect()
}

fn keys(expected: &Value, supported: &[&str]) {
    for key in expected.as_object().expect("expected object").keys() {
        assert!(
            supported.contains(&key.as_str()),
            "unhandled expected key: {key}"
        );
    }
}

fn cases(fixture: &Value) -> &[Value] {
    let cases = fixture["test_cases"].as_array().expect("test_cases");
    assert!(!cases.is_empty(), "fixture cases must not be empty");
    cases
}

#[test]
fn test_conformance_declaration_matches_current_version_and_fixture_scope() {
    let declaration: Value =
        serde_yaml_ng::from_str(include_str!("../apcore-conformance.yaml")).unwrap();
    assert_eq!(
        declaration["implementation"]["version"],
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(declaration["implementation"]["spec_version"], "1.65.0");
    let results = &declaration["conformance"]["fixture_results"];
    let names = strings(&results["fixture_names"]);
    assert_eq!(names.len(), 9);
    assert!(names.iter().any(|name| name == "canonicalize_name"));
    assert_eq!(json!(names.len()), results["fixtures"]);
    let count: usize = names.iter().map(|name| cases(&fixture(name)).len()).sum();
    assert_eq!(count, 79);
    assert_eq!(json!(count), results["cases"]);
    assert_eq!(results["passed"], results["cases"]);
    assert_eq!(results["failed"], 0);
    assert_eq!(results["skipped"], 0);
    assert_eq!(
        strings(&results["report"]),
        [
            "tests/test_issue_123_conformance.rs",
            "tests/test_canonicalize_name.rs"
        ]
    );
    assert_eq!(
        results["command"],
        "cargo test --all-features --test it --test test_issue_123_conformance"
    );
    assert_eq!(
        declaration["conformance"]["level"], 0,
        "scoped verification cannot certify a higher level"
    );
}

fn outcome(result: Result<Value, ModuleError>, expected: &Value) {
    if let Some(code) = expected.get("error_code") {
        assert_eq!(json!(result.expect_err("expected error").code), *code);
    } else {
        assert_eq!(result.expect("expected output"), expected["output"]);
    }
}

// Collect each case's assertion failures without losing the rest of the fixture.
macro_rules! run_cases {
    ($fixture:expr, $case:ident, $body:block) => {{
        let mut failures = Vec::new();
        for $case in cases(&$fixture) {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| $body));
            if result.is_err() {
                failures.push($case["id"].as_str().unwrap().to_owned());
            }
        }
        assert!(failures.is_empty(), "failed cases: {failures:?}");
    }};
}

struct FixtureModule {
    contract: Value,
    options: Value,
}

#[async_trait::async_trait]
impl Module for FixtureModule {
    fn description(&self) -> &'static str {
        "Canonical issue conformance module"
    }
    fn input_schema(&self) -> Value {
        self.contract["input_schema"].clone()
    }
    fn output_schema(&self) -> Value {
        self.contract["output_schema"].clone()
    }
    fn as_streaming(&self) -> Option<&dyn StreamingModule> {
        (self.contract["annotations"]["streaming"] == true).then_some(self)
    }
    fn stream(&self, inputs: Value, context: &Context<Value>) -> Option<ChunkStream> {
        self.as_streaming()
            .map(|module| module.stream_typed(inputs, context))
    }
    fn preflight(&self, _: &Value, _: Option<&Context<Value>>) -> Vec<String> {
        if self.options["implements_preflight"] == true {
            strings(&self.contract["preflight_returns"])
        } else {
            Vec::new()
        }
    }
    fn preview(&self, _: &Value, _: Option<&Context<Value>>) -> Option<PreviewResult> {
        if self.options["implements_preview"] != true
            || self.options["preview_returns_null"] == true
        {
            return None;
        }
        let change: Change =
            serde_json::from_value(self.contract["preview_change"].clone()).unwrap();
        Some(PreviewResult::new(vec![change]))
    }
    async fn execute(&self, _: Value, _: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({"ok": true}))
    }
}

impl StreamingModule for FixtureModule {
    fn stream_typed(&self, _: Value, _: &Context<Value>) -> ChunkStream {
        Box::pin(futures_util::stream::iter([Ok(json!({"ok": true}))]))
    }
}

fn registry(contract: &Value, options: &Value) -> Arc<Registry> {
    let registry = Arc::new(Registry::new());
    registry
        .register_module(
            contract["module_id"].as_str().unwrap(),
            Box::new(FixtureModule {
                contract: contract.clone(),
                options: options.clone(),
            }),
        )
        .expect("register fixture module");
    registry
}

#[test]
fn conformance_binding_file_validation() {
    #[derive(Serialize, Deserialize, JsonSchema)]
    struct GreetInput {
        name: String,
    }
    #[derive(Serialize, Deserialize, JsonSchema)]
    struct GreetOutput {
        greeting: String,
    }
    let fx = fixture("binding_file_validation");
    run_cases!(fx, case, {
        keys(&case["expected"], &["error_code", "module_ids"]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.binding.yaml");
        std::fs::write(
            &path,
            serde_yaml_ng::to_string(&case["input"]["file"]).unwrap(),
        )
        .unwrap();
        let mut loader = BindingLoader::new();
        let registry = Registry::new();
        let mut handlers = HashMap::new();
        handlers.insert(
            "fixture_targets:typed_greet".to_owned(),
            typed_handler::<GreetInput, GreetOutput>(|input| {
                Ok(GreetOutput {
                    greeting: format!("Hello {}", input.name),
                })
            }),
        );
        handlers.insert(
            "fixture_targets:untyped_greet".to_owned(),
            TypedBindingHandler {
                handler: Arc::new(|_, _| Box::pin(async { Ok(json!({"greeting":"Hello"})) })),
                input_schema: None,
                output_schema: None,
            },
        );
        let result = loader
            .load_from_yaml(&path)
            .and_then(|()| loader.register_into_with_typed_handlers(&registry, handlers));
        if let Some(code) = case["expected"].get("error_code") {
            assert_eq!(json!(result.expect_err("binding must fail").code), *code);
        } else {
            result.expect("binding must load");
            assert_eq!(json!(registry.module_ids()), case["expected"]["module_ids"]);
        }
    });
}

#[test]
fn conformance_export_profiles() {
    let fx = fixture("export_profiles");
    run_cases!(fx, case, {
        keys(
            &case["expected"],
            &["paths", "absent_paths", "no_x_keywords_under"],
        );
        let mut contract = fx["module_contract"].clone();
        contract["annotations"] = case["input"]["annotations"].clone();
        let descriptor: ModuleDescriptor = serde_json::from_value(contract.clone()).unwrap();
        let registry = Registry::new();
        let id = contract["module_id"].as_str().unwrap();
        registry
            .register(
                id,
                Box::new(FixtureModule {
                    contract: contract.clone(),
                    options: json!({}),
                }),
                descriptor,
            )
            .unwrap();
        let descriptor = registry.get_definition(id).unwrap().unwrap();
        let definition = SchemaDefinition {
            module_id: descriptor.module_id,
            description: descriptor.description,
            input_schema: descriptor.input_schema,
            output_schema: descriptor.output_schema,
            error_schema: None,
            definitions: None,
            version: None,
        };
        let profile: ExportProfile =
            serde_json::from_value(case["input"]["profile"].clone()).unwrap();
        let typed = SchemaExporter::new()
            .export_def(
                &definition,
                profile,
                descriptor.annotations.as_ref(),
                None,
                None,
            )
            .unwrap();
        let mut raw = contract.clone();
        raw["name"] = json!(if profile == ExportProfile::Mcp {
            id.to_string()
        } else {
            id.replace('.', "_")
        });
        let raw = SchemaExporter::new().export(&raw, profile, None).unwrap();
        for exported in [typed, raw] {
            if let Some(paths) = case["expected"].get("paths") {
                for (path, expected) in paths.as_object().unwrap() {
                    assert_eq!(exported.pointer(path), Some(expected), "{path}");
                }
            }
            if let Some(paths) = case["expected"].get("absent_paths") {
                for path in strings(paths) {
                    assert!(exported.pointer(&path).is_none(), "{path}");
                }
            }
            if let Some(path) = case["expected"].get("no_x_keywords_under") {
                fn check(value: &Value, properties: bool) {
                    match value {
                        Value::Object(object) => {
                            for (key, value) in object {
                                assert!(
                                    properties || !key.starts_with("x-"),
                                    "unexpected extension keyword {key}"
                                );
                                check(value, key == "properties");
                            }
                        }
                        Value::Array(array) => {
                            for value in array {
                                check(value, false);
                            }
                        }
                        _ => {}
                    }
                }
                check(
                    exported
                        .pointer(path.as_str().unwrap())
                        .expect("export schema"),
                    false,
                );
            }
        }
    });
}

#[test]
fn export_preserves_property_and_definition_names() {
    let input = json!({
        "type": "object",
        "description": "Ordinary description", "x-llm-description": "LLM description",
        "x-owner": "private",
        "properties": {
            "x-trace": {"type": "string", "x-sensitive": true},
            "default": {"type": "string"},
            "value": {"$ref": "#/$defs/x-item"}
        },
        "required": ["x-trace", "default", "value"],
        "$defs": {"x-item": {"type": "string", "x-owner": "private"}}
    });
    let definition = SchemaDefinition {
        module_id: "demo.send".into(),
        description: "Send a message".into(),
        input_schema: input.clone(),
        output_schema: json!({}),
        error_schema: None,
        definitions: None,
        version: None,
    };
    let raw = json!({"name": "demo_send", "description": "Send a message", "input_schema": input});
    let exporter = SchemaExporter::new();
    for profile in [ExportProfile::Anthropic, ExportProfile::OpenAi] {
        for exported in [
            exporter.export(&raw, profile, None).unwrap(),
            exporter
                .export_def(&definition, profile, None, None, None)
                .unwrap(),
        ] {
            let schema = exported
                .pointer(if profile == ExportProfile::Anthropic {
                    "/input_schema"
                } else {
                    "/function/parameters"
                })
                .unwrap();
            assert_eq!(schema["properties"]["x-trace"]["type"], "string");
            assert_eq!(schema["properties"]["default"]["type"], "string");
            assert_eq!(schema["$defs"]["x-item"]["type"], "string");
            assert_eq!(schema["description"], "LLM description");
            assert!(schema.get("x-owner").is_none());
            assert!(schema["properties"]["x-trace"].get("x-sensitive").is_none());
            assert!(schema["$defs"]["x-item"].get("x-owner").is_none());
        }
    }
}

#[tokio::test]
async fn conformance_error_details_shape() {
    let fx = fixture("error_details_shape");
    let executor = Executor::new(
        registry(&fx["module_contract"], &json!({})),
        Config::default(),
    );
    let mut failures = Vec::new();
    for case in cases(&fx) {
        keys(
            &case["expected"],
            &[
                "error_code",
                "errors",
                "error_count",
                "detail_keys_present",
                "detail_keys_absent",
            ],
        );
        let module_id = case["input"]["call_module_id"]
            .as_str()
            .unwrap_or(fx["module_contract"]["module_id"].as_str().unwrap());
        let result = executor
            .call(module_id, case["input"]["inputs"].clone(), None, None)
            .await;
        let assertion = std::panic::catch_unwind(|| {
            let serialized = serde_json::to_value(result.expect_err("call must fail")).unwrap();
            let expected = &case["expected"];
            assert_eq!(serialized["code"], expected["error_code"]);
            let details = &serialized["details"];
            if let Some(errors) = expected.get("errors") {
                let actual = details["errors"].as_array().expect("errors array");
                for item in actual {
                    let names: BTreeSet<_> = item
                        .as_object()
                        .unwrap()
                        .keys()
                        .map(String::as_str)
                        .collect();
                    assert_eq!(names, BTreeSet::from(["path", "keyword", "message"]));
                    assert_ne!(item["message"].as_str().unwrap(), "");
                }
                for error in errors.as_array().unwrap() {
                    assert!(actual
                        .iter()
                        .any(|item| item["path"] == error["path"]
                            && item["keyword"] == error["keyword"]));
                }
            }
            if let Some(count) = expected.get("error_count") {
                assert_eq!(json!(details["errors"].as_array().unwrap().len()), *count);
            }
            for (key, present) in [("detail_keys_present", true), ("detail_keys_absent", false)] {
                if let Some(names) = expected.get(key) {
                    for name in strings(names) {
                        assert_eq!(details.get(&name).is_some(), present);
                    }
                }
            }
        });
        if assertion.is_err() {
            failures.push(case["id"].clone());
        }
    }
    assert!(failures.is_empty(), "failed cases: {failures:?}");
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One fixture driver keeps each check assertion beside its execution result.
async fn conformance_preflight_check_reporting() {
    let fx = fixture("preflight_check_reporting");
    let mut failures = Vec::new();
    for case in cases(&fx) {
        let input = &case["input"];
        let expected = &case["expected"];
        keys(
            expected,
            &[
                "valid",
                "failed_checks",
                "passed_checks",
                "checks_absent",
                "checks_present",
                "optional_passed_checks",
                "predicted_changes_count",
                "predicted_changes_present",
            ],
        );
        let registry = if input["register"] == false {
            Arc::new(Registry::new())
        } else {
            registry(&fx["module_contract"], input)
        };
        let rules = input["acl_rules"]
            .as_array()
            .unwrap()
            .iter()
            .map(|rule| {
                ACLRule::new(
                    strings(&rule["callers"]),
                    strings(&rule["targets"]),
                    rule["effect"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        let mut executor = Executor::new(registry, Config::default());
        executor.set_acl(ACL::new(
            rules,
            input["default_effect"].as_str().unwrap(),
            None,
        ));
        let context = Context::<Value>::builder()
            .caller_id(Some(input["caller_id"].as_str().unwrap().to_owned()))
            .build();
        let actual = serde_json::to_value(
            executor
                .validate(
                    input["module_id"].as_str().unwrap(),
                    &input["inputs"],
                    Some(&context),
                )
                .await
                .unwrap(),
        )
        .unwrap();
        let assertion = std::panic::catch_unwind(|| {
            assert_eq!(actual["valid"], expected["valid"]);
            let checks = actual["checks"].as_array().unwrap();
            let find = |name: &str| checks.iter().find(|check| check["check"] == name);
            let failed: BTreeSet<_> = checks
                .iter()
                .filter(|check| check["passed"] == false)
                .map(|check| check["check"].as_str().unwrap().to_owned())
                .collect();
            assert_eq!(
                failed,
                strings(&expected["failed_checks"]).into_iter().collect()
            );
            for key in [
                "passed_checks",
                "checks_absent",
                "checks_present",
                "optional_passed_checks",
            ] {
                if let Some(names) = expected.get(key) {
                    for name in strings(names) {
                        match key {
                            "checks_absent" => assert!(find(&name).is_none(), "{name}"),
                            "checks_present" => assert!(find(&name).is_some(), "{name}"),
                            "passed_checks" => {
                                assert_eq!(find(&name).expect(&name)["passed"], true);
                            }
                            "optional_passed_checks" => {
                                if let Some(check) = find(&name) {
                                    assert_eq!(check["passed"], true);
                                    assert_eq!(
                                        check["warnings"].as_array().unwrap(),
                                        &Vec::<Value>::new()
                                    );
                                }
                            }
                            _ => unreachable!(),
                        }
                    }
                }
            }
            if let Some(present) = expected.get("predicted_changes_present") {
                assert_eq!(
                    json!(actual.get("predicted_changes").is_some_and(Value::is_array)),
                    *present
                );
            }
            if let Some(count) = expected.get("predicted_changes_count") {
                assert_eq!(
                    json!(actual["predicted_changes"].as_array().unwrap().len()),
                    *count
                );
            }
        });
        if assertion.is_err() {
            failures.push(case["id"].clone());
        }
    }
    assert!(failures.is_empty(), "failed cases: {failures:?}");
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum NativeLevel {
    Low,
    High,
}
#[derive(Serialize, Deserialize, JsonSchema)]
struct NativeInput {
    #[schemars(with = "String")]
    when: chrono::DateTime<chrono::Utc>,
    #[schemars(with = "String")]
    request_id: uuid::Uuid,
    level: NativeLevel,
}
#[derive(Serialize, Deserialize, JsonSchema)]
struct NativeOutput {
    accepted: bool,
}

#[tokio::test]
async fn conformance_json_input_native_types() {
    let fx = fixture("json_input_native_types");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("native.binding.yaml");
    let module_id = fx["module_contract"]["module_id"].as_str().unwrap();
    std::fs::write(
        &path,
        serde_yaml_ng::to_string(&json!({"spec_version":"1.0", "bindings":[{
        "module_id":module_id, "target":"native:schedule"}]}))
        .unwrap(),
    )
    .unwrap();
    let mut loader = BindingLoader::new();
    loader.load_from_yaml(&path).unwrap();
    let registry = Registry::new();
    let accepted = fx["module_contract"]["returns"]["accepted"]
        .as_bool()
        .unwrap();
    loader
        .register_into_with_typed_handlers(
            &registry,
            HashMap::from([(
                "native:schedule".to_owned(),
                typed_handler::<NativeInput, NativeOutput>(move |_| Ok(NativeOutput { accepted })),
            )]),
        )
        .unwrap();
    let executor = Executor::new(registry, Config::default());
    for case in cases(&fx) {
        keys(&case["expected"], &["output", "error_code"]);
        outcome(
            executor
                .call(module_id, case["input"]["inputs"].clone(), None, None)
                .await,
            &case["expected"],
        );
    }
}

type CapturedTokens = Arc<Mutex<HashMap<String, Option<CancelToken>>>>;
struct TimedModule {
    id: String,
    specification: Value,
    captures: CapturedTokens,
}

#[async_trait::async_trait]
impl Module for TimedModule {
    fn description(&self) -> &'static str {
        "Cooperative timeout conformance module"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type":"object"})
    }
    fn annotations(&self) -> ModuleAnnotations {
        let mut annotations = ModuleAnnotations::default();
        annotations.extra.insert(
            "resources".to_owned(),
            json!({"timeout":self.specification["module_timeout_ms"]}),
        );
        annotations
    }
    async fn execute(&self, _: Value, context: &Context<Value>) -> Result<Value, ModuleError> {
        self.captures
            .lock()
            .unwrap()
            .insert(self.id.clone(), context.cancel_token.clone());
        match self.specification["kind"].as_str().unwrap() {
            "sleeper" => {
                let mut remaining = self.specification["sleep_ms"].as_u64().unwrap();
                while remaining > 0 {
                    if self.specification["checks_token"] == true {
                        if let Some(token) = &context.cancel_token {
                            token.check().map_err(ModuleError::from)?;
                        }
                    }
                    let step = remaining.min(10);
                    tokio::time::sleep(Duration::from_millis(step)).await;
                    remaining -= step;
                }
                Ok(json!({"slept":true}))
            }
            "caller" => {
                let result = context
                    .executor()
                    .expect("bound executor")
                    .call(
                        self.specification["calls"].as_str().unwrap(),
                        json!({}),
                        Some(context),
                        None,
                    )
                    .await;
                match result {
                    Err(error) if self.specification["catches"] == json!(error.code) => {
                        Ok(json!({"caught":error.code}))
                    }
                    other => other,
                }
            }
            kind => panic!("unhandled module kind {kind}"),
        }
    }
}

#[tokio::test]
async fn conformance_timeout_cancellation() {
    let fx = fixture("timeout_cancellation");
    let mut failures = Vec::new();
    for case in cases(&fx) {
        let input = &case["input"];
        keys(
            &case["expected"],
            &[
                "error_code",
                "output",
                "token_cancelled",
                "returns_within_ms",
            ],
        );
        let captures: CapturedTokens = Arc::new(Mutex::new(HashMap::new()));
        let registry = Registry::new();
        for (id, specification) in input["modules"].as_object().unwrap() {
            registry
                .register_module(
                    id,
                    Box::new(TimedModule {
                        id: id.clone(),
                        specification: specification.clone(),
                        captures: captures.clone(),
                    }),
                )
                .unwrap();
        }
        let mut config = Config::default();
        if let Some(config_values) = input.get("config") {
            for (key, value) in config_values["executor"].as_object().unwrap() {
                config.set(&format!("executor.{key}"), value.clone());
            }
        }
        let mut executor = Executor::new(registry, config);
        executor.set_acl(ACL::new(
            vec![ACLRule::new(vec!["*".into()], vec!["*".into()], "allow")],
            "deny",
            None,
        ));
        let executor = executor.into_shared();
        let application = (input["application_token"] == true).then(CancelToken::new);
        let context =
            Context::<Value>::create(None, None, application.clone(), None, json!({}), None);
        let cancellation = input["cancel_application_token_after_ms"]
            .as_u64()
            .map(|ms| {
                let token = application.clone().expect("application token");
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                    token.cancel();
                })
            });
        let start = Instant::now();
        let result = executor
            .call(
                input["call"].as_str().unwrap(),
                json!({}),
                Some(&context),
                None,
            )
            .await;
        let elapsed = start.elapsed().as_millis();
        if let Some(task) = cancellation {
            task.abort();
        }
        let assertion = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            outcome(result, &case["expected"]);
            for (id, expected) in case["expected"]["token_cancelled"].as_object().unwrap() {
                let cancelled = if id == "application" {
                    application.as_ref().is_some_and(CancelToken::is_cancelled)
                } else {
                    captures
                        .lock()
                        .unwrap()
                        .get(id)
                        .and_then(Option::as_ref)
                        .is_some_and(CancelToken::is_cancelled)
                };
                assert_eq!(json!(cancelled), *expected, "{id} token state");
            }
            assert!(
                elapsed <= u128::from(case["expected"]["returns_within_ms"].as_u64().unwrap()),
                "elapsed {elapsed}"
            );
        }));
        if assertion.is_err() {
            failures.push(case["id"].clone());
        }
    }
    assert!(failures.is_empty(), "failed cases: {failures:?}");
}

#[test]
fn conformance_env_prefix_dispatch() {
    const CHILD_CASE: &str = "CONFORMANCE_ENV_DISPATCH_CASE";
    let fx = fixture("env_prefix_dispatch");
    if let Ok(id) = std::env::var(CHILD_CASE) {
        let case = cases(&fx)
            .iter()
            .find(|case| case["id"] == id)
            .expect("known case");
        let input = &case["input"];
        let expected = &case["expected"];
        keys(expected, &["register_error_code", "values", "absent"]);
        for (name, _) in std::env::vars().filter(|(name, _)| name.starts_with("APCORE")) {
            std::env::remove_var(name);
        }
        let registrations = input["namespaces"].as_array().unwrap();
        for (index, namespace) in registrations.iter().enumerate() {
            let result = Config::register_namespace(NamespaceRegistration {
                name: namespace["name"].as_str().unwrap().into(),
                env_prefix: Some(namespace["env_prefix"].as_str().unwrap().into()),
                defaults: None,
                schema: None,
                env_style: EnvStyle::Auto,
                max_depth: 32,
                env_map: None,
            });
            if let Some(code) = expected.get("register_error_code") {
                assert_eq!(
                    index,
                    registrations.len() - 1,
                    "error fixture must finish on final registration"
                );
                assert_eq!(json!(result.expect_err("reserved prefix").code), *code);
                return;
            }
            result.unwrap();
        }
        for (name, value) in input["env"].as_object().unwrap() {
            std::env::set_var(name, value.as_str().unwrap());
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("apcore.yaml");
        std::fs::write(
            &path,
            serde_yaml_ng::to_string(&input["config_file"]).unwrap(),
        )
        .unwrap();
        let config = Config::load(&path).unwrap();
        if let Some(values) = expected.get("values") {
            for (path, value) in values.as_object().unwrap() {
                assert_eq!(config.get(path), Some(value.clone()), "{path}");
                if path == "apcore.executor.default_timeout" {
                    assert_eq!(
                        json!(config.executor.default_timeout),
                        *value,
                        "runtime timeout consumer"
                    );
                }
            }
        }
        if let Some(paths) = expected.get("absent") {
            for path in strings(paths) {
                assert!(config.get(&path).is_none(), "{path}");
            }
        }
    } else {
        let mut failures = Vec::new();
        for case in cases(&fx) {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "conformance_env_prefix_dispatch", "--nocapture"])
                .env(CHILD_CASE, case["id"].as_str().unwrap())
                .status()
                .unwrap();
            if !status.success() {
                failures.push(case["id"].clone());
            }
        }
        assert!(failures.is_empty(), "failed isolated cases: {failures:?}");
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Ordered operations and every audit assertion share one per-case driver.
async fn conformance_ephemeral_modules() {
    let fx = fixture("ephemeral_modules");
    let mut failures = Vec::new();
    for case in cases(&fx) {
        keys(
            &case["expected"],
            &["events", "secret_absent", "error_code"],
        );
        let mut config = Config::default();
        config.set("sys_modules.enabled", json!(true));
        config.set("sys_modules.events.enabled", json!(true));
        let mut client = apcore::APCore::with_config(config);
        let events = Arc::new(Mutex::new(Vec::new()));
        for kind in [
            "apcore.registry.module_registered",
            "apcore.registry.module_unregistered",
        ] {
            let records = events.clone();
            client
                .on(kind, move |event| {
                    records.lock().unwrap().push(json!({
                "event_type":event.event_type, "module_id":event.module_id, "payload":event.data,
            }));
                })
                .unwrap();
        }
        let mut error = None;
        let operations = case["input"]["operations"].as_array().unwrap();
        for operation in operations {
            let context = operation.get("context").map(|specification| {
                let identity = specification.get("identity").map(|identity| {
                    apcore::Identity::new(
                        identity["id"].as_str().unwrap().to_owned(),
                        identity["type"].as_str().unwrap().to_owned(),
                        strings(&identity["roles"]),
                        serde_json::from_value(identity["attrs"].clone()).unwrap(),
                    )
                });
                let mut context =
                    Context::<Value>::create(identity, None, None, None, json!({}), None);
                context.caller_id = specification["caller_id"].as_str().map(str::to_owned);
                context
            });
            let module_id = operation["module_id"].as_str().unwrap_or("ephemeral.tool");
            let contract = json!({"module_id":module_id,"description":"Ephemeral fixture module",
                "input_schema":{"type":"object"},"output_schema":{"type":"object"}});
            let module = || {
                Box::new(FixtureModule {
                    contract: contract.clone(),
                    options: json!({}),
                }) as Box<dyn Module>
            };
            let result = match operation["op"].as_str().unwrap() {
                "register" => client.registry().register_module_with_context(
                    module_id,
                    module(),
                    context.as_ref(),
                ),
                "unregister" => client
                    .registry()
                    .unregister_with_context(module_id, context.as_ref())
                    .map(|_| ()),
                "register_internal" => client.registry().register_internal(
                    module_id,
                    module(),
                    serde_json::from_value(contract).unwrap(),
                ),
                "discover" => {
                    let directory = tempfile::tempdir().unwrap();
                    let relative = format!("{}.rs", operation["file"].as_str().unwrap());
                    let path = directory.path().join(relative);
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(&path, "pub struct Tool;\n").unwrap();
                    client.registry().set_extension_roots(vec![directory
                        .path()
                        .to_str()
                        .unwrap()
                        .to_owned()]);
                    client
                        .registry()
                        .set_discoverer(Box::new(DefaultDiscoverer::new()));
                    client.registry().discover_internal().await.map(|_| ())
                }
                name => panic!("unhandled operation {name}"),
            };
            if let Err(failure) = result {
                error = Some(failure);
                break;
            }
        }
        tokio::task::yield_now().await;
        client.events().unwrap().flush_default().await.unwrap();
        let records = events.lock().unwrap().clone();
        let assertion = std::panic::catch_unwind(|| {
            let expected = &case["expected"];
            if let Some(code) = expected.get("error_code") {
                assert_eq!(json!(error.expect("operation error").code), *code);
            } else {
                assert!(error.is_none(), "unexpected operation error: {error:?}");
            }
            let wanted = expected["events"].as_array().unwrap();
            let ids: BTreeSet<_> = operations
                .iter()
                .filter_map(|op| op["module_id"].as_str())
                .collect();
            let records: Vec<_> = records
                .iter()
                .filter(|event| ids.contains(event["module_id"].as_str().unwrap_or("")))
                .collect();
            assert_eq!(records.len(), wanted.len(), "exact event count");
            for (record, event) in records.iter().zip(wanted) {
                assert_eq!(record["event_type"], event["event_type"]);
                assert_eq!(record["module_id"], event["module_id"]);
                if let Some(payload) = event.get("payload") {
                    for (key, value) in payload.as_object().unwrap() {
                        assert_eq!(record["payload"].get(key), Some(value), "{key}");
                    }
                }
                if let Some(names) = event.get("payload_absent_keys") {
                    for name in strings(names) {
                        assert!(record["payload"].get(&name).is_none());
                    }
                }
                if let Some(id) = event.get("identity_id") {
                    assert_eq!(record["payload"]["identity"]["id"], *id);
                }
            }
            if let Some(secret) = expected.get("secret_absent") {
                assert!(!serde_json::to_string(&records)
                    .unwrap()
                    .contains(secret.as_str().unwrap()));
            }
        });
        if assertion.is_err() {
            failures.push(case["id"].clone());
        }
    }
    assert!(failures.is_empty(), "failed cases: {failures:?}");
}

#[derive(Debug)]
struct FailingSubscriber(Arc<AtomicUsize>);

struct CleanupModule(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl Module for CleanupModule {
    fn description(&self) -> &'static str {
        "Cooperative timeout cleanup regression"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type":"object"})
    }
    fn annotations(&self) -> ModuleAnnotations {
        let mut annotations = ModuleAnnotations::default();
        annotations
            .extra
            .insert("resources".into(), json!({"timeout":20}));
        annotations
    }
    async fn execute(&self, _: Value, context: &Context<Value>) -> Result<Value, ModuleError> {
        let token = context.cancel_token.as_ref().expect("executor call token");
        while !token.is_cancelled() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        self.0.fetch_add(1, Ordering::SeqCst);
        token.check().map_err(ModuleError::from)?;
        Ok(json!({}))
    }
}

#[tokio::test]
async fn test_timeout_signals_without_dropping_cooperative_execution() {
    let cleanups = Arc::new(AtomicUsize::new(0));
    let registry = Registry::new();
    registry
        .register_module("cleanup.worker", Box::new(CleanupModule(cleanups.clone())))
        .unwrap();
    let executor = Executor::new(registry, Config::default());
    let start = Instant::now();
    let error = executor
        .call("cleanup.worker", json!({}), None, None)
        .await
        .expect_err("timeout");
    assert_eq!(error.code, apcore::ErrorCode::ModuleTimeout);
    assert!(
        start.elapsed() < Duration::from_millis(100),
        "timeout must not wait for cleanup"
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        cleanups.load(Ordering::SeqCst),
        1,
        "module must remain alive to observe cancellation and clean up"
    );
}

#[async_trait::async_trait]
impl apcore::events::EventSubscriber for FailingSubscriber {
    fn subscriber_id(&self) -> &'static str {
        "issue123-failing"
    }
    fn event_pattern(&self) -> &'static str {
        "issue123.probe"
    }
    async fn on_event(&self, _: &apcore::events::ApCoreEvent) -> Result<(), ModuleError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(ModuleError::new(
            apcore::ErrorCode::GeneralInternalError,
            "Subscriber deliberately fails",
        ))
    }
}

#[tokio::test]
async fn test_configured_subscriber_circuit_breaker_uses_standard_bootstrap() {
    let calls = Arc::new(AtomicUsize::new(0));
    let factory_calls = calls.clone();
    apcore::register_subscriber_type(
        "issue123-failing",
        Box::new(move |_| Ok(Box::new(FailingSubscriber(factory_calls.clone())))),
    );
    let mut config = Config::default();
    config.set("sys_modules.enabled", json!(true));
    config.set("sys_modules.events.enabled", json!(true));
    config.set(
        "sys_modules.events.subscribers",
        json!([{
            "type":"issue123-failing", "circuit_breaker":{
                "timeout_ms":1000, "open_threshold":2, "recovery_window_ms":60000,
            },
        }]),
    );
    let mut client = apcore::APCore::with_config(config);
    let transitions = Arc::new(Mutex::new(Vec::new()));
    let recording = transitions.clone();
    client
        .on("apcore.subscriber.circuit_opened", move |event| {
            recording.lock().unwrap().push(event.data.clone());
        })
        .unwrap();
    let emitter = client.events().unwrap();
    for _ in 0..3 {
        emitter
            .emit_sequential(&apcore::events::ApCoreEvent::new(
                "issue123.probe",
                json!({}),
            ))
            .await;
        emitter.flush_default().await.unwrap();
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "OPEN circuit must suppress the third delivery"
    );
    let transitions = transitions.lock().unwrap();
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0]["subscriber_id"], "issue123-failing");
}

struct CapabilityStep {
    name: &'static str,
    failed: bool,
    requires: &'static [&'static str],
    provides: &'static [&'static str],
    calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl apcore::pipeline::Step for CapabilityStep {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &'static str {
        "Preflight capability dependency regression"
    }
    fn removable(&self) -> bool {
        true
    }
    fn replaceable(&self) -> bool {
        true
    }
    fn pure(&self) -> bool {
        true
    }
    fn requires(&self) -> &[&str] {
        self.requires
    }
    fn provides(&self) -> &[&str] {
        self.provides
    }
    async fn execute(
        &self,
        _: &mut apcore::pipeline::PipelineContext,
    ) -> Result<apcore::pipeline::StepResult, ModuleError> {
        self.calls.lock().unwrap().push(self.name.to_owned());
        if self.failed {
            Err(ModuleError::new(
                apcore::ErrorCode::GeneralInvalidInput,
                "Producer failed",
            ))
        } else {
            Ok(apcore::pipeline::StepResult::continue_step())
        }
    }
}

#[tokio::test]
async fn test_preflight_skips_failed_capability_dependents_but_runs_independent_steps() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let step = |name, failed, requires, provides| {
        Box::new(CapabilityStep {
            name,
            failed,
            requires,
            provides,
            calls: calls.clone(),
        }) as Box<dyn apcore::pipeline::Step>
    };
    let strategy = apcore::pipeline::ExecutionStrategy::new(
        "capabilities",
        vec![
            step("producer", true, &[], &["probe_data"]),
            step("consumer", false, &["probe_data"], &["derived_data"]),
            step("transitive", false, &["derived_data"], &[]),
            step("independent", false, &[], &[]),
        ],
    )
    .unwrap();
    let mut context = apcore::pipeline::PipelineContext::new(
        "demo.test",
        json!({}),
        Context::<Value>::anonymous(),
        "capabilities",
    );
    context.dry_run = true;
    apcore::pipeline::PipelineEngine::run(&strategy, &mut context)
        .await
        .unwrap();
    assert_eq!(*calls.lock().unwrap(), ["producer", "independent"]);
    assert_eq!(context.preflight_errors.len(), 1);
    assert_eq!(
        context
            .trace
            .steps
            .iter()
            .filter(|step| step.skip_reason.as_deref() == Some("missing_dependency"))
            .count(),
        2
    );
    calls.lock().unwrap().clear();
    context.dry_run = false;
    assert!(
        apcore::pipeline::PipelineEngine::run(&strategy, &mut context)
            .await
            .is_err()
    );
    assert_eq!(
        *calls.lock().unwrap(),
        ["producer"],
        "normal execution remains fail-fast"
    );
}
