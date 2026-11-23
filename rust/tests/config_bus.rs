// Integration tests for Config Bus (§9.4–§9.15, v0.15.0)

use std::collections::HashMap;

use apcore::config::{
    Config, ConfigMode, EnvStyle, MountSource, NamespaceRegistration, DEFAULT_MAX_DEPTH,
};
use apcore::errors::ErrorCode;
use serde::Deserialize;

/// Populate the spec-mandated required fields on a `from_defaults()` config so
/// that legacy-mode `validate()` passes. Unlike apcore-python, Rust's
/// `from_defaults()` does not seed these top-level fields, so tests that assert
/// a successful `validate()` must supply them (A-D-03 required-field contract).
fn with_required_fields(cfg: &mut Config) {
    cfg.set("version", serde_json::json!("0.23.0"));
    cfg.set("project.name", serde_json::json!("demo"));
    cfg.set("extensions.root", serde_json::json!("./extensions"));
    cfg.set("schema.root", serde_json::json!("./schemas"));
    cfg.set("acl.root", serde_json::json!("./acl"));
    cfg.set("acl.default_effect", serde_json::json!("deny"));
}

// ---------------------------------------------------------------------------
// Namespace registration
// ---------------------------------------------------------------------------

#[test]
fn test_register_namespace_reserved_name_returns_error() {
    let result = Config::register_namespace(NamespaceRegistration {
        name: "apcore".to_string(),
        env_prefix: None,
        defaults: None,
        schema: None,
        env_style: EnvStyle::Nested,
        max_depth: DEFAULT_MAX_DEPTH,
        env_map: None,
    });
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, ErrorCode::ConfigNamespaceReserved);
}

#[test]
fn test_register_namespace_reserved_config_name_returns_error() {
    let result = Config::register_namespace(NamespaceRegistration {
        name: "_config".to_string(),
        env_prefix: None,
        defaults: None,
        schema: None,
        env_style: EnvStyle::Nested,
        max_depth: DEFAULT_MAX_DEPTH,
        env_map: None,
    });
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, ErrorCode::ConfigNamespaceReserved);
}

// PROTOCOL_SPEC §9.9.5 — Reserved namespace public query API.
#[test]
fn test_reserved_namespaces_contains_apcore_and_config() {
    let reserved = Config::reserved_namespaces();
    assert!(reserved.contains(&"apcore"));
    assert!(reserved.contains(&"_config"));
}

#[test]
fn test_reserved_namespaces_matches_public_const() {
    // Single source of truth (§9.9.5 req 2): the query API and the public
    // const MUST yield the same set of names. We compare by value because
    // Rust may inline `const` items at each use site (slice descriptors are
    // not guaranteed pointer-equal even when referring to the same data).
    let from_fn = Config::reserved_namespaces();
    let from_const: &'static [&'static str] = apcore::RESERVED_NAMESPACES;
    assert_eq!(from_fn, from_const);
}

#[test]
fn test_reserved_namespaces_drive_register_namespace_enforcement() {
    // Behavioural single-source-of-truth: every name returned by the query
    // API MUST be rejected by `register_namespace`. This is the contract
    // §9.9.5 req 2 actually cares about.
    for name in Config::reserved_namespaces() {
        let result = Config::register_namespace(NamespaceRegistration {
            name: (*name).to_string(),
            env_prefix: None,
            defaults: None,
            schema: None,
            env_style: EnvStyle::Nested,
            max_depth: DEFAULT_MAX_DEPTH,
            env_map: None,
        });
        assert!(result.is_err(), "register_namespace({name:?}) should fail");
        assert_eq!(
            result.unwrap_err().code,
            ErrorCode::ConfigNamespaceReserved,
            "register_namespace({name:?}) should return CONFIG_NAMESPACE_RESERVED",
        );
    }
}

#[test]
fn test_reserved_namespaces_callable_without_instance() {
    // Associated function — no `Config` instance constructed.
    let _ = Config::reserved_namespaces();
}

#[test]
fn test_register_namespace_env_prefix_duplicate_raises() {
    // Duplicate env_prefix should raise ConfigEnvPrefixConflict.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let prefix = format!("DUP_PREFIX_{ts}");
    Config::register_namespace(NamespaceRegistration {
        name: format!("dup_pfx_a_{ts}"),
        env_prefix: Some(prefix.clone()),
        defaults: None,
        schema: None,
        env_style: EnvStyle::Nested,
        max_depth: DEFAULT_MAX_DEPTH,
        env_map: None,
    })
    .unwrap();
    let result = Config::register_namespace(NamespaceRegistration {
        name: format!("dup_pfx_b_{ts}"),
        env_prefix: Some(prefix),
        defaults: None,
        schema: None,
        env_style: EnvStyle::Nested,
        max_depth: DEFAULT_MAX_DEPTH,
        env_map: None,
    });
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, ErrorCode::ConfigEnvPrefixConflict);
}

#[test]
fn test_register_namespace_success_and_list() {
    let name = format!(
        "test_ns_list_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    );

    Config::register_namespace(NamespaceRegistration {
        name: name.clone(),
        env_prefix: None,
        defaults: Some(serde_json::json!({"key": "value"})),
        schema: None,
        env_style: EnvStyle::Nested,
        max_depth: DEFAULT_MAX_DEPTH,
        env_map: None,
    })
    .unwrap();

    let namespaces = Config::registered_namespaces();
    let found = namespaces.iter().any(|n| n.name == name);
    assert!(found, "registered namespace must appear in list");
}

#[test]
fn test_register_namespace_duplicate_returns_error() {
    let name = format!(
        "test_dup_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    );

    Config::register_namespace(NamespaceRegistration {
        name: name.clone(),
        env_prefix: None,
        defaults: None,
        schema: None,
        env_style: EnvStyle::Nested,
        max_depth: DEFAULT_MAX_DEPTH,
        env_map: None,
    })
    .unwrap();

    let result = Config::register_namespace(NamespaceRegistration {
        name: name.clone(),
        env_prefix: None,
        defaults: None,
        schema: None,
        env_style: EnvStyle::Nested,
        max_depth: DEFAULT_MAX_DEPTH,
        env_map: None,
    });
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err().code,
        ErrorCode::ConfigNamespaceDuplicate
    );
}

// ---------------------------------------------------------------------------
// Mode detection
// ---------------------------------------------------------------------------

#[test]
fn test_mode_detection_legacy_from_defaults() {
    let config = Config::from_defaults();
    // from_defaults() without an "apcore" top-level key → Legacy mode.
    assert_eq!(config.mode, ConfigMode::Legacy);
}

// ---------------------------------------------------------------------------
// Namespace get / set
// ---------------------------------------------------------------------------

#[test]
fn test_namespace_mode_get_set_dot_path() {
    let mut config = Config::from_defaults();
    // Force namespace mode so we can exercise namespace path handling.
    config.mode = ConfigMode::Namespace;
    config
        .user_namespaces
        .insert("apcore".to_string(), serde_json::json!({}));

    config.set(
        "my-adapter.transport",
        serde_json::Value::String("grpc".to_string()),
    );
    let val = config.get("my-adapter.transport");
    assert_eq!(val, Some(serde_json::Value::String("grpc".to_string())));
}

#[test]
fn test_namespace_method_returns_subtree() {
    let mut config = Config::from_defaults();
    config.mode = ConfigMode::Namespace;
    config.user_namespaces.insert(
        "widget".to_string(),
        serde_json::json!({"color": "blue", "size": 42}),
    );

    let ns = config.namespace("widget");
    assert_eq!(ns["color"], serde_json::json!("blue"));
    assert_eq!(ns["size"], serde_json::json!(42));
}

#[test]
fn test_namespace_method_returns_empty_for_unknown() {
    let config = Config::from_defaults();
    assert!(config.namespace("nonexistent_ns_xyz").is_empty());
}

// ---------------------------------------------------------------------------
// Mount
// ---------------------------------------------------------------------------

#[test]
fn test_mount_dict_merges_into_namespace() {
    let mut config = Config::from_defaults();
    config.mode = ConfigMode::Namespace;
    config.user_namespaces.insert(
        "plugins".to_string(),
        serde_json::json!({"existing_key": true}),
    );

    config
        .mount(
            "plugins",
            MountSource::Dict(serde_json::json!({"new_key": "hello"})),
        )
        .unwrap();

    let ns = config.namespace("plugins");
    assert_eq!(ns["existing_key"], serde_json::json!(true));
    assert_eq!(ns["new_key"], serde_json::json!("hello"));
}

#[test]
fn test_mount_non_object_returns_error() {
    let mut config = Config::from_defaults();
    let result = config.mount("plugins", MountSource::Dict(serde_json::json!([1, 2, 3])));
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, ErrorCode::ConfigMountError);
}

// ---------------------------------------------------------------------------
// bind / get_typed
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, PartialEq)]
struct PluginConfig {
    enabled: bool,
    max_workers: u32,
}

#[test]
fn test_bind_namespace_into_typed_struct() {
    let mut config = Config::from_defaults();
    config.mode = ConfigMode::Namespace;
    config.user_namespaces.insert(
        "plugin_bind_test".to_string(),
        serde_json::json!({"enabled": true, "max_workers": 4}),
    );

    let result: PluginConfig = config.bind("plugin_bind_test").unwrap();
    assert!(result.enabled);
    assert_eq!(result.max_workers, 4);
}

#[test]
fn test_bind_unknown_namespace_returns_error() {
    let config = Config::from_defaults();
    let result: Result<PluginConfig, _> = config.bind("totally_unknown_ns");
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, ErrorCode::ConfigBindError);
}

// Regression: sync finding A-D-018 — bind() on a missing namespace must
// silently bind into an empty object (so `T`'s serde defaults take effect),
// matching apcore-python's `_instantiate_model(model, {}, namespace)` and
// apcore-typescript's `new schema({})`. Previously Rust returned
// ConfigBindError("namespace not found"), breaking portable code that
// relied on default-fill behavior.
#[derive(Debug, Deserialize, PartialEq, Default)]
struct AllDefaultsConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    max_workers: u32,
}

#[test]
fn test_bind_missing_namespace_uses_empty_object_for_serde_defaults() {
    let config = Config::from_defaults();
    let result: AllDefaultsConfig = config
        .bind("totally_unknown_ns_with_defaults")
        .expect("bind into all-default struct must succeed when namespace is missing");
    assert_eq!(result, AllDefaultsConfig::default());
}

// Regression [config-bind-defaults]: bind() must deserialize from the merged
// namespace view (registered defaults + loaded YAML), not the raw
// user_namespaces entry. A registered default absent from YAML must be present
// in the bound struct. Mirrors apcore-python / apcore-typescript.
#[derive(Debug, Deserialize, PartialEq)]
struct ServiceConfig {
    host: String,
    port: u16,
}

#[test]
fn test_bind_includes_registered_defaults_absent_from_yaml() {
    use apcore::{EnvStyle, NamespaceRegistration};

    let ns_name = format!(
        "svc-bind-defaults-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let _ = Config::register_namespace(NamespaceRegistration {
        name: ns_name.clone(),
        env_prefix: None,
        // `port` is a registered default; only `host` will be in the YAML.
        defaults: Some(serde_json::json!({"port": 8080})),
        schema: None,
        env_style: EnvStyle::Auto,
        max_depth: 5,
        env_map: None,
    });

    let mut config = Config::from_defaults();
    config.mode = ConfigMode::Namespace;
    config
        .user_namespaces
        .insert(ns_name.clone(), serde_json::json!({"host": "db.internal"}));

    let bound: ServiceConfig = config
        .bind(&ns_name)
        .expect("bind must merge registered defaults before deserializing");
    assert_eq!(
        bound,
        ServiceConfig {
            host: "db.internal".to_string(),
            port: 8080,
        },
        "registered default 'port' must be present even though absent from YAML"
    );
}

// Regression: sync finding A-D-017 — Config::get must perform longest-prefix
// match against registered namespaces so hyphenated names route correctly.
// Apcore-python and apcore-typescript both sort registered names by length
// descending and match the longest prefix.
#[test]
fn test_get_supports_hyphenated_namespace_via_longest_prefix_match() {
    use apcore::{EnvStyle, NamespaceRegistration};

    // Use a unique name to avoid global-state collisions across tests.
    let ns_name = format!(
        "apcore-test-hyphenated-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let _ = Config::register_namespace(NamespaceRegistration {
        name: ns_name.clone(),
        env_prefix: None,
        defaults: None,
        schema: None,
        env_style: EnvStyle::Auto,
        max_depth: 5,
        env_map: None,
    });

    let mut config = Config::from_defaults();
    config.mode = ConfigMode::Namespace;
    config.user_namespaces.insert(
        ns_name.clone(),
        serde_json::json!({"transport": {"endpoint": "https://example.com"}}),
    );

    // Naive `key.split('.')` would route `<ns>.transport.endpoint` to
    // `user_namespaces.get("apcore-test-hyphenated-...")` only if the split
    // happens to leave the hyphenated prefix intact. The longest-prefix
    // match guarantees it.
    let key = format!("{ns_name}.transport.endpoint");
    let value = config.get(&key);
    assert_eq!(
        value,
        Some(serde_json::json!("https://example.com")),
        "Config::get must resolve hyphenated namespace prefixes via longest-match"
    );
}

#[test]
fn test_get_typed_returns_value() {
    let mut config = Config::from_defaults();
    config.mode = ConfigMode::Namespace;
    config
        .user_namespaces
        .insert("svc".to_string(), serde_json::json!({"port": 8080}));

    let port: u64 = config.get_typed("svc.port").unwrap();
    assert_eq!(port, 8080);
}

// ---------------------------------------------------------------------------
// Built-in namespaces (§9.15)
// ---------------------------------------------------------------------------

#[test]
fn test_builtin_namespaces_observability_registered() {
    // from_defaults() triggers init_builtin_namespaces().
    let _ = Config::from_defaults();
    let ns = Config::registered_namespaces();
    let has_observability = ns.iter().any(|n| n.name == "observability");
    assert!(
        has_observability,
        "observability namespace must be built-in"
    );
}

#[test]
fn test_builtin_namespaces_sys_modules_registered() {
    let _ = Config::from_defaults();
    let ns = Config::registered_namespaces();
    let has_sys = ns.iter().any(|n| n.name == "sys_modules");
    assert!(has_sys, "sys_modules namespace must be built-in");
}

// ---------------------------------------------------------------------------
// env_style flat
// ---------------------------------------------------------------------------

#[test]
fn test_env_style_flat_preserves_underscores() {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let ns_name = format!("flatns-{ts}");
    let prefix = format!("FLAT_{ts}");

    Config::register_namespace(NamespaceRegistration {
        name: ns_name.clone(),
        env_prefix: Some(prefix.clone()),
        defaults: None,
        schema: None,
        env_style: EnvStyle::Flat,
        max_depth: DEFAULT_MAX_DEPTH,
        env_map: None,
    })
    .unwrap();

    // Set env vars with underscores in the key name.
    let key1 = format!("{prefix}_DEVTO_API_KEY");
    let key2 = format!("{prefix}_LLM_MODEL");
    std::env::set_var(&key1, "abc123");
    std::env::set_var(&key2, "gemini-pro");

    // Load from a namespace-mode YAML file so env overrides are applied.
    let tmp_dir = std::env::temp_dir().join(format!("apcore-flat-test-{ts}"));
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let yaml_path = tmp_dir.join("cfg.yaml");
    std::fs::write(
        &yaml_path,
        "executor:\n  max_call_depth: 32\n  max_module_repeat: 3\n  default_timeout: 30000\n  global_timeout: 60000\napcore:\n  version: '1.0.0'\n",
    )
    .unwrap();
    let config = Config::load(&yaml_path).unwrap();

    // Flat style: underscores preserved, no nesting.
    assert_eq!(
        config.get(&format!("{ns_name}.devto_api_key")),
        Some(serde_json::json!("abc123"))
    );
    assert_eq!(
        config.get(&format!("{ns_name}.llm_model")),
        Some(serde_json::json!("gemini-pro"))
    );

    // Cleanup.
    std::env::remove_var(&key1);
    std::env::remove_var(&key2);
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

const NS_YAML: &str = "executor:\n  max_call_depth: 32\n  max_module_repeat: 3\n  default_timeout: 30000\n  global_timeout: 60000\napcore:\n  version: '1.0.0'\n";

#[test]
fn test_env_style_auto_resolves_mixed_flat_and_nested() {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let ns_name = format!("autons-{ts}");
    let prefix = format!("AUTO_{ts}");

    Config::register_namespace(NamespaceRegistration {
        name: ns_name.clone(),
        env_prefix: Some(prefix.clone()),
        defaults: Some(serde_json::json!({
            "devto_api_key": "",
            "publish": { "delay": 5, "retry": 3 }
        })),
        schema: None,
        env_style: EnvStyle::Auto,
        max_depth: DEFAULT_MAX_DEPTH,
        env_map: None,
    })
    .unwrap();

    let k1 = format!("{prefix}_DEVTO_API_KEY");
    let k2 = format!("{prefix}_PUBLISH_DELAY");
    let k3 = format!("{prefix}_PUBLISH_RETRY");
    std::env::set_var(&k1, "abc123");
    std::env::set_var(&k2, "10");
    std::env::set_var(&k3, "7");

    let tmp_dir = std::env::temp_dir().join(format!("apcore-auto-test-{ts}"));
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let yaml_path = tmp_dir.join("cfg.yaml");
    std::fs::write(&yaml_path, NS_YAML).unwrap();
    let config = Config::load(&yaml_path).unwrap();

    // Flat key matched.
    assert_eq!(
        config.get(&format!("{ns_name}.devto_api_key")),
        Some(serde_json::json!("abc123"))
    );
    // Nested keys matched.
    assert_eq!(
        config.get(&format!("{ns_name}.publish.delay")),
        Some(serde_json::json!(10))
    );
    assert_eq!(
        config.get(&format!("{ns_name}.publish.retry")),
        Some(serde_json::json!(7))
    );

    std::env::remove_var(&k1);
    std::env::remove_var(&k2);
    std::env::remove_var(&k3);
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_max_depth_limits_nesting() {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let ns_name = format!("depthns-{ts}");
    let prefix = format!("DEPTH_{ts}");

    Config::register_namespace(NamespaceRegistration {
        name: ns_name.clone(),
        env_prefix: Some(prefix.clone()),
        defaults: None,
        schema: None,
        env_style: EnvStyle::Nested,
        max_depth: 3,
        env_map: None,
    })
    .unwrap();

    let k1 = format!("{prefix}_A_B_C_D_E");
    std::env::set_var(&k1, "val");

    let tmp_dir = std::env::temp_dir().join(format!("apcore-depth-test-{ts}"));
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let yaml_path = tmp_dir.join("cfg.yaml");
    std::fs::write(&yaml_path, NS_YAML).unwrap();
    let config = Config::load(&yaml_path).unwrap();

    // max_depth=3: at most 3 segments (2 dots), rest literal.
    assert_eq!(
        config.get(&format!("{ns_name}.a.b.c_d_e")),
        Some(serde_json::json!("val"))
    );

    std::env::remove_var(&k1);
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_env_prefix_auto_derived_from_name() {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let ns_name = format!("autoderiv-{ts}");
    // No env_prefix → auto-derived as "AUTODERIV_{ts}" (uppercase, - → _)
    Config::register_namespace(NamespaceRegistration {
        name: ns_name.clone(),
        env_prefix: None, // auto-derive
        defaults: None,
        schema: None,
        env_style: EnvStyle::Auto,
        max_depth: DEFAULT_MAX_DEPTH,
        env_map: None,
    })
    .unwrap();

    let derived_prefix = ns_name.to_uppercase().replace('-', "_");
    let k1 = format!("{derived_prefix}_FOO");
    std::env::set_var(&k1, "bar");

    let tmp_dir = std::env::temp_dir().join(format!("apcore-autoderiv-{ts}"));
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let yaml_path = tmp_dir.join("cfg.yaml");
    std::fs::write(&yaml_path, NS_YAML).unwrap();
    let config = Config::load(&yaml_path).unwrap();

    assert_eq!(
        config.get(&format!("{ns_name}.foo")),
        Some(serde_json::json!("bar"))
    );

    std::env::remove_var(&k1);
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_namespace_env_map() {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let ns_name = format!("envmapns-{ts}");
    let redis_key = format!("REDIS_URL_{ts}");
    Config::register_namespace(NamespaceRegistration {
        name: ns_name.clone(),
        env_prefix: None,
        defaults: None,
        schema: None,
        env_style: EnvStyle::Auto,
        max_depth: DEFAULT_MAX_DEPTH,
        env_map: Some(HashMap::from([(
            redis_key.clone(),
            "cache_url".to_string(),
        )])),
    })
    .unwrap();

    std::env::set_var(&redis_key, "redis://localhost");

    let tmp_dir = std::env::temp_dir().join(format!("apcore-envmap-{ts}"));
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let yaml_path = tmp_dir.join("cfg.yaml");
    std::fs::write(&yaml_path, NS_YAML).unwrap();
    let config = Config::load(&yaml_path).unwrap();

    assert_eq!(
        config.get(&format!("{ns_name}.cache_url")),
        Some(serde_json::json!("redis://localhost"))
    );

    std::env::remove_var(&redis_key);
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

// ---------------------------------------------------------------------------
// A-D-007 — legacy mode consults the global env_map (parity with Python/TS)
// ---------------------------------------------------------------------------

#[test]
fn test_legacy_mode_applies_global_env_map() {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    // Unique env var name → unique top-level config key, to avoid cross-test
    // contamination of the global env_map / process environment.
    let env_var = format!("ADPORT_{ts}");
    let config_key = format!("adport_{ts}");

    Config::env_map(HashMap::from([(env_var.clone(), config_key.clone())])).unwrap();
    std::env::set_var(&env_var, "8080");

    // Legacy-mode YAML: NO `apcore:` mapping key, so detect_mode() => Legacy.
    let tmp_dir = std::env::temp_dir().join(format!("apcore-legacy-envmap-{ts}"));
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let yaml_path = tmp_dir.join("cfg.yaml");
    // Legacy YAML must satisfy validate()'s required fields (legacy-mode only).
    std::fs::write(
        &yaml_path,
        "version: '1.0.0'\n\
         project:\n  name: 'demo'\n\
         extensions:\n  root: './ext'\n\
         schema:\n  root: './schema'\n\
         acl:\n  root: './acl'\n  default_effect: 'deny'\n\
         executor:\n  max_call_depth: 32\n  max_module_repeat: 3\n",
    )
    .unwrap();
    let config = Config::load(&yaml_path).unwrap();

    assert_eq!(
        config.mode,
        ConfigMode::Legacy,
        "YAML without an apcore mapping must be legacy mode"
    );
    // A-D-007: the bare env var, mapped via the global env_map, must apply even
    // in legacy mode (was silently dropped before the fix).
    assert_eq!(
        config.get(&config_key),
        Some(serde_json::json!(8080)),
        "global env_map must be honored in legacy mode"
    );

    std::env::remove_var(&env_var);
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

// ---------------------------------------------------------------------------
// A-D-009 — namespace mode implicit-apcore fallback for bare keys
// ---------------------------------------------------------------------------

#[test]
fn test_namespace_mode_implicit_apcore_fallback() {
    // A config in namespace mode (apcore.* is a mapping) where a user key is
    // stored under `apcore.custom_key`. A bare `get("custom_key")` must resolve
    // it via the implicit-apcore fallback (§9.9.1), matching Python/TS.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let tmp_dir = std::env::temp_dir().join(format!("apcore-ns-fallback-{ts}"));
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let yaml_path = tmp_dir.join("cfg.yaml");
    std::fs::write(
        &yaml_path,
        "apcore:\n  version: '1.0.0'\n  custom_key: 42\n",
    )
    .unwrap();
    let config = Config::load(&yaml_path).unwrap();

    assert_eq!(config.mode, ConfigMode::Namespace);
    assert_eq!(
        config.get("custom_key"),
        Some(serde_json::json!(42)),
        "bare key must resolve via implicit-apcore fallback"
    );
    // The explicit path must still work too.
    assert_eq!(config.get("apcore.custom_key"), Some(serde_json::json!(42)));

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

// ---------------------------------------------------------------------------
// Sync CB-001 — validate() spec-required field constraints
// ---------------------------------------------------------------------------

#[test]
fn test_validate_rejects_invalid_acl_default_effect() {
    let mut cfg = Config::from_defaults();
    cfg.set("acl.default_effect", serde_json::json!("maybe"));
    let err = cfg
        .validate()
        .expect_err("invalid acl.default_effect must be rejected");
    assert_eq!(err.code, ErrorCode::ConfigInvalid);
    assert!(
        err.message.contains("acl.default_effect"),
        "error must mention acl.default_effect: {}",
        err.message
    );
}

#[test]
fn test_validate_accepts_allow_or_deny_for_default_effect() {
    let mut cfg = Config::from_defaults();
    with_required_fields(&mut cfg);
    cfg.set("acl.default_effect", serde_json::json!("allow"));
    cfg.validate().unwrap();
    cfg.set("acl.default_effect", serde_json::json!("deny"));
    cfg.validate().unwrap();
}

#[test]
fn test_validate_rejects_sampling_rate_out_of_range() {
    let mut cfg = Config::from_defaults();
    cfg.set(
        "observability.tracing.sampling_rate",
        serde_json::json!(1.5),
    );
    let err = cfg
        .validate()
        .expect_err("sampling_rate > 1.0 must be rejected");
    assert!(err.message.contains("sampling_rate"));
}

#[test]
fn test_validate_accepts_sampling_rate_in_unit_interval() {
    let mut cfg = Config::from_defaults();
    with_required_fields(&mut cfg);
    cfg.set(
        "observability.tracing.sampling_rate",
        serde_json::json!(0.0),
    );
    cfg.validate().unwrap();
    cfg.set(
        "observability.tracing.sampling_rate",
        serde_json::json!(0.5),
    );
    cfg.validate().unwrap();
    cfg.set(
        "observability.tracing.sampling_rate",
        serde_json::json!(1.0),
    );
    cfg.validate().unwrap();
}

// ---------------------------------------------------------------------------
// Sync CB-002 — mount() deep-merge
// ---------------------------------------------------------------------------

#[test]
fn test_mount_deep_merges_nested_objects() {
    let mut cfg = Config::from_defaults();
    cfg.user_namespaces
        .insert("db".to_string(), serde_json::json!({"port": 5432}));
    cfg.mount("db", MountSource::Dict(serde_json::json!({"host": "a"})))
        .unwrap();
    let db = cfg.namespace("db");
    assert_eq!(db["host"], "a");
    assert_eq!(
        db["port"], 5432,
        "peer key 'port' must be preserved by deep-merge"
    );
}

#[test]
fn test_mount_deep_merges_recursively_under_nested_keys() {
    let mut cfg = Config::from_defaults();
    cfg.user_namespaces.insert(
        "services".to_string(),
        serde_json::json!({
            "auth": { "host": "auth.local", "port": 8080 }
        }),
    );
    cfg.mount(
        "services",
        MountSource::Dict(serde_json::json!({
            "auth": { "tls": true }
        })),
    )
    .unwrap();
    let auth = cfg.namespace("services")["auth"].clone();
    assert_eq!(auth["host"], "auth.local", "peer key preserved at depth 2");
    assert_eq!(auth["port"], 8080);
    assert_eq!(auth["tls"], true);
}
