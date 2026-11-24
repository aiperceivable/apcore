// APCore Protocol — Schema reference resolver
// Spec reference: JSON $ref resolution and circular reference detection

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::errors::{ErrorCode, ModuleError, SchemaCircularRefError};

/// Scheme prefix for canonical cross-file references
/// (`apcore://common.types.error/ErrorDetail`), PROTOCOL_SPEC §4.11.
const CANONICAL_SCHEME: &str = "apcore://";

/// Filename suffixes tried when resolving a canonical or relative reference
/// that does not name an extension, in priority order.
const SCHEMA_FILE_SUFFIXES: &[&str] = &[".schema.yaml", ".schema.yml", ".schema.json"];

/// Default maximum depth for `$ref` resolution. Matches apcore-python
/// (`schema.max_ref_depth = 32`) and apcore-typescript.
pub const DEFAULT_MAX_REF_DEPTH: usize = 32;

/// The `$ref` strings that denote *document* itself.
///
/// Seeding the visited set with them makes a self-reference lazy from the very
/// first encounter, so a recursive schema is never inlined even once
/// (PROTOCOL_SPEC §4.15.2). The root `$id` is included because JSON Schema lets a
/// document reference itself by identifier — `{"$id": "TreeNode", … "$ref": "TreeNode"}`.
fn root_ref_aliases(document: &serde_json::Value) -> HashSet<String> {
    let mut aliases: HashSet<String> = ["#", "#/"].iter().map(|s| (*s).to_string()).collect();
    if let Some(id) = document.get("$id").and_then(serde_json::Value::as_str) {
        if !id.is_empty() {
            aliases.insert(id.to_string());
        }
    }
    aliases
}

/// The document a node is being resolved in.
///
/// Root, file and D-104 fallback travel together because all three are
/// properties of ONE document, and `$ref` resolution rebases all three on every
/// cross-document hop. They used to be two parameters plus a resolver field,
/// and the field is the one that did not rebase: `node_fallback` was consulted
/// for a local pointer no matter which document resolution had reached, so an
/// external schema's unresolvable `#/$defs/X` fell back to the *calling*
/// interface schema's `$defs` and silently resolved to an unrelated definition
/// where it owes the caller `SCHEMA_NOT_FOUND`. That changes what gets
/// validated, and — since §10.6 reads `x-sensitive` off the resolved schema —
/// what gets redacted.
#[derive(Clone, Copy)]
struct DocumentBase<'a> {
    /// The root a local `#/…` pointer resolves against first.
    root: &'a serde_json::Value,
    /// The file `root` was loaded from, when it came from one. Relative
    /// references resolve against its directory.
    file: Option<&'a Path>,
    /// Secondary base for a local `#/…` pointer that does not resolve at
    /// `root` (PROTOCOL_SPEC §4.11 step 4a, D-104).
    ///
    /// `Some` only while resolution is still inside the document
    /// [`RefResolver::with_node_fallback`] described. Crossing into another
    /// document clears it permanently for that sub-tree: a nested in-document
    /// hop taken *within* the external document must not bring it back, or the
    /// external document would inherit a fallback belonging to a schema it has
    /// never heard of.
    node_fallback: Option<&'a serde_json::Value>,
}

/// Resolves $ref references in JSON schemas.
///
/// Supports the three reference formats PROTOCOL_SPEC §4.11 mandates:
///
/// | Form | Example |
/// |---|---|
/// | Local (same document) | `#/definitions/ErrorDetail` |
/// | Relative cross-file | `./common/error.schema.yaml#/definitions/ErrorDetail` |
/// | Canonical cross-file | `apcore://common.types.error/ErrorDetail` |
///
/// The two cross-file forms require a schemas root — set it with
/// [`Self::with_schemas_dir`]. Without one they are rejected with
/// `SCHEMA_NOT_FOUND` rather than silently resolving against an arbitrary
/// directory. Every resolved file path is checked for containment under that
/// root, so a `../../etc/passwd`-style reference cannot escape it.
///
/// Schemas registered in-memory via [`Self::register`] are still matched by
/// exact URI string first, ahead of any filesystem lookup.
#[derive(Debug)]
pub struct RefResolver {
    schemas: HashMap<String, serde_json::Value>,
    max_depth: usize,
    schemas_dir: Option<PathBuf>,
    current_file: Option<PathBuf>,
    /// Secondary base for a local `#/…` pointer that does not resolve at the
    /// file root (PROTOCOL_SPEC §4.11 step 4a, D-104). Set by the loader to the
    /// `input_schema` / `output_schema` node currently being resolved.
    node_fallback: Option<serde_json::Value>,
}

/// A `$ref` target together with the document it came from.
///
/// The document is carried so a `#/…` pointer *inside* an external schema is
/// looked up in that schema's own tree rather than in the calling document's
/// (JSON Schema 2020-12 §8.2 base-URI resolution). apcore-python and
/// apcore-typescript compute the same per-hop base (`effective_file`).
struct RefTarget {
    value: serde_json::Value,
    /// Root of the document `value` was found in.
    root: serde_json::Value,
    /// True only for an in-document `#/…` pointer, where `root` is the same
    /// document the reference was written in. False for every cross-document
    /// form (a registered URI, a canonical `apcore://` reference, a relative
    /// file path) — which is what decides whether D-104's `node_fallback`
    /// survives the hop. See [`DocumentBase::node_fallback`].
    same_document: bool,
    /// Path of that document, when it came from disk.
    file: Option<PathBuf>,
    /// Stable identity for cycle detection: the raw ref for in-document
    /// lookups, `<abs-file>#<pointer>` for cross-file ones (so the same
    /// relative string used in two different files does not collide).
    seen_key: String,
}

impl RefResolver {
    /// Create a new ref resolver with the default max depth.
    #[must_use]
    pub fn new() -> Self {
        Self {
            schemas: HashMap::new(),
            max_depth: DEFAULT_MAX_REF_DEPTH,
            schemas_dir: None,
            current_file: None,
            node_fallback: None,
        }
    }

    /// Create a ref resolver with an explicit `max_depth` for `$ref` recursion.
    #[must_use]
    pub fn with_max_depth(max_depth: usize) -> Self {
        Self {
            max_depth,
            ..Self::new()
        }
    }

    /// Anchor cross-file `$ref` resolution at `dir` (the `schema.root` config
    /// key). Required for the `apcore://` and relative cross-file forms.
    #[must_use]
    pub fn with_schemas_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.schemas_dir = Some(dir.into());
        self
    }

    /// Record the file the document being resolved was loaded from, so
    /// relative references resolve against its directory rather than the
    /// schemas root.
    #[must_use]
    pub fn with_current_file(mut self, file: impl Into<PathBuf>) -> Self {
        self.current_file = Some(file.into());
        self
    }

    /// Resolve a local `#/…` pointer against `node` when it does not resolve at
    /// the document root (PROTOCOL_SPEC §4.11 step 4a, D-104).
    ///
    /// Both layouts are normative: `definitions:` beside `input_schema` as a
    /// top-level key of the schema FILE (what §4.11's own example writes), and
    /// a `$defs` block nested INSIDE `input_schema` (what apcore-python and
    /// apcore-typescript accept). File root is tried first; the two lookups
    /// cannot collide, because a pointer either resolves at the file root or it
    /// does not.
    /// The fallback is scoped to THAT document: once resolution follows a
    /// reference into an external file or a registered schema, it is no longer
    /// consulted. Otherwise an external document's unresolvable `#/$defs/X`
    /// would quietly resolve against this node's `$defs`.
    #[must_use]
    pub fn with_node_fallback(mut self, node: serde_json::Value) -> Self {
        self.node_fallback = Some(node);
        self
    }

    /// The starting document context for a public entry point: the document
    /// itself, the file it came from, and the D-104 fallback (which is valid
    /// only here, at the origin).
    fn document_base<'a>(
        &'a self,
        document: &'a serde_json::Value,
        file: Option<&'a Path>,
    ) -> DocumentBase<'a> {
        DocumentBase {
            root: document,
            file,
            node_fallback: self.node_fallback.as_ref(),
        }
    }

    /// Resolve every `$ref` in `node`, treating `document` as the root the
    /// local `#/…` pointers are looked up in.
    ///
    /// [`Self::resolve`] is the case where the two coincide.
    pub fn resolve_in_document(
        &self,
        node: &serde_json::Value,
        document: &serde_json::Value,
    ) -> Result<serde_json::Value, ModuleError> {
        let mut seen: HashSet<String> = root_ref_aliases(document);
        let base = self.current_file.clone();
        self.resolve_inner(
            node,
            self.document_base(document, base.as_deref()),
            &mut seen,
            0,
            false,
        )
    }

    /// Returns the configured maximum recursion depth for `$ref` resolution.
    #[must_use]
    pub fn max_depth(&self) -> usize {
        self.max_depth
    }

    /// Returns the configured schemas root, if any.
    #[must_use]
    pub fn schemas_dir(&self) -> Option<&Path> {
        self.schemas_dir.as_deref()
    }

    /// Register a schema that can be referenced.
    pub fn register(&mut self, uri: &str, schema: serde_json::Value) {
        self.schemas.insert(uri.to_string(), schema);
    }

    /// Resolve all $ref references in a schema, returning a dereferenced schema.
    ///
    /// A *self-reference* — a `$ref` that re-enters a schema location reached by
    /// descending through `properties` / `items` / a combinator — is preserved
    /// verbatim as a lazy reference rather than inlined, so recursive data
    /// structures such as `TreeNode` resolve without looping (PROTOCOL_SPEC
    /// §4.15.2). A *circular reference* — a `$ref` → `$ref` chain that never
    /// reaches a schema body — still fails with `SCHEMA_CIRCULAR_REF`.
    pub fn resolve(&self, schema: &serde_json::Value) -> Result<serde_json::Value, ModuleError> {
        let mut seen: HashSet<String> = root_ref_aliases(schema);
        let base = self.current_file.clone();
        self.resolve_inner(
            schema,
            self.document_base(schema, base.as_deref()),
            &mut seen,
            0,
            false,
        )
    }

    /// Check if a schema contains a *circular* reference — a `$ref` → `$ref`
    /// chain that never reaches a schema body (PROTOCOL_SPEC §4.15).
    ///
    /// A *self-reference* reached by structural descent (`properties`,
    /// `items`, a combinator branch) is **not** circular: the spec mandates
    /// support for recursive contracts such as `TreeNode`, and [`Self::resolve`]
    /// preserves them as lazy `$ref` nodes.
    ///
    /// Implemented in terms of [`Self::resolve`] so a single code path defines
    /// which re-entry is which. Previously this walked its own traversal that
    /// seeded an empty visited set and carried no `from_ref_chain`
    /// discriminator, so it answered `true` for every recursive schema
    /// `resolve` accepted — a caller gating registration on the predicate
    /// rejected exactly the contracts §4.15 requires.
    ///
    /// An unresolvable `$ref` (`SCHEMA_NOT_FOUND`) and depth-cap exhaustion
    /// (`SCHEMA_MAX_DEPTH_EXCEEDED`) are distinct conditions and both answer
    /// `false` here; use [`Self::resolve`] directly to surface them.
    #[must_use]
    pub fn has_circular_refs(&self, schema: &serde_json::Value) -> bool {
        matches!(
            self.resolve(schema),
            Err(e) if e.code == ErrorCode::SchemaCircularRef
        )
    }

    /// Recursively resolve `$ref` nodes.
    ///
    /// `from_ref_chain` marks `node` as the immediate target of another `$ref`.
    /// Re-entering a reference along such a chain is a genuine cycle: resolution
    /// never reaches a schema body and cannot terminate. Re-entering one after a
    /// structural descent is a self-reference and is deferred instead.
    ///
    /// `base` is the document `node` lives in, rebased on every cross-document
    /// hop so relative references, `#/…` pointers and the D-104 fallback all
    /// resolve against the right document.
    fn resolve_inner(
        &self,
        node: &serde_json::Value,
        base: DocumentBase<'_>,
        seen: &mut HashSet<String>,
        depth: usize,
        from_ref_chain: bool,
    ) -> Result<serde_json::Value, ModuleError> {
        if depth >= self.max_depth {
            // A-D-038: depth-cap exhaustion is distinct from an actual cycle.
            // Emit SCHEMA_MAX_DEPTH_EXCEEDED here; the genuine-cycle branch
            // below (seen.contains) emits SCHEMA_CIRCULAR_REF. Cross-SDK note:
            // all three SDKs are aligned — apcore-python and apcore-typescript
            // also raise a distinct max-depth error (SchemaMaxDepthExceededError)
            // on the depth cap, separate from the circular-ref error.
            let mut details = std::collections::HashMap::new();
            details.insert(
                "max_depth".to_string(),
                serde_json::Value::from(self.max_depth),
            );
            return Err(ModuleError::new(
                ErrorCode::SchemaMaxDepthExceeded,
                format!(
                    "Schema $ref recursion exceeded max_depth={} (sync SCHEMA-001)",
                    self.max_depth
                ),
            )
            .with_details(details));
        }
        match node {
            serde_json::Value::Object(map) => {
                // If this node is a $ref, resolve it
                if let Some(ref_val) = map.get("$ref") {
                    if let Some(ref_str) = ref_val.as_str() {
                        if seen.contains(ref_str) {
                            if from_ref_chain {
                                return Err(SchemaCircularRefError::new(
                                    format!("Circular $ref detected: {ref_str}"),
                                    ref_str.to_string(),
                                )
                                .to_module_error());
                            }
                            // Self-reference: leave the `$ref` for the validator
                            // to bind lazily instead of inlining it forever.
                            return Ok(node.clone());
                        }
                        let target = self.lookup_ref(ref_str, base)?;
                        if seen.contains(&target.seen_key) {
                            if from_ref_chain {
                                return Err(SchemaCircularRefError::new(
                                    format!("Circular $ref detected: {ref_str}"),
                                    ref_str.to_string(),
                                )
                                .to_module_error());
                            }
                            return Ok(node.clone());
                        }
                        seen.insert(ref_str.to_string());
                        seen.insert(target.seen_key.clone());

                        // Sync finding A-D-028: increment `depth` ONLY when
                        // following a $ref (this is the recursion the spec's
                        // max_depth=32 cap targets). Apcore-python and
                        // apcore-typescript also bump depth only on $ref
                        // dereferencing — Rust previously incremented on every
                        // child object/array element, so a flat 33-property
                        // schema with no $refs threw SCHEMA_MAX_DEPTH_EXCEEDED.
                        //
                        // `target.root` / `target.file` rebase the recursion on
                        // the resolved document (JSON Schema 2020-12 §8.2), so a
                        // `#/…` pointer *inside* an external schema is looked up
                        // there rather than in the calling document's tree.
                        // apcore-python and apcore-typescript compute the same
                        // per-hop base (`effective_file`).
                        let target_base = DocumentBase {
                            root: &target.root,
                            file: target.file.as_deref(),
                            // D-104's fallback belongs to the document
                            // `resolve` was entered with; it does not follow
                            // the reference into another one.
                            node_fallback: if target.same_document {
                                base.node_fallback
                            } else {
                                None
                            },
                        };
                        let mut result =
                            self.resolve_inner(&target.value, target_base, seen, depth + 1, true)?;
                        seen.remove(ref_str);
                        seen.remove(&target.seen_key);

                        // Overlay the node's NON-`$ref` keys onto the resolved
                        // target. JSON Schema 2019-09+ makes keys sitting
                        // beside a `$ref` independent assertions, and
                        // PROTOCOL_SPEC §4.11 step 1b already requires them
                        // preserved on the self-reference path (the two
                        // `Ok(node.clone())` returns above) — dropping them
                        // here made resolution the only place they vanished.
                        //
                        // This is a data leak, not a cosmetic gap: §10.6
                        // redaction reads `x-sensitive` off the RESOLVED input
                        // schema, so `{"$ref": …, "x-sensitive": true}` was
                        // redacted by apcore-python and apcore-typescript and
                        // logged in plaintext here.
                        //
                        // Siblings WIN on a key collision, matching
                        // apcore-python (`result.update(sibling_keys)`) and
                        // apcore-typescript (`Object.assign`) — the referring
                        // node is the more specific statement. They are
                        // resolved in turn, since a sibling may carry a `$ref`
                        // of its own; against the REFERRING document's base
                        // (`root` / `base_file`), which is where they
                        // textually live, not the target's.
                        //
                        // A non-object target (a boolean schema) has nothing to
                        // overlay onto and is returned unchanged, as in both
                        // peers.
                        if map.len() > 1 {
                            if let Some(obj) = result.as_object_mut() {
                                for (key, value) in map {
                                    if key == "$ref" {
                                        continue;
                                    }
                                    obj.insert(
                                        key.clone(),
                                        self.resolve_inner(value, base, seen, depth, false)?,
                                    );
                                }
                            }
                        }
                        return Ok(result);
                    }
                }

                // Otherwise walk all children — same `depth`. Tree traversal
                // through map/array children does not consume the $ref budget.
                let mut new_map = serde_json::Map::new();
                for (k, v) in map {
                    new_map.insert(k.clone(), self.resolve_inner(v, base, seen, depth, false)?);
                }
                Ok(serde_json::Value::Object(new_map))
            }
            serde_json::Value::Array(arr) => {
                let resolved: Result<Vec<_>, _> = arr
                    .iter()
                    .map(|v| self.resolve_inner(v, base, seen, depth, false))
                    .collect();
                Ok(serde_json::Value::Array(resolved?))
            }
            other => Ok(other.clone()),
        }
    }

    /// Resolve a `$ref` string to its target node plus the document that node
    /// belongs to (PROTOCOL_SPEC §4.11 step 4).
    ///
    /// Resolution order:
    /// 1. `#…` — pointer into the *current* document.
    /// 2. An exact match among schemas registered via [`Self::register`].
    /// 3. `apcore://<dotted.module.id>[/<TargetName>]` — canonical cross-file.
    /// 4. Anything else — path relative to `base_file`'s directory, with an
    ///    optional `#<pointer>` fragment.
    ///
    /// Formats 3 and 4 were previously absent entirely: the fragment was never
    /// split off, so even registering the bare file path could not match, and
    /// there was no schemas-directory containment check.
    fn lookup_ref(&self, ref_str: &str, base: DocumentBase<'_>) -> Result<RefTarget, ModuleError> {
        // 1. In-document pointer. Resolved against the FILE ROOT first and,
        //    when it does not resolve there, against the schema node being
        //    resolved (D-104 — both layouts are normative).
        if let Some(pointer) = ref_str.strip_prefix('#') {
            let value = if pointer.is_empty() {
                base.root.clone()
            } else {
                base.root
                    .pointer(pointer)
                    .cloned()
                    .or_else(|| {
                        base.node_fallback
                            .and_then(|node| node.pointer(pointer).cloned())
                    })
                    .ok_or_else(|| {
                        ModuleError::new(
                            ErrorCode::SchemaNotFound,
                            format!("Local $ref not found: {ref_str}"),
                        )
                    })?
            };
            return Ok(RefTarget {
                value,
                root: base.root.clone(),
                file: base.file.map(Path::to_path_buf),
                seen_key: Self::seen_key_for(base.file, ref_str),
                same_document: true,
            });
        }

        // 2. Registered in-memory URI (exact string match).
        if let Some(schema) = self.schemas.get(ref_str) {
            return Ok(RefTarget {
                value: schema.clone(),
                root: schema.clone(),
                file: None,
                seen_key: format!("registered:{ref_str}"),
                same_document: false,
            });
        }

        // 3 / 4. Cross-file forms.
        let is_canonical = ref_str.starts_with(CANONICAL_SCHEME);
        let (file_part, pointer) = if let Some(rest) = ref_str.strip_prefix(CANONICAL_SCHEME) {
            Self::split_canonical(rest)
        } else {
            match ref_str.split_once('#') {
                Some((p, f)) => (p.to_string(), format!("#{f}")),
                None => (ref_str.to_string(), String::new()),
            }
        };

        let path = self.resolve_ref_file(&file_part, base.file, is_canonical, ref_str)?;
        let doc = load_schema_document(&path)?;

        let value = if pointer.is_empty() || pointer == "#" {
            doc.clone()
        } else {
            // Strip the leading '#', then walk the pointer. A canonical
            // reference names a definition, so try the two conventional
            // containers before giving up.
            let ptr = &pointer[1..];
            doc.pointer(ptr)
                .cloned()
                .or_else(|| doc.pointer(&format!("/definitions{ptr}")).cloned())
                .or_else(|| doc.pointer(&format!("/$defs{ptr}")).cloned())
                .ok_or_else(|| {
                    ModuleError::new(
                        ErrorCode::SchemaNotFound,
                        format!(
                            "$ref '{ref_str}' resolved to '{}' but pointer '{pointer}' was not found in it",
                            path.display()
                        ),
                    )
                })?
        };

        Ok(RefTarget {
            value,
            root: doc,
            seen_key: format!("{}{pointer}", path.display()),
            file: Some(path),
            same_document: false,
        })
    }

    /// Split `apcore://<dotted.module.id>/<TargetName>` into the file part
    /// (`<dotted.module.id>`) and a JSON-pointer fragment (`#/<TargetName>`).
    fn split_canonical(rest: &str) -> (String, String) {
        match rest.split_once('/') {
            Some((module_id, target)) if !target.is_empty() => {
                // A caller may write either `…/ErrorDetail` or an explicit
                // `…#/definitions/ErrorDetail`; normalize both to a pointer.
                let target = target.strip_prefix('#').unwrap_or(target);
                if target.starts_with('/') {
                    (module_id.to_string(), format!("#{target}"))
                } else {
                    (module_id.to_string(), format!("#/{target}"))
                }
            }
            _ => (rest.trim_end_matches('/').to_string(), String::new()),
        }
    }

    /// Turn a reference's file part into an existing path under `schemas_dir`.
    fn resolve_ref_file(
        &self,
        file_part: &str,
        base_file: Option<&Path>,
        is_canonical: bool,
        ref_str: &str,
    ) -> Result<PathBuf, ModuleError> {
        let Some(schemas_dir) = self.schemas_dir.as_deref() else {
            return Err(ModuleError::new(
                ErrorCode::SchemaNotFound,
                format!(
                    "Referenced schema not found: {ref_str} (cross-file $ref requires a schemas \
                     root — build the resolver with RefResolver::with_schemas_dir, or register the \
                     schema in-memory with RefResolver::register)"
                ),
            ));
        };

        let candidates: Vec<PathBuf> = if is_canonical {
            // Canonical IDs always resolve under the schemas root
            // (PROTOCOL_SPEC §4.11 "Canonical ID references look in schemas/").
            let module_path = file_part.replace('.', "/");
            SCHEMA_FILE_SUFFIXES
                .iter()
                .map(|suffix| schemas_dir.join(format!("{module_path}{suffix}")))
                .chain(std::iter::once(schemas_dir.join(&module_path)))
                .collect()
        } else {
            // Relative paths are relative to the *current* schema file's
            // directory, falling back to the schemas root for a top-level
            // document.
            let anchor = base_file
                .and_then(Path::parent)
                .map_or_else(|| schemas_dir.to_path_buf(), Path::to_path_buf);
            let direct = anchor.join(file_part);
            let mut out = vec![direct.clone()];
            if direct.extension().is_none() {
                out.extend(
                    SCHEMA_FILE_SUFFIXES
                        .iter()
                        .map(|suffix| anchor.join(format!("{file_part}{suffix}"))),
                );
            }
            out
        };

        let root = normalize_path(schemas_dir);
        for candidate in &candidates {
            let resolved = normalize_path(candidate);
            // Containment check: a `../../`-style reference must not escape the
            // schemas root. Checked before the existence probe so a traversal
            // attempt is reported as such rather than as "not found".
            if !resolved.starts_with(&root) {
                return Err(ModuleError::new(
                    ErrorCode::SchemaNotFound,
                    format!(
                        "$ref '{ref_str}' resolves to '{}', which is outside the schemas root '{}'",
                        resolved.display(),
                        root.display()
                    ),
                ));
            }
            if resolved.is_file() {
                return Ok(resolved);
            }
        }

        Err(ModuleError::new(
            ErrorCode::SchemaNotFound,
            format!(
                "Referenced schema not found: {ref_str} (tried {})",
                candidates
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ))
    }

    /// Cycle-detection key for an in-document reference: scoped by file so the
    /// same `#/$defs/x` string in two different documents is not conflated.
    fn seen_key_for(base_file: Option<&Path>, ref_str: &str) -> String {
        base_file.map_or_else(
            || ref_str.to_string(),
            |f| format!("{}{ref_str}", f.display()),
        )
    }
}

/// Read and parse a schema document. YAML is a superset of JSON, so the YAML
/// parser handles both; `.json` files take the strict JSON path for better
/// error messages.
fn load_schema_document(path: &Path) -> Result<serde_json::Value, ModuleError> {
    let contents = std::fs::read_to_string(path).map_err(|e| {
        ModuleError::new(
            ErrorCode::SchemaNotFound,
            format!("Failed to read referenced schema '{}': {e}", path.display()),
        )
    })?;
    if path.extension().is_some_and(|ext| ext == "json") {
        serde_json::from_str(&contents).map_err(|e| {
            ModuleError::new(
                ErrorCode::SchemaParseError,
                format!(
                    "Failed to parse referenced JSON schema '{}': {e}",
                    path.display()
                ),
            )
        })
    } else {
        serde_yaml_ng::from_str(&contents).map_err(|e| {
            ModuleError::new(
                ErrorCode::SchemaParseError,
                format!(
                    "Failed to parse referenced YAML schema '{}': {e}",
                    path.display()
                ),
            )
        })
    }
}

/// Lexically normalize a path (resolve `.` and `..`) and make it absolute
/// against the current directory.
///
/// `std::fs::canonicalize` is deliberately avoided: it requires the path to
/// exist (so a traversal attempt would be reported as "not found" instead of
/// "outside the schemas root") and it resolves symlinks, which would make a
/// legitimately-symlinked schemas tree fail the containment check.
fn normalize_path(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

impl Default for RefResolver {
    fn default() -> Self {
        Self::new()
    }
}
