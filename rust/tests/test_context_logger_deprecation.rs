//! `Context::logger` is deprecated (apcore#121), removed at v2.0.
//!
//! The decision is D-67's boundary applied to an API surface rather than a
//! configuration key: apcore does not own the host's logging policy, and this
//! method's output is fixed at stderr / `info` / JSON — notably NOT going
//! through `tracing`, so a host that configured a subscriber never sees it.
//! `ObsLoggingMiddleware` is deliberately NOT the migration target; it emits
//! apcore's execution events, a different facility.
//!
//! Rust states the notice in the type system rather than at runtime, so these
//! cases read the attribute and the call sites rather than a log line.

/// The `#[deprecated]` attribute is the notice here, and it must say the three
/// things a caller needs: when it goes, where to go instead, and why the
/// obvious substitute is not it.
#[test]
fn the_deprecation_attribute_names_the_version_the_target_and_the_issue() {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/context.rs"),
    )
    .expect("context.rs is readable");
    let start = src
        .find("#[deprecated(")
        .expect("Context::logger must carry a #[deprecated] attribute");
    let attr = &src[start..start + 600.min(src.len() - start)];

    assert!(attr.contains("since = \"0.31.0\""), "attribute: {attr}");
    assert!(
        attr.contains("host application's logger"),
        "attribute: {attr}"
    );
    assert!(attr.contains("apcore#121"), "attribute: {attr}");
    assert!(attr.contains("Removed at v2.0"), "attribute: {attr}");
    // The one substitution a reader would otherwise reach for, ruled out where
    // they will read it rather than only in the changelog.
    assert!(
        attr.contains("ObsLoggingMiddleware"),
        "the note must say what ObsLoggingMiddleware is FOR, so a reader does \
         not treat it as a drop-in replacement: {attr}"
    );
}

/// The premise of deprecating rather than wiring: nothing inside the crate uses
/// it. Written as a test rather than left as a claim in the issue, because it
/// is what makes removal at v2.0 safe for the framework itself — a future
/// caller inside `src/` fails here and has to argue for itself.
#[test]
fn no_apcore_code_path_reaches_it() {
    fn walk(dir: &std::path::Path, hits: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("readable dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, hits);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                for (n, line) in text.lines().enumerate() {
                    let calls = line.contains("ctx.logger()") || line.contains("context.logger()");
                    // The definition itself, and prose about it, are not calls.
                    let is_prose = line.trim_start().starts_with("//");
                    if calls && !is_prose {
                        hits.push(format!("{}:{}", path.display(), n + 1));
                    }
                }
            }
        }
    }
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    walk(&src, &mut hits);
    assert!(
        hits.is_empty(),
        "apcore's own code now reaches Context::logger: {hits:?}"
    );
}

/// A deprecation is a notice, not a removal — §13.2's two-minor floor. The
/// method keeps returning a logger carrying this context's correlation fields
/// until v2.0, so a project that has not migrated yet still runs.
#[test]
#[allow(deprecated)]
fn it_still_works_through_the_1_x_line() {
    let identity = apcore::Identity::new(
        "api.probe".to_string(),
        "service".to_string(),
        Vec::new(),
        std::collections::HashMap::new(),
    );
    let ctx: apcore::Context<serde_json::Value> = apcore::Context::create(
        Some(identity),
        None,
        None,
        None,
        serde_json::Value::Null,
        None,
    );
    let logger = ctx.logger();
    // Asserted against the CONTEXT's own fields rather than literals: carrying
    // this context's correlation values is the whole contract of the accessor,
    // and a literal would pin how `Context::create` derives them instead —
    // which is a different question and one this case has no business fixing.
    assert_eq!(logger.trace_id.as_deref(), Some(ctx.trace_id.as_str()));
    assert_eq!(logger.caller_id, ctx.caller_id);
    assert_eq!(logger.module_id, ctx.call_chain.last().cloned());
}
