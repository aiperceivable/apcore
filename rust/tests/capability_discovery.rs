use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use apcore::{
    CapabilityQuery, CapabilitySearchProvider, Config, Context, Executor, Identity, Module,
    ModuleAnnotations, ModuleError, PolicyEvaluationRequest, PolicyEvaluator, PolicyVerdict,
    Registry, ACL,
};
use async_trait::async_trait;
use serde_json::{json, Value};

use super::conformance_env;

struct DiscoveryModule {
    annotations: ModuleAnnotations,
    description: String,
    tags: Vec<String>,
}

#[async_trait]
impl Module for DiscoveryModule {
    fn description(&self) -> &str {
        &self.description
    }

    fn tags(&self) -> Vec<String> {
        self.tags.clone()
    }

    fn annotations(&self) -> ModuleAnnotations {
        self.annotations.clone()
    }

    fn input_schema(&self) -> Value {
        schema()
    }

    fn output_schema(&self) -> Value {
        schema()
    }

    async fn execute(&self, _: Value, _: &Context<Value>) -> Result<Value, ModuleError> {
        panic!("discovery must not execute a module")
    }
}

fn schema() -> Value {
    json!({"type":"object", "properties":{"message":{"type":"string"}}})
}

struct NeverEvaluate;

#[async_trait]
impl PolicyEvaluator for NeverEvaluate {
    async fn evaluate(&self, _: &PolicyEvaluationRequest) -> Result<PolicyVerdict, ModuleError> {
        panic!("discovery must not call the policy evaluator")
    }
}

fn reader(id: &str, roles: Vec<String>) -> Context<Value> {
    Context::new(Identity::new(
        id.into(),
        "user".into(),
        roles,
        HashMap::new(),
    ))
}

fn register(executor: &Executor, fixture: &Value) {
    let module_id = fixture["module_id"].as_str().unwrap();
    let module = Box::new(DiscoveryModule {
        annotations: ModuleAnnotations {
            discoverable: fixture["discoverable"].as_bool().unwrap_or(true),
            readonly: fixture["readonly"].as_bool().unwrap_or(false),
            requires_approval: fixture["requires_approval"].as_bool().unwrap_or(false),
            ..Default::default()
        },
        description: fixture["description"].as_str().unwrap_or("A tool").into(),
        tags: serde_json::from_value(fixture.get("tags").cloned().unwrap_or(json!([]))).unwrap(),
    });
    executor
        .registry()
        .register_module(module_id, module)
        .unwrap();
}

#[test]
fn test_capability_discovery_conformance() {
    let fixture_path = conformance_env::find_fixtures_root().join("capability_discovery.json");
    let fixture: Value = serde_json::from_slice(&std::fs::read(fixture_path).unwrap()).unwrap();
    for case in fixture["test_cases"].as_array().unwrap() {
        let case_id = case["id"].as_str().unwrap();
        let input = &case["input"];
        let expected = &case["expected"];
        let audit = Arc::new(AtomicUsize::new(0));
        let audit_capture = Arc::clone(&audit);
        let mut executor = Executor::new(Registry::new(), Config::default());
        executor.set_acl(ACL::new(
            serde_json::from_value(input["rules"].clone()).unwrap(),
            "deny",
            Some(Arc::new(move |_| {
                audit_capture.fetch_add(1, Ordering::Relaxed);
            })),
        ));
        for module in fixture["driver_contract"]["modules"].as_array().unwrap() {
            register(&executor, module);
        }
        if input["policy_evaluator"].as_bool() == Some(true) {
            executor
                .set_policy_evaluator(Some(Arc::new(NeverEvaluate)))
                .unwrap();
        }
        let mut context = reader(
            "test-user",
            serde_json::from_value(input.get("roles").cloned().unwrap_or(json!([]))).unwrap(),
        );
        if let Some(caller_id) = input["caller_id"].as_str() {
            context.call_chain.push(caller_id.to_owned());
        }
        let options =
            serde_json::from_value(input.get("options").cloned().unwrap_or(json!({}))).unwrap();
        let page = executor.list_callable(Some(&context), options).unwrap();
        let items: Vec<Value> = page
            .items
            .iter()
            .map(|item| {
                assert_eq!(item.version, "1.0.0", "{case_id}");
                let encoded = serde_json::to_value(item).unwrap();
                assert!(encoded.get("input_schema").is_none(), "{case_id}");
                assert!(encoded.get("output_schema").is_none(), "{case_id}");
                json!({"module_id": item.module_id, "access_hint": item.access_hint})
            })
            .collect();
        assert_eq!(json!(items), expected["items"], "{case_id}");
        let definitions = executor.get_callable_definitions(
            &[
                "demo.hidden",
                "demo.echo",
                "demo.review",
                "demo.unknown",
                "demo.echo",
            ],
            Some(&context),
        );
        let names: Vec<_> = definitions
            .iter()
            .map(|definition| {
                assert_eq!(definition.version, "1.0.0", "{case_id}");
                assert_eq!(definition.input_schema, schema(), "{case_id}");
                assert_eq!(definition.output_schema, schema(), "{case_id}");
                &definition.module_id
            })
            .collect();
        let expected_names = expected.get("definitions").cloned().unwrap_or_else(|| {
            json!(items
                .iter()
                .map(|item| &item["module_id"])
                .collect::<Vec<_>>())
        });
        assert_eq!(json!(names), expected_names, "{case_id}");
        assert_eq!(
            json!(audit.load(Ordering::Relaxed)),
            expected["audit_entries"],
            "{case_id}"
        );
    }
}

struct BroadProvider;

impl CapabilitySearchProvider for BroadProvider {
    fn search(&self, _: Option<&str>, visible_ids: &[String]) -> Vec<String> {
        assert_eq!(visible_ids, &["demo.a", "demo.b"]);
        vec![
            "demo.hidden".into(),
            "demo.disabled".into(),
            "demo.b".into(),
            "demo.b".into(),
            "demo.a".into(),
        ]
    }
}

#[test]
fn test_capability_cursor_is_bound_and_rechecks_current_registrations() {
    let executor = Executor::new(Registry::new(), Config::default());
    for id in ["demo.a", "demo.b", "demo.disabled"] {
        register(&executor, &json!({"module_id": id}));
    }
    register(
        &executor,
        &json!({"module_id": "demo.hidden", "discoverable": false}),
    );
    executor.registry().disable("demo.disabled").unwrap();
    executor.set_capability_search_provider(Some(Arc::new(BroadProvider)));
    let owner = reader("owner", vec!["reader".into()]);
    let options = CapabilityQuery {
        limit: 1,
        ..Default::default()
    };
    let first = executor
        .list_callable(Some(&owner), options.clone())
        .unwrap();
    assert_eq!(first.items[0].module_id, "demo.b");
    let second = CapabilityQuery {
        cursor: first.next_cursor,
        ..options
    };
    assert!(executor
        .list_callable(
            Some(&reader("intruder", vec!["reader".into()])),
            second.clone()
        )
        .is_err());
    assert!(executor
        .list_callable(
            Some(&reader("owner", vec!["writer".into()])),
            second.clone()
        )
        .is_err());
    executor.registry().disable("demo.a").unwrap();
    let resumed = executor
        .list_callable(Some(&owner), second.clone())
        .unwrap();
    assert_eq!(resumed.items, []);
    executor.set_capability_search_provider(None);
    assert!(executor.list_callable(Some(&owner), second).is_err());
}

#[test]
fn test_scoped_capability_cursor_rejects_new_subject_or_effective_scope() {
    let mut executor = Executor::new(Registry::new(), Config::default());
    executor.set_acl(ACL::new(
        serde_json::from_value(json!([{
            "callers": ["@external"], "targets": ["demo.*"],
            "effect": "allow", "conditions": {"scopes": ["read"]}
        }]))
        .unwrap(),
        "deny",
        None,
    ));
    for id in ["demo.a", "demo.b"] {
        register(&executor, &json!({"module_id": id}));
    }
    let delegated = |subject, scopes| {
        let root = reader(subject, vec!["eligible".into()])
            .identity
            .unwrap()
            .with_authority(Some(vec!["read".into(), "write".into()]), None)
            .unwrap();
        let actor = reader("shared-actor", vec![]).identity.unwrap();
        Context::new(root.delegate(actor, scopes).unwrap())
    };
    let owner = delegated("subject", vec!["read".into(), "write".into()]);
    let query = CapabilityQuery {
        limit: 1,
        ..Default::default()
    };
    let first = executor.list_callable(Some(&owner), query.clone()).unwrap();
    assert_eq!(first.items[0].module_id, "demo.a");
    assert!(first.next_cursor.is_some());
    let next = CapabilityQuery {
        cursor: first.next_cursor,
        ..query
    };
    for other in [
        delegated("different-subject", vec!["read".into(), "write".into()]),
        delegated("subject", vec!["read".into()]),
    ] {
        assert!(executor.list_callable(Some(&other), next.clone()).is_err());
    }
    let equivalent = delegated(
        "subject",
        vec!["write".into(), "read".into(), "read".into()],
    );
    let last = executor.list_callable(Some(&equivalent), next).unwrap();
    assert_eq!(last.items[0].module_id, "demo.b");
}

#[test]
fn test_capability_descriptors_check_current_version_and_same_version_replacement() {
    let executor = Executor::new(Registry::new(), Config::default());
    register(
        &executor,
        &json!({"module_id": "demo.replace", "description": "old metadata"}),
    );
    let old = executor
        .get_callable_descriptor("demo.replace", Some("1.0.0"), None)
        .unwrap();
    assert_eq!(old.description, "old metadata");
    assert!(executor
        .get_callable_descriptor("demo.replace", Some("2.0.0"), None)
        .is_none());
    executor.registry().unregister("demo.replace").unwrap();
    register(
        &executor,
        &json!({"module_id": "demo.replace", "description": "live replacement"}),
    );
    let live = executor
        .get_callable_descriptor("demo.replace", Some("1.0.0"), None)
        .unwrap();
    assert_eq!(live.description, "live replacement");
    assert_eq!(old.description, "old metadata");
}

#[cfg(feature = "sys-modules")]
#[tokio::test]
async fn test_manifest_search_uses_the_originating_frame_and_acl() {
    let mut config = Config::default();
    config.set("sys_modules.enabled", json!(true));
    let mut executor = Executor::new(Registry::new(), config.clone());
    let rules = serde_json::from_value(json!([
        {"callers": ["worker.reader"], "targets": ["system.manifest.search"], "effect": "allow"},
        {"callers": ["system.manifest.search"], "targets": ["demo.privileged"], "effect": "allow"},
        {"callers": ["worker.reader"], "targets": ["demo.ordinary"], "effect": "allow",
         "conditions": {"max_call_depth": 2}}
    ]))
    .unwrap();
    executor.set_acl(ACL::new(rules, "deny", None));
    register(&executor, &json!({"module_id": "demo.privileged"}));
    register(&executor, &json!({"module_id": "demo.ordinary"}));
    let client = apcore::APCore::try_start(apcore::APCoreOptions {
        executor: Some(executor),
        config: Some(config),
        ..Default::default()
    })
    .unwrap();
    let mut requester = reader("requester", vec![]);
    requester.call_chain.push("worker.reader".into());
    let args = json!({"prefix": "demo."});
    let result = client
        .call(
            "system.manifest.search",
            args.clone(),
            Some(&requester),
            None,
        )
        .await
        .unwrap();
    assert_eq!(result["items"].as_array().unwrap().len(), 1);
    assert_eq!(result["items"][0]["module_id"], "demo.ordinary");
    assert_eq!(result["next_cursor"], Value::Null);
    assert!(client
        .call(
            "system.manifest.search",
            json!({"query": null}),
            Some(&requester),
            None
        )
        .await
        .is_err());
    assert!(client
        .call("system.manifest.search", args, None, None)
        .await
        .is_err());
}

#[test]
fn test_capability_discovery_fails_closed_without_the_stock_gate() {
    let mut strategy = apcore::builtin_steps::build_standard_strategy();
    strategy.remove("acl_check").unwrap();
    let executor = Executor::with_strategy(Registry::new(), Config::default(), strategy);
    register(&executor, &json!({"module_id": "demo.private"}));
    let page = executor
        .list_callable(None, CapabilityQuery::default())
        .unwrap();
    assert_eq!(page.items, []);
    assert!(executor
        .get_callable_descriptor("demo.private", None, None)
        .is_none());
}

#[tokio::test]
async fn test_capability_visibility_requires_context_creation_for_the_target() {
    let strategy = apcore::pipeline_config::build_strategy_from_config(&json!({
        "configure": {"context_creation": {"match_modules": ["demo.included"]}}
    }))
    .unwrap();
    let mut executor = Executor::with_strategy(Registry::new(), Config::default(), strategy);
    executor.set_acl(ACL::new(
        vec![apcore::ACLRule::new(
            vec!["worker.reader".into()],
            vec!["demo.*".into()],
            "allow",
        )],
        "deny",
        None,
    ));
    for module_id in ["demo.included", "demo.skipped"] {
        register(&executor, &json!({"module_id": module_id}));
    }
    let mut parent = reader("requester", vec![]);
    parent.call_chain.push("worker.reader".into());
    let denied = executor
        .call("demo.skipped", json!({}), Some(&parent), None)
        .await
        .unwrap_err();
    assert_eq!(denied.code, apcore::ErrorCode::ACLDenied);

    let page = executor
        .list_callable(Some(&parent), CapabilityQuery::default())
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].module_id, "demo.included");
    assert!(executor
        .get_callable_descriptor("demo.skipped", None, Some(&parent))
        .is_none());
}

#[test]
fn test_capability_hint_tracks_the_legacy_unguarded_approval_profile() {
    let mut executor = Executor::new(Registry::new(), Config::default());
    let mut acl_rule = apcore::ACLRule::new(
        vec!["@external".into()],
        vec!["demo.approve".into()],
        "allow",
    );
    acl_rule.approval = Some(apcore::ApprovalRequirement::Required);
    executor.set_acl(ACL::new(vec![acl_rule], "deny", None));
    executor.set_policy(Some(apcore::ExecutionPolicy::default().with_strict(false)));
    register(
        &executor,
        &json!({"module_id": "demo.approve", "requires_approval": true}),
    );

    let page = executor
        .list_callable(None, CapabilityQuery::default())
        .unwrap();
    assert_eq!(page.items[0].access_hint, apcore::AccessHint::Allowed);

    executor
        .set_policy_evaluator(Some(Arc::new(NeverEvaluate)))
        .unwrap();
    let page = executor
        .list_callable(None, CapabilityQuery::default())
        .unwrap();
    assert_eq!(
        page.items[0].access_hint,
        apcore::AccessHint::RequiresApproval
    );
}
