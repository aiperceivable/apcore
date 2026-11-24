//! A config section that no `Config::load` test reaches is the shape both
//! apcore-rust#33 and the `executor` gap had. This guard fails when a new one
//! appears (apcore-rust#34).
//!
//! ## Why a guard rather than more tests
//!
//! The individual load-path suites
//! (`test_config_load_executor_namespace.rs`, `test_config_load_observability_subkeys.rs`,
//! `test_config_load_framework_sections.rs`, `test_config_load_sections.rs`)
//! pin the sections that exist **today**. They
//! cannot notice a section added tomorrow. Both defects this issue is about
//! were found by reasoning about the mechanism, not by a failing test, and both
//! had been shipping — precisely because nothing failed when the untested
//! section was introduced.
//!
//! So this test asks the inverse question: **for every config section
//! `src/config.rs` declares, is there a `Config::load` test that both writes it
//! into a real file and asserts on the value that comes back?** A new typed
//! field, a new framework section in `apcore-config.schema.json`, a new
//! default-table section, a new built-in namespace or a new required field all
//! fail here until such a test exists.
//!
//! The section list has to be assembled from **every** place `src/config.rs`
//! names one, not the convenient ones. This guard originally read five of the
//! six and skipped `FRAMEWORK_CONFIG_KEYS` — the longest list, and the only
//! one that is a projection of the canonical schema. Seven sections
//! (`bindings`, `id_map`, `logging`, `middleware`, `obs`, `pipeline`,
//! `validation`) appear *only* there, so they had no `Config::load` coverage
//! and the guard reported none missing. A guard over a partial list is the
//! defect it was built to catch, wearing the costume of a test.
//!
//! ## How it works, and what it cannot do
//!
//! Rust has no runtime reflection over struct fields, so the section list is
//! recovered by parsing `src/config.rs` — pulled in with `include_str!`, which
//! is resolved at COMPILE time relative to this file, so the test does not
//! depend on the working directory or on the source tree being present at run
//! time. The same trick pulls in the load-path suites as the corpus to search.
//!
//! This is a **coverage** guard, not a correctness one: it proves a section
//! appears in a fixture and in an assertion, not that the assertion is a good
//! one. That is deliberate — the shape it detects ("nothing loads this from a
//! file at all") is exactly the shape that let #33 ship, and it is cheap and
//! stable to detect. The per-section suites carry the real assertions.
//!
//! Every extractor below is checked for a non-vacuous result before it is used.
//! Without that, a rename in `src/config.rs` that broke the parsing would leave
//! this test asserting over an empty list — green, and worthless.

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

/// The module whose sections are under audit.
const CONFIG_RS: &str = include_str!("../src/config.rs");

/// The load-path suites. A file belongs here only if **every** `Config` in it
/// is built by `Config::load` / `from_yaml_file` / `from_json_file` from a real
/// file — that is what makes "the name appears in this corpus" mean "a load
/// test reaches it".
const LOAD_SUITES: &[(&str, &str)] = &[
    (
        "test_config_load_executor_namespace.rs",
        include_str!("test_config_load_executor_namespace.rs"),
    ),
    (
        "test_config_load_observability_subkeys.rs",
        include_str!("test_config_load_observability_subkeys.rs"),
    ),
    (
        "test_config_load_framework_sections.rs",
        include_str!("test_config_load_framework_sections.rs"),
    ),
    (
        "test_config_load_sections.rs",
        include_str!("test_config_load_sections.rs"),
    ),
];

// ---------------------------------------------------------------------------
// Extractors — recover the section list from src/config.rs
// ---------------------------------------------------------------------------

/// The body of the first `<kind> <name> ... { ... }` item, by brace matching.
fn item_body(src: &str, header: &str) -> String {
    let start = src
        .find(header)
        .unwrap_or_else(|| panic!("`{header}` no longer appears in src/config.rs"));
    let open = src[start..]
        .find('{')
        .unwrap_or_else(|| panic!("`{header}` has no body"))
        + start;
    let mut depth = 0usize;
    for (offset, ch) in src[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return src[open + 1..open + offset].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("`{header}` body is unbalanced");
}

/// The body of a `const NAME: ... = ...;` item, up to the terminating `;`.
fn const_body(src: &str, header: &str) -> String {
    let start = src
        .find(header)
        .unwrap_or_else(|| panic!("`{header}` no longer appears in src/config.rs"));
    let end = src[start..]
        .find("];")
        .unwrap_or_else(|| panic!("`{header}` is not a slice literal"))
        + start;
    src[start..end].to_string()
}

/// Every `"..."` string literal in `body`.
fn string_literals(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes: Vec<char> = body.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '"' {
            let mut j = i + 1;
            let mut lit = String::new();
            while j < bytes.len() && bytes[j] != '"' {
                if bytes[j] == '\\' {
                    j += 1;
                }
                if j < bytes.len() {
                    lit.push(bytes[j]);
                }
                j += 1;
            }
            out.push(lit);
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// Every framework section `schemas/apcore-config.schema.json` declares, as
/// projected into `FRAMEWORK_CONFIG_KEYS`.
///
/// This is the longest of the section lists in `src/config.rs` and the one the
/// guard originally did not read — which is precisely why `bindings`, `id_map`,
/// `logging`, `middleware`, `obs`, `pipeline` and `validation` sat with no
/// `Config::load` coverage while the guard stayed green (apcore-rust#34).
/// Every name here is a top-level key an operator can write in `apcore.yaml`
/// and the canonical schema will accept, so every one needs load-path coverage.
///
/// The constant holds full dot-paths since A-D-020 made the strict-mode check
/// recursive, so a section is the head of a path. It used to hold
/// `("section", &["key", …])` tuples, which needed a positional parser to tell
/// the section half from the key half.
fn framework_schema_sections() -> Vec<String> {
    let body = const_body(CONFIG_RS, "pub const FRAMEWORK_CONFIG_KEYS:");
    let mut out: Vec<String> = string_literals(&body)
        .iter()
        .map(|path| path.split('.').next().unwrap_or(path).to_string())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The typed fields `Config::deserialize` models, i.e. the fields of
/// `ConfigHelper` — the helper struct that mirrors the wire form. This is the
/// authoritative list: a key it names is consumed by a typed field and sits
/// OUTSIDE the `#[serde(flatten)]` bag, which is exactly where #33's data loss
/// happened.
///
/// `user_namespaces` is excluded: it IS the flatten bag, not a section.
fn typed_config_fields() -> Vec<String> {
    let body = item_body(CONFIG_RS, "struct ConfigHelper");
    body.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.starts_with('#') || line.starts_with("//") {
                return None;
            }
            let name = line.split(':').next()?.trim();
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                return None;
            }
            Some(name.to_string())
        })
        .filter(|name| name != "user_namespaces")
        .collect()
}

/// The top-level section of every key in the canonical default table.
///
/// Only the KEY half of each `("key", DefaultValue::…)` entry counts: the value
/// half holds strings like `"deny"` and `"yaml_first"`, which are settings, not
/// sections.
fn default_table_sections() -> Vec<String> {
    let body = const_body(CONFIG_RS, "const CONFIG_DEFAULTS:");
    let mut out: Vec<String> = body
        .split("(\"")
        .skip(1)
        .filter_map(|chunk| {
            let (key, rest) = chunk.split_once('"')?;
            rest.trim_start()
                .starts_with(", DefaultValue::")
                .then(|| key.split('.').next().unwrap_or_default().to_string())
        })
        .filter(|s| !s.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The top-level section of every key carrying a `validate_key_constraint`
/// entry. A constrained key is one an operator is expected to set, so its
/// section needs load-path coverage like any other.
fn constrained_key_sections() -> Vec<String> {
    let body = const_body(CONFIG_RS, "pub const CONSTRAINED_CONFIG_KEYS:");
    let mut out: Vec<String> = string_literals(&body)
        .into_iter()
        .map(|key| key.split('.').next().unwrap_or_default().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The names of the namespaces `init_builtin_namespaces` registers. These are
/// the only sections with a §9.15 default layer under `Config::namespace`, and
/// therefore the only ones that can fail the #33 way — by reporting a
/// registered default in place of the operator's value.
fn builtin_namespace_names() -> Vec<String> {
    let body = item_body(CONFIG_RS, "fn init_builtin_namespaces()");
    let mut out = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("name: \"") {
            if let Some(name) = rest.split('"').next() {
                out.push(name.to_string());
            }
        }
    }
    out
}

/// The top-level section of every §9.1 required field.
fn required_field_sections() -> Vec<String> {
    let body = const_body(CONFIG_RS, "const REQUIRED_FIELDS:");
    string_literals(&body)
        .into_iter()
        .map(|key| key.split('.').next().unwrap_or_default().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// The framework-reserved top-level names. Both are file-declarable — `apcore:`
/// selects namespace mode and `_config:` carries the strict-mode switch — so
/// both need load-path coverage like any other section.
fn reserved_namespace_names() -> Vec<String> {
    let body = const_body(CONFIG_RS, "pub const RESERVED_NAMESPACES:");
    string_literals(&body)
}

// ---------------------------------------------------------------------------
// Corpus probes
// ---------------------------------------------------------------------------

/// Does some load suite DECLARE `section` at the TOP LEVEL of a config fixture?
///
/// A config section is a top-level key, so only a top-level occurrence counts.
/// The check is deliberately position-aware rather than a substring search,
/// because several section names are also *subkey* names elsewhere in the
/// canonical tree — `logging` is both the root `logging:` section and the
/// `observability.logging.*` family. A naive `contains("\n  logging:")` reads
/// the nested family in `test_config_load_observability_subkeys.rs` as coverage
/// for the root section and reports a section covered that no test loads. A
/// guard that passes for the wrong reason is worse than no guard, and that
/// false pass is the same class of mistake as the defects being guarded
/// against, so the scan tracks indentation instead.
///
/// Two positions count as top level:
///   * column 0 — the legacy-mode shape, and the namespace-mode shape used by
///     these suites (sections sit beside the `apcore:` block, not inside it);
///   * two-space indent when the nearest preceding column-0 line is `apcore:`
///     — the §9.6 shape where the whole framework tree nests under `apcore:`.
///
/// Both raw-string fixtures (a real newline before the key) and ordinary
/// strings (a literal `\` `n` escape) are covered by normalising the escape
/// before the scan.
fn declared_in_a_fixture(section: &str) -> Option<&'static str> {
    fn declares(src: &str, section: &str) -> bool {
        let normalised = src.replace("\\n", "\n");
        let mut under_apcore = false;
        for line in normalised.lines() {
            let indent = line.len() - line.trim_start().len();
            let trimmed = line.trim_start();
            if indent == 0 {
                // A column-0 line closes any `apcore:` block that was open.
                under_apcore = trimmed.starts_with("apcore:");
                if trimmed.starts_with(&format!("{section}:")) {
                    return true;
                }
            } else if indent == 2 && under_apcore && trimmed.starts_with(&format!("{section}:")) {
                return true;
            }
        }
        false
    }

    LOAD_SUITES
        .iter()
        .find(|(_, src)| declares(src, section))
        .map(|(name, _)| *name)
}

/// Does some load suite ASSERT on `section` through a `Config` reader?
///
/// The markers match how the section name is spelled in *code* rather than in a
/// fixture: a dotted key literal (`"stream.max_merge_depth"`, whether written
/// inline or held in an expectation table), the sole argument of a reader call
/// (`namespace("acl")`), a `data()` index, or a typed-field access. A YAML
/// fixture spells the same name `stream:`, which none of these match, so a
/// section that is only ever declared and never read still fails.
fn asserted_in_a_suite(section: &str) -> Option<&'static str> {
    let markers = [
        format!("\"{section}."),
        format!("\"{section}\")"),
        format!("[\"{section}\"]"),
        format!("config.{section}"),
    ];
    LOAD_SUITES
        .iter()
        .find(|(_, src)| markers.iter().any(|m| src.contains(m.as_str())))
        .map(|(name, _)| *name)
}

// ---------------------------------------------------------------------------
// The guard
// ---------------------------------------------------------------------------

/// Sanity: every extractor must find something.
///
/// A rename in `src/config.rs` that broke the parsing would otherwise leave
/// [`every_config_section_has_load_path_coverage`] iterating over an empty list
/// — green, and asserting nothing at all. That is the same failure mode this
/// whole issue is about, so it gets its own test rather than a comment.
#[test]
fn the_section_extractors_are_not_vacuous() {
    let typed = typed_config_fields();
    assert!(
        typed.len() >= 3,
        "expected at least modules_path/executor/observability from ConfigHelper, got {typed:?}"
    );
    assert!(
        typed.contains(&"executor".to_string()) && typed.contains(&"observability".to_string()),
        "the two typed fields that broke must still be recovered, got {typed:?}"
    );

    let framework = framework_schema_sections();
    assert!(
        framework.len() >= 10,
        "FRAMEWORK_CONFIG_KEYS should yield every section of \
         apcore-config.schema.json, got {framework:?}"
    );
    for expected in ["acl", "bindings", "id_map", "logging", "obs", "validation"] {
        assert!(
            framework.contains(&expected.to_string()),
            "`{expected}` must be recovered from FRAMEWORK_CONFIG_KEYS, got {framework:?}"
        );
    }
    assert!(
        !framework.contains(&"strict".to_string()) && !framework.contains(&"redaction".to_string()),
        "`strict` and `redaction` are declared KEYS, not sections — the \
         dot-path head split broke: {framework:?}"
    );

    let defaults = default_table_sections();
    assert!(
        defaults.len() >= 5,
        "CONFIG_DEFAULTS should yield several sections, got {defaults:?}"
    );
    assert!(
        defaults.contains(&"acl".to_string()) && defaults.contains(&"stream".to_string()),
        "the key half of the default table must be recovered, got {defaults:?}"
    );
    assert!(
        !defaults.contains(&"deny".to_string()),
        "`deny` is a default VALUE, not a section — the key/value split broke: {defaults:?}"
    );

    let constrained = constrained_key_sections();
    assert!(
        constrained.contains(&"executor".to_string()),
        "CONSTRAINED_CONFIG_KEYS should yield the executor section, got {constrained:?}"
    );

    let builtins = builtin_namespace_names();
    assert_eq!(
        builtins,
        vec![
            "observability".to_string(),
            "sys_modules".to_string(),
            "obs".to_string(),
        ],
        "the built-in namespace list changed; if that is intended, this \
         expectation moves with it"
    );

    assert_eq!(required_field_sections(), vec!["version", "project"]);
    assert_eq!(reserved_namespace_names(), vec!["apcore", "_config"]);
}

/// Every config section `src/config.rs` declares must be written into a real
/// file by some `Config::load` test, and asserted on there.
///
/// A failure means a section can be declared by an operator that no test loads
/// from disk — the state `observability` was in the day before apcore-rust#33
/// was found, and `executor` the day before its own gap was.
#[test]
fn every_config_section_has_load_path_coverage() {
    let mut sections: Vec<(String, &'static str)> = Vec::new();
    for name in typed_config_fields() {
        sections.push((
            name,
            "typed field on ConfigHelper (outside the flatten bag)",
        ));
    }
    for name in framework_schema_sections() {
        sections.push((
            name,
            "framework section declared by apcore-config.schema.json",
        ));
    }
    for name in default_table_sections() {
        sections.push((name, "section of the CONFIG_DEFAULTS table"));
    }
    for name in builtin_namespace_names() {
        sections.push((
            name,
            "built-in namespace registered by init_builtin_namespaces",
        ));
    }
    for name in constrained_key_sections() {
        sections.push((name, "section of a validate_key_constraint key"));
    }
    for name in required_field_sections() {
        sections.push((name, "section of a §9.1 required field"));
    }
    for name in reserved_namespace_names() {
        sections.push((name, "framework-reserved top-level name"));
    }
    sections.sort();
    sections.dedup_by(|a, b| a.0 == b.0);

    let mut missing: Vec<String> = Vec::new();
    for (section, origin) in &sections {
        if declared_in_a_fixture(section).is_none() {
            missing.push(format!(
                "`{section}` ({origin}) is never written into a config FILE by any \
                 load-path suite — add it to a fixture in one of {:?}",
                LOAD_SUITES.iter().map(|(n, _)| *n).collect::<Vec<_>>()
            ));
            continue;
        }
        if asserted_in_a_suite(section).is_none() {
            missing.push(format!(
                "`{section}` ({origin}) appears in a load-path fixture but nothing \
                 asserts what comes back — add a `get(\"{section}…\")` / \
                 `namespace(\"{section}\")` assertion"
            ));
        }
    }

    assert!(
        missing.is_empty(),
        "{} config section(s) have no Config::load coverage. This is the shape \
         both apcore-rust#33 and the `executor` gap had: a section an operator \
         can declare in a file, verified only through `Config::from_defaults()` \
         + `.set(…)`, which writes a `user_namespaces` entry no YAML file can \
         produce.\n\n  {}\n",
        missing.len(),
        missing.join("\n  ")
    );
}
