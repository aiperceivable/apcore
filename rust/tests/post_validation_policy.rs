use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use apcore::approval::{ApprovalHandler, ApprovalRequest, ApprovalResult};
use apcore::errors::ErrorCode;
use apcore::middleware::Middleware;
use apcore::pipeline::{PipelineContext, PipelineState, Step, StepMiddleware, StepResult};
use apcore::registry::ModuleDescriptor;
use apcore::{
    ChunkStream, Config, Context, Executor, Identity, Module, ModuleAnnotations, ModuleError,
    PolicyEffect, PolicyEvaluationRequest, PolicyEvaluator, PolicyVerdict, Registry, ACL,
};
use async_trait::async_trait;
use futures_util::{stream, StreamExt};
use serde_json::{json, Value};

use super::conformance_env;

struct Probe {
    schema: Value,
    requirements: bool,
    starts: Arc<AtomicUsize>,
}

#[async_trait]
impl Module for Probe {
    fn description(&self) -> &'static str {
        "Return validated policy inputs"
    }

    fn input_schema(&self) -> Value {
        self.schema.clone()
    }

    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }

    fn annotations(&self) -> ModuleAnnotations {
        ModuleAnnotations {
            requires_approval: self.requirements,
            ..ModuleAnnotations::default()
        }
    }

    async fn execute(
        &self,
        inputs: Value,
        _context: &Context<Value>,
    ) -> Result<Value, ModuleError> {
        self.starts.fetch_add(1, Ordering::Relaxed);
        Self::result(inputs)
    }

    fn stream(&self, inputs: Value, _context: &Context<Value>) -> Option<ChunkStream> {
        self.starts.fetch_add(1, Ordering::Relaxed);
        Some(Box::pin(stream::iter([Self::result(inputs)])))
    }
}

impl Probe {
    fn result(inputs: Value) -> Result<Value, ModuleError> {
        if inputs.get("fail_execution") == Some(&Value::Bool(true)) {
            return Err(ModuleError::new(
                ErrorCode::ModuleExecuteError,
                "probe failed",
            ));
        }
        Ok(inputs)
    }
}

struct Evaluator {
    decision: String,
    observed: Mutex<Vec<PolicyEvaluationRequest>>,
}

#[async_trait]
impl PolicyEvaluator for Evaluator {
    async fn evaluate(
        &self,
        request: &PolicyEvaluationRequest,
    ) -> Result<PolicyVerdict, ModuleError> {
        self.observed.lock().unwrap().push(request.clone());
        let decision = match self.decision.as_str() {
            "allow" => PolicyEffect::Allow,
            "deny" => PolicyEffect::Deny,
            "require_approval" => PolicyEffect::RequireApproval,
            "throw" => {
                return Err(ModuleError::new(
                    ErrorCode::GeneralInternalError,
                    "backend leaked private-fixture-value",
                ));
            }
            unexpected => panic!("unsupported fixture verdict: {unexpected}"),
        };
        Ok(PolicyVerdict {
            decision,
            rule_id: Some("fixture-rule".to_string()),
        })
    }
}

#[derive(Debug)]
enum Rewrite {
    Amount(Value),
    Reject,
}

#[async_trait]
impl Middleware for Rewrite {
    fn name(&self) -> &'static str {
        "fixture-rewrite"
    }

    async fn before(
        &self,
        _module_id: &str,
        mut inputs: Value,
        _context: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        match self {
            Self::Amount(amount) => {
                inputs["amount"] = amount.clone();
                Ok(Some(inputs))
            }
            Self::Reject => Err(ModuleError::new(
                ErrorCode::GeneralInvalidInput,
                "normalization failed",
            )),
        }
    }

    async fn after(
        &self,
        _module_id: &str,
        _inputs: Value,
        _output: Value,
        _context: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        Ok(None)
    }

    async fn on_error(
        &self,
        _module_id: &str,
        _inputs: Value,
        _error: &ModuleError,
        _context: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        // Policy denials and faults must remain terminal even with recovery.
        Ok(Some(json!({"recovered": true})))
    }
}

fn approval_scope(request: &ApprovalRequest) -> Value {
    json!({
        "authority": request.authority_key(),
        "principal": request.context.as_ref().and_then(|ctx| ctx.identity.as_ref()),
        "caller_id": request.caller_id,
        "module_id": request.module_id,
        "module_version": request.module_version,
        "canonical_args_digest": request.canonical_args_digest,
    })
}

const TEST_CREDENTIAL: &str = "fixture-resume-credential";

#[derive(Debug)]
struct Approver {
    pending: bool,
    supports_final_scope: bool,
    supports_authority_scope: bool,
    requested: Mutex<Vec<ApprovalRequest>>,
    scope: Mutex<Option<Value>>,
    checks: AtomicUsize,
}

#[async_trait]
impl ApprovalHandler for Approver {
    fn supports_final_scope(&self) -> bool {
        self.supports_final_scope
    }

    fn supports_authority_scope(&self) -> bool {
        self.supports_authority_scope
    }

    async fn request_approval(
        &self,
        request: &ApprovalRequest,
    ) -> Result<ApprovalResult, ModuleError> {
        self.requested.lock().unwrap().push(request.clone());
        if self.pending {
            *self.scope.lock().unwrap() = Some(approval_scope(request));
            let mut result = ApprovalResult::default();
            result.status = "pending".to_string();
            result.approval_id = Some(TEST_CREDENTIAL.to_string());
            return Ok(result);
        }
        Ok(ApprovalResult::approved("fixture-reviewer"))
    }

    async fn check_approval(&self, _approval_id: &str) -> Result<ApprovalResult, ModuleError> {
        panic!("status query cannot authorize execution");
    }

    async fn check_approval_for_call(
        &self,
        approval_id: &str,
        request: &ApprovalRequest,
    ) -> Result<ApprovalResult, ModuleError> {
        self.checks.fetch_add(1, Ordering::Relaxed);
        let mut stored = self.scope.lock().unwrap();
        if approval_id != TEST_CREDENTIAL || stored.as_ref() != Some(&approval_scope(request)) {
            return Ok(ApprovalResult::rejected("scope mismatch"));
        }
        stored.take();
        Ok(ApprovalResult::approved("fixture-reviewer"))
    }
}

fn override_inputs(inputs: &mut Value, overrides: &Value) {
    if let Some(overrides) = overrides.as_object() {
        inputs.as_object_mut().unwrap().extend(overrides.clone());
    }
}

async fn invoke_fixture(
    executor: &Executor,
    module_id: &str,
    case: &Value,
    mut inputs: Value,
) -> (Result<Value, ModuleError>, Option<bool>) {
    let id = case["id"].as_str().unwrap();
    let entry = case["entry"].as_str().unwrap();
    let mut requires_approval = None;
    let result = match entry {
        "call" => executor.call(module_id, inputs, None, None).await,
        "preflight" => {
            let preflight = executor.validate(module_id, &inputs, None).await.unwrap();
            assert!(preflight
                .checks
                .iter()
                .any(|check| check.check == "post_validation_policy" && check.passed));
            requires_approval = Some(preflight.requires_approval);
            Ok(Value::Null)
        }
        "stream" => executor
            .stream(module_id, inputs, None, None)
            .next()
            .await
            .unwrap(),
        "resume" => {
            let pending = executor
                .call(module_id, inputs.clone(), None, None)
                .await
                .unwrap_err();
            assert_eq!(pending.code, ErrorCode::ApprovalPending, "{id}: {pending}");
            override_inputs(&mut inputs, &case["retry_inputs"]);
            assert_eq!(pending.details["approval_id"], TEST_CREDENTIAL, "{id}");
            inputs["_approval_token"] = pending.details["approval_id"].clone();
            executor.call(module_id, inputs, None, None).await
        }
        _ => panic!("unsupported fixture entry: {entry}"),
    };
    (result, requires_approval)
}

fn assert_evaluations(observed: &[PolicyEvaluationRequest], contract: &Value, id: &str) {
    let module_id = contract["module_id"].as_str().unwrap();
    for request in observed {
        assert_eq!(request.module_id, module_id, "{id}");
        assert_eq!(
            json!(request.module_version),
            contract["module_version"],
            "{id}"
        );
        assert_eq!(request.sensitive_fields, ["/secret"], "{id}");
        assert!(request.arguments.get("_approval_token").is_none(), "{id}");
    }
}

#[tokio::test]
async fn post_validation_policy_matches_shared_fixture() {
    let fixture: Value = serde_json::from_str(
        &std::fs::read_to_string(
            conformance_env::find_fixtures_root().join("post_validation_policy.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let contract = &fixture["driver_contract"];
    let module_id = contract["module_id"].as_str().unwrap();
    for case in fixture["test_cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let entry = case["entry"].as_str().unwrap();
        let registry = Arc::new(Registry::new());
        let starts = Arc::new(AtomicUsize::new(0));
        let probe = Probe {
            schema: contract["input_schema"].clone(),
            requirements: case["requires_approval"].as_bool().unwrap_or(false),
            starts: Arc::clone(&starts),
        };
        let descriptor: ModuleDescriptor = serde_json::from_value(json!({
            "module_id": module_id,
            "version": contract["module_version"],
            "input_schema": probe.input_schema(),
            "output_schema": probe.output_schema(),
        }))
        .unwrap();
        registry
            .register(module_id, Box::new(probe), descriptor)
            .unwrap();

        let evaluator = spy_evaluator(case["verdict"].as_str().unwrap());
        let approver = Arc::new(Approver {
            supports_authority_scope: false,
            pending: entry == "resume",
            supports_final_scope: true,
            requested: Mutex::new(Vec::new()),
            scope: Mutex::new(None),
            checks: AtomicUsize::new(0),
        });
        let mut executor = Executor::new(registry, Config::in_memory());
        executor
            .set_policy_evaluator(Some(evaluator.clone()))
            .unwrap();
        executor.set_approval_handler_shared(approver.clone());
        if let Some(amount) = case.get("rewrite_amount") {
            executor
                .use_middleware(Box::new(Rewrite::Amount(amount.clone())))
                .unwrap();
        } else if matches!(case["verdict"].as_str(), Some("deny" | "throw")) {
            executor
                .use_middleware(Box::new(Rewrite::Amount(
                    contract["business_input"]["amount"].clone(),
                )))
                .unwrap();
        }
        let mut inputs = contract["business_input"].clone();
        override_inputs(&mut inputs, &case["inputs"]);
        let (result, requires_approval) = invoke_fixture(&executor, module_id, case, inputs).await;
        let observed = evaluator.observed.lock().unwrap();
        assert_evaluations(&observed, contract, id);
        let approval_requests = approver.requested.lock().unwrap();
        for request in approval_requests.iter() {
            assert_eq!(
                json!(request.module_version),
                contract["module_version"],
                "{id}"
            );
            assert_eq!(
                request.canonical_args_digest.as_ref().unwrap().len(),
                64,
                "{id}"
            );
            assert!(request.arguments.get("_approval_token").is_none(), "{id}");
        }
        let error = result.as_ref().err();
        if let (Some(request), Ok(output)) = (observed.last(), &result) {
            if entry != "preflight" {
                assert_eq!(&request.arguments, output, "{id}");
            }
        }
        assert!(
            !error.is_some_and(|error| error.to_string().contains("private-fixture-value")),
            "{id}"
        );
        let observed_results = json!({
            "module_starts": starts.load(Ordering::Relaxed),
            "evaluated_amount": observed.last().map(|r| &r.arguments["amount"]),
            "approval_requests": approval_requests.len(),
            "approved_amount": approval_requests.last().map(|r| &r.arguments["amount"]),
            "requires_approval": requires_approval,
            "resume_checks": approver.checks.load(Ordering::Relaxed),
            "error_code": error.map(ModuleError::wire_code),
        });
        for (key, expected) in case["expected"].as_object().unwrap() {
            assert_eq!(&observed_results[key], expected, "{id}: {key}");
        }
    }
}

fn probe_registry(version: &str) -> Arc<Registry> {
    let registry = Arc::new(Registry::new());
    register_version(&registry, version);
    registry
}

fn register_version(registry: &Registry, version: &str) {
    let probe = Probe {
        schema: json!({"type": "object"}),
        requirements: false,
        starts: Arc::new(AtomicUsize::new(0)),
    };
    register_probe(registry, version, probe);
}

fn register_probe(registry: &Registry, version: &str, probe: Probe) {
    let descriptor = serde_json::from_value(json!({
        "module_id": "demo.policy",
        "version": version,
        "input_schema": probe.input_schema(),
        "output_schema": probe.output_schema(),
    }))
    .unwrap();
    registry
        .register("demo.policy", Box::new(probe), descriptor)
        .unwrap();
}

fn spy_evaluator(verdict: &str) -> Arc<Evaluator> {
    Arc::new(Evaluator {
        decision: verdict.to_string(),
        observed: Mutex::new(Vec::new()),
    })
}

fn pending_approver(supports_final_scope: bool) -> Arc<Approver> {
    Arc::new(Approver {
        pending: true,
        supports_final_scope,
        supports_authority_scope: false,
        requested: Mutex::new(Vec::new()),
        scope: Mutex::new(None),
        checks: AtomicUsize::new(0),
    })
}

fn delegated_context(subject: &str, scopes: &[&str]) -> Context<Value> {
    let principal = |id: &str| {
        Identity::new(
            id.into(),
            "user".into(),
            vec![],
            std::collections::HashMap::new(),
        )
    };
    let root = principal(subject)
        .with_authority(
            Some(vec!["read".into(), "write".into()]),
            Some("idp".into()),
        )
        .unwrap();
    Context::new(
        root.delegate(
            principal("agent"),
            scopes.iter().map(ToString::to_string).collect(),
        )
        .unwrap(),
    )
}

fn read_scoped_acl() -> ACL {
    ACL::new(
        serde_json::from_value(json!([{
            "effect": "allow", "callers": ["@external"], "targets": ["demo.policy"],
            "conditions": {"scopes": ["read"]}
        }]))
        .unwrap(),
        "deny",
        None,
    )
}

struct AlternateFlow;

#[async_trait]
impl Step for AlternateFlow {
    fn name(&self) -> &'static str {
        "alternate_flow"
    }

    fn description(&self) -> &'static str {
        "Route to execution after lookup"
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

    async fn execute(&self, _: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        Ok(StepResult::skip_to("execute"))
    }
}

#[tokio::test]
async fn scoped_invocation_requires_observed_acl_admission_on_the_active_path() {
    let mut strategy = apcore::builtin_steps::build_standard_strategy();
    strategy
        .insert_before("acl_check", Box::new(AlternateFlow))
        .unwrap();
    let mut executor =
        Executor::with_strategy(probe_registry("1.0.0"), Config::in_memory(), strategy);
    executor.set_acl(read_scoped_acl());
    let target = "demo.policy";
    let context = delegated_context("subject", &["read"]);
    let call = executor.call(target, json!({}), Some(&context), None).await;
    assert_eq!(call.unwrap_err().code, ErrorCode::ACLDenied);
    let stream = executor
        .stream(target, json!({}), Some(&context), None)
        .next()
        .await
        .unwrap();
    assert_eq!(stream.unwrap_err().code, ErrorCode::ACLDenied);
    assert!(
        !executor
            .validate(target, &json!({}), Some(&context))
            .await
            .unwrap()
            .valid
    );
}

#[tokio::test]
async fn scoped_calls_streams_preflights_and_discovery_require_the_enforcing_gate() {
    let registry = probe_registry("1.0.0");
    let target = "demo.policy";
    let context = delegated_context("subject", &["read"]);
    let absent = Executor::new(Arc::clone(&registry), Config::in_memory());
    let mut removed =
        Executor::with_strategy_name(Arc::clone(&registry), Config::in_memory(), "minimal")
            .unwrap();
    removed.set_acl(read_scoped_acl());
    for executor in [&absent, &removed] {
        assert_eq!(
            executor
                .call(target, json!({}), Some(&context), None)
                .await
                .unwrap_err()
                .code,
            ErrorCode::ACLDenied
        );
        assert_eq!(
            executor
                .stream(target, json!({}), Some(&context), None)
                .next()
                .await
                .unwrap()
                .unwrap_err()
                .code,
            ErrorCode::ACLDenied
        );
        let report = executor
            .validate(target, &json!({}), Some(&context))
            .await
            .unwrap();
        assert!(!report.valid);
        assert_eq!(report.checks.len(), 1);
        assert_eq!(
            report.checks[0].error.as_ref().unwrap()["code"],
            "ACL_DENIED"
        );
        assert!(executor
            .get_callable_descriptor(target, None, Some(&context))
            .is_none());
    }
    let mut guarded = Executor::new(registry, Config::in_memory());
    guarded.set_acl(read_scoped_acl());
    assert!(guarded
        .call(target, json!({}), Some(&context), None)
        .await
        .is_ok());
    assert!(guarded
        .stream(target, json!({}), Some(&context), None)
        .next()
        .await
        .unwrap()
        .is_ok());
    assert!(
        guarded
            .validate(target, &json!({}), Some(&context))
            .await
            .unwrap()
            .valid
    );
    assert!(guarded
        .get_callable_descriptor(target, None, Some(&context))
        .is_some());
    assert_eq!(
        guarded
            .call(
                target,
                json!({"_approval_token": TEST_CREDENTIAL}),
                Some(&context),
                None
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::ApprovalDenied
    );
}

#[tokio::test]
async fn scoped_approval_cannot_resume_under_a_different_subject_or_ceiling() {
    let mut executor = Executor::new(probe_registry("1.0.0"), Config::in_memory());
    executor.set_acl(read_scoped_acl());
    let policy = spy_evaluator("require_approval");
    executor.set_policy_evaluator(Some(policy.clone())).unwrap();
    let target = "demo.policy";
    let original = delegated_context("subject", &["read", "write"]);
    let unsupported = pending_approver(true);
    executor.set_approval_handler_shared(unsupported.clone());
    assert_eq!(
        executor
            .call(target, json!({}), Some(&original), None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::ApprovalDenied
    );
    let credential = json!({"_approval_token": TEST_CREDENTIAL});
    assert_eq!(
        executor
            .call(target, credential.clone(), Some(&original), None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::ApprovalDenied
    );
    assert_eq!(unsupported.checks.load(Ordering::Relaxed), 0);

    let mut handler = pending_approver(true);
    Arc::get_mut(&mut handler).unwrap().supports_authority_scope = true;
    executor.set_approval_handler_shared(handler.clone());
    let pending = executor
        .call(target, json!({}), Some(&original), None)
        .await
        .unwrap_err();
    assert_eq!(pending.code, ErrorCode::ApprovalPending);
    assert_eq!(pending.details["approval_id"], TEST_CREDENTIAL);
    let original_key = original.identity.as_ref().unwrap().authority_key();
    assert_eq!(
        handler.requested.lock().unwrap()[0].authority_key(),
        original_key
    );
    assert_eq!(
        policy.observed.lock().unwrap()[0].authority_key(),
        original_key
    );
    for different in [
        delegated_context("other-subject", &["read", "write"]),
        delegated_context("subject", &["read"]),
    ] {
        assert_eq!(
            executor
                .call(target, credential.clone(), Some(&different), None)
                .await
                .unwrap_err()
                .code,
            ErrorCode::ApprovalDenied
        );
        assert!(handler.scope.lock().unwrap().is_some());
    }
    assert!(executor
        .call(target, credential, Some(&original), None)
        .await
        .is_ok());
    assert!(handler.scope.lock().unwrap().is_none());
}

#[tokio::test]
async fn preflight_after_acl_denial_reaches_neither_policy_nor_preview() {
    let mut executor = Executor::new(probe_registry("1.0.0"), Config::in_memory());
    let evaluator = spy_evaluator("allow");
    let mut extensions = apcore::ExtensionManager::new();
    extensions
        .register(
            "policy_evaluator",
            apcore::ExtensionKind::PolicyEvaluator(evaluator.clone()),
        )
        .unwrap();
    let registry = Arc::clone(&executor.registry);
    extensions.apply(&registry, &mut executor).unwrap();
    executor.set_acl(ACL::new(vec![], "deny", None));
    let report = executor
        .validate("demo.policy", &json!({}), None)
        .await
        .unwrap();
    assert!(!report.valid);
    assert!(report
        .checks
        .iter()
        .all(|check| !matches!(check.check.as_str(), "module_preflight" | "module_preview")));
    assert!(evaluator.observed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn installed_policy_refuses_an_unguarded_strategy() {
    let registry = probe_registry("1.0.0");
    let evaluator = spy_evaluator("allow");
    let mut incompatible =
        Executor::with_strategy_name(Arc::clone(&registry), Config::in_memory(), "minimal")
            .unwrap();
    assert_eq!(
        incompatible
            .set_policy_evaluator(Some(evaluator.clone()))
            .unwrap_err()
            .code,
        ErrorCode::PipelineConfigurationError
    );
    let mut executor = Executor::new(registry, Config::in_memory());
    executor
        .set_policy_evaluator(Some(evaluator.clone()))
        .unwrap();
    assert_eq!(
        executor
            .call_with_trace(
                "demo.policy",
                json!({}),
                None,
                None,
                Some(&apcore::builtin_steps::build_minimal_strategy())
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::PipelineConfigurationError
    );
    assert!(evaluator.observed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn final_resume_checks_the_current_version_even_after_allow() {
    let registry = probe_registry("1.0.0");
    let mut executor = Executor::new(Arc::clone(&registry), Config::in_memory());
    let handler = pending_approver(true);
    executor.set_approval_handler_shared(handler.clone());
    executor
        .set_policy_evaluator(Some(spy_evaluator("require_approval")))
        .unwrap();
    let pending = executor
        .call("demo.policy", json!({}), None, None)
        .await
        .unwrap_err();
    assert_eq!(pending.code, ErrorCode::ApprovalPending);
    assert_eq!(pending.details["approval_id"], TEST_CREDENTIAL);
    registry.unregister("demo.policy").unwrap();
    register_version(&registry, "2.0.0");
    executor
        .set_policy_evaluator(Some(spy_evaluator("allow")))
        .unwrap();

    let result = executor
        .call(
            "demo.policy",
            json!({"_approval_token": TEST_CREDENTIAL}),
            None,
            Some("1.0.0"),
        )
        .await;
    assert_eq!(result.unwrap_err().code, ErrorCode::ApprovalDenied);
    assert_eq!(handler.checks.load(Ordering::Relaxed), 1);
    assert_eq!(handler.requested.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn unsupported_handler_cannot_expose_a_final_scope_credential() {
    let mut executor = Executor::new(probe_registry("1.0.0"), Config::in_memory());
    executor
        .set_policy_evaluator(Some(spy_evaluator("require_approval")))
        .unwrap();
    let handler = pending_approver(false);
    executor.set_approval_handler_shared(handler.clone());
    let result = executor
        .call("demo.policy", json!({}), None, None)
        .await
        .unwrap_err();
    assert_eq!(result.code, ErrorCode::ApprovalDenied);
    assert!(!result.details.contains_key("approval_id"));
    assert!(!result.to_dict().to_string().contains(TEST_CREDENTIAL));
    let result = executor
        .call(
            "demo.policy",
            json!({"_approval_token": TEST_CREDENTIAL}),
            None,
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(result.code, ErrorCode::ApprovalDenied);
    assert_eq!(handler.checks.load(Ordering::Relaxed), 0);
}

struct PendingPolicy(Arc<tokio::sync::Notify>);

#[async_trait]
impl PolicyEvaluator for PendingPolicy {
    async fn evaluate(
        &self,
        _request: &PolicyEvaluationRequest,
    ) -> Result<PolicyVerdict, ModuleError> {
        self.0.notify_one();
        std::future::pending().await
    }
}

#[tokio::test]
async fn policy_timeout_is_distinct_from_caller_cancellation() {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    let mut executor = Executor::new(probe_registry("1.0.0"), Config::in_memory());
    let entered = Arc::new(tokio::sync::Notify::new());
    executor
        .set_policy_evaluator(Some(Arc::new(PendingPolicy(Arc::clone(&entered)))))
        .unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let mut context = Context::anonymous();
    context.global_deadline = Some(now + 0.12);
    let timeout = tokio::time::timeout(
        Duration::from_millis(600),
        executor.call("demo.policy", json!({}), Some(&context), None),
    )
    .await
    .expect("evaluator must honor the earlier invocation deadline")
    .unwrap_err();
    assert_eq!(timeout.code, ErrorCode::PolicyEvaluationError);
    assert_eq!(timeout.to_dict()["retryable"], true);
    assert_eq!(timeout.to_dict()["user_fixable"], false);

    let cancel = apcore::CancelToken::new();
    let entered = Arc::new(tokio::sync::Notify::new());
    executor
        .set_policy_evaluator(Some(Arc::new(PendingPolicy(Arc::clone(&entered)))))
        .unwrap();
    context.global_deadline = None;
    context.cancel_token = Some(cancel.clone());
    let (result, ()) = tokio::join!(
        executor.call("demo.policy", json!({}), Some(&context), None),
        async {
            entered.notified().await;
            cancel.cancel();
        },
    );
    assert_eq!(result.unwrap_err().code, ErrorCode::ExecutionCancelled);
}

struct LateApproval {
    expiration: std::time::SystemTime,
    evaluations: AtomicUsize,
}

#[async_trait]
impl PolicyEvaluator for LateApproval {
    async fn evaluate(
        &self,
        _request: &PolicyEvaluationRequest,
    ) -> Result<PolicyVerdict, ModuleError> {
        self.evaluations.fetch_add(1, Ordering::Relaxed);
        let remaining = self
            .expiration
            .duration_since(std::time::SystemTime::now())
            .unwrap_or_default();
        // Deliberately do not yield before returning the verdict.
        std::thread::sleep(remaining + std::time::Duration::from_millis(20));
        Ok(PolicyVerdict {
            decision: PolicyEffect::RequireApproval,
            rule_id: None,
        })
    }
}

fn counted_policy_probe() -> (Registry, Arc<AtomicUsize>) {
    let registry = Registry::new();
    let starts = Arc::new(AtomicUsize::new(0));
    register_probe(
        &registry,
        "1.0.0",
        Probe {
            schema: json!({"type":"object", "properties":{"amount":{"type":"integer"}},"required":["amount"]}),
            requirements: false,
            starts: Arc::clone(&starts),
        },
    );
    (registry, starts)
}

#[tokio::test]
async fn a_late_nonyielding_verdict_cannot_advance_admission() {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    let (registry, starts) = counted_policy_probe();
    let mut executor = Executor::new(registry, Config::in_memory());
    let approver = pending_approver(true);
    executor.set_approval_handler_shared(approver.clone());
    for entry in ["call", "stream", "preflight"] {
        let expiration = SystemTime::now() + Duration::from_millis(200);
        let evaluator = Arc::new(LateApproval {
            expiration,
            evaluations: AtomicUsize::new(0),
        });
        executor
            .set_policy_evaluator(Some(evaluator.clone()))
            .unwrap();
        let mut context = Context::anonymous();
        context.global_deadline =
            Some(expiration.duration_since(UNIX_EPOCH).unwrap().as_secs_f64());

        let input = json!({"amount": 10});
        let error_code = match entry {
            "call" => executor
                .call("demo.policy", input, Some(&context), None)
                .await
                .unwrap_err()
                .code
                .wire_str(),
            "stream" => executor
                .stream("demo.policy", input, Some(&context), None)
                .next()
                .await
                .unwrap()
                .unwrap_err()
                .code
                .wire_str(),
            _ => {
                let report = executor
                    .validate("demo.policy", &input, Some(&context))
                    .await
                    .unwrap();
                assert!(!report.valid && !report.requires_approval);
                report
                    .checks
                    .iter()
                    .find(|check| check.check == "post_validation_policy")
                    .and_then(|check| check.error.as_ref())
                    .unwrap()["code"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            }
        };
        assert_eq!(evaluator.evaluations.load(Ordering::Relaxed), 1, "{entry}");
        assert_eq!(error_code, "POLICY_EVALUATION_ERROR", "{entry}");
    }
    assert!(approver.requested.lock().unwrap().is_empty());
    assert_eq!(starts.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn module_recovery_is_gated_by_successful_final_admission() {
    let (registry, starts) = counted_policy_probe();
    let mut executor = Executor::new(registry, Config::in_memory());
    let evaluator = spy_evaluator("allow");
    executor
        .set_policy_evaluator(Some(evaluator.clone()))
        .unwrap();
    executor
        .use_middleware(Box::new(Rewrite::Amount(json!("invalid"))))
        .unwrap();

    let input = json!({"amount": 10});
    let error = executor
        .call("demo.policy", input.clone(), None, None)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::SchemaValidationError);
    let error = executor
        .stream("demo.policy", input.clone(), None, None)
        .next()
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::SchemaValidationError);
    assert!(evaluator.observed.lock().unwrap().is_empty());
    assert_eq!(starts.load(Ordering::Relaxed), 0);

    // Legacy calls retain recovery from the same pre-dispatch failure.
    executor.set_policy_evaluator(None).unwrap();
    assert_eq!(
        executor
            .call("demo.policy", input, None, None)
            .await
            .unwrap(),
        json!({"recovered":true})
    );

    // Once the policy admits an invocation, execution errors remain recoverable.
    executor.remove("fixture-rewrite");
    executor
        .use_middleware(Box::new(Rewrite::Amount(json!(10))))
        .unwrap();
    executor.set_policy_evaluator(Some(evaluator)).unwrap();
    let input = json!({"amount": 10, "fail_execution": true});
    assert_eq!(
        executor
            .call("demo.policy", input.clone(), None, None)
            .await
            .unwrap(),
        json!({"recovered":true})
    );
    assert_eq!(
        executor
            .stream("demo.policy", input, None, None)
            .next()
            .await
            .unwrap()
            .unwrap(),
        json!({"recovered":true})
    );
    assert_eq!(starts.load(Ordering::Relaxed), 2);
}

struct StepRecovery(AtomicUsize);

#[async_trait]
impl StepMiddleware for StepRecovery {
    async fn on_step_error(
        &self,
        _step_name: &str,
        _state: &PipelineState<'_>,
        _error: &ModuleError,
    ) -> Result<Option<Value>, ModuleError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(Some(json!({"recovered": true})))
    }
}

#[tokio::test]
async fn step_recovery_cannot_advance_a_failed_normalization_to_policy() {
    let (registry, starts) = counted_policy_probe();
    let observer = Arc::new(StepRecovery(AtomicUsize::new(0)));
    let mut strategy = apcore::builtin_steps::build_standard_strategy();
    strategy.add_step_middleware(observer.clone());
    let mut executor = Executor::with_strategy(registry, Config::in_memory(), strategy);
    let evaluator = spy_evaluator("allow");
    executor
        .set_policy_evaluator(Some(evaluator.clone()))
        .unwrap();
    executor.use_middleware(Box::new(Rewrite::Reject)).unwrap();

    let input = json!({"amount": 10});
    let error = executor
        .call("demo.policy", input.clone(), None, None)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::GeneralInvalidInput);
    let error = executor
        .stream("demo.policy", input.clone(), None, None)
        .next()
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::GeneralInvalidInput);
    assert!(evaluator.observed.lock().unwrap().is_empty());
    assert_eq!(starts.load(Ordering::Relaxed), 0);
    assert_eq!(observer.0.load(Ordering::Relaxed), 2);

    // Dry-run preflight still reads the submitted input without normalizing it.
    let report = executor
        .validate("demo.policy", &input, None)
        .await
        .unwrap();
    assert!(report.valid);
    assert_eq!(evaluator.observed.lock().unwrap().len(), 1);
}
