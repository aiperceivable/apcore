//! Platform filesystem watching and application of discovery updates.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::errors::ModuleError;
use crate::events::emitter::ApCoreEvent;
use crate::lifecycle::{lifecycle_runtime, ReloadOptions};

use super::{DiscoveredModule, Registry};

#[derive(Default)]
pub(super) struct FilesystemWatch {
    /// Live filesystem watcher (sync finding A-D-010). `None` when
    /// `watch()` has not been called or has been stopped via `unwatch()`.
    watcher: parking_lot::Mutex<Option<notify::RecommendedWatcher>>,
    /// Background task handle that consumes notify events and triggers
    /// debounced re-discovery. Cleared on `unwatch()`.
    watch_handle: parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

/// A watched path in the form the platform watcher reports it.
///
/// Watchers report resolved paths (`/private/var/...` on macOS for a root under
/// `/var/...`), so both sides of a comparison are canonicalised. A removed file
/// can no longer be canonicalised, so its parent is instead.
fn normalize_watched_path(path: &std::path::Path) -> std::path::PathBuf {
    if let Ok(resolved) = std::fs::canonicalize(path) {
        return resolved;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => {
            std::fs::canonicalize(parent).map_or_else(|_| path.to_path_buf(), |dir| dir.join(name))
        }
        _ => path.to_path_buf(),
    }
}

impl Registry {
    /// Watch the extension roots and apply file changes to the registry.
    ///
    /// Watches every path in `extension_roots` recursively. File events are
    /// grouped after 300 ms of quiet and handled through the configured
    /// discoverer. New candidates are registered. When an existing source or
    /// its metadata sidecar changes, the old instance is suspended and
    /// unregistered before the replacement is registered. The replacement
    /// receives `on_resume` if suspension returned state. A deleted source is
    /// unregistered when its module is absent from the discovery results.
    ///
    /// # Caller obligations
    ///
    /// `watch()` requires `&Arc<Self>` because the spawned background task
    /// holds a `Weak<Registry>` to drive re-discovery; the registry must
    /// already live behind `Arc`. Calling on a non-Arc Registry is a
    /// compile-time error.
    ///
    /// # Errors
    ///
    /// - `Err(ModuleError(code=ReloadFailed))` if the platform watcher cannot
    ///   be created (kernel resource exhaustion, missing permissions, etc.)
    ///
    /// # Idempotency
    ///
    /// Calling `watch()` while already watching is a no-op (returns `Ok(())`).
    /// Use [`Self::unwatch`] to stop and re-call `watch()` to pick up new
    /// `extension_roots`.
    // Keep the async API for existing callers. Startup installs the watcher
    // and spawns its Tokio task; the task consumes events after this returns.
    // Rust 1.98 split the lint. The unknown-lints allowance keeps the new lint
    // name compatible with older toolchains.
    #[allow(unknown_lints)]
    #[allow(clippy::unused_async)]
    #[allow(clippy::unused_async_trait_impl)]
    pub async fn watch(self: &Arc<Self>) -> Result<(), ModuleError> {
        self.start_watcher(None)
    }

    /// Watch configured roots with managed, generation-aware replacement.
    pub fn watch_with_lifecycle(
        self: &Arc<Self>,
        options: ReloadOptions,
    ) -> Result<(), ModuleError> {
        self.start_watcher(Some(options.validate()?))
    }

    fn start_watcher(
        self: &Arc<Self>,
        lifecycle: Option<ReloadOptions>,
    ) -> Result<(), ModuleError> {
        use notify::{RecursiveMode, Watcher};

        if self.core.read().closed {
            return Err(Self::closed_error());
        }

        {
            let watcher_slot = self.filesystem_watch.watcher.lock();
            if watcher_slot.is_some() {
                return Ok(()); // already watching
            }
        }

        let extension_roots: Vec<String> = self.extension_roots.read().clone();
        if extension_roots.is_empty() {
            tracing::warn!(
                "Registry::watch() called with no extension_roots — call set_extension_roots() first"
            );
            return Ok(());
        }

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<notify::Result<notify::Event>>();

        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            // Best-effort send; if the receiver is gone, the watcher will be
            // dropped shortly after.
            let _ = tx.send(res);
        })
        .map_err(|e| {
            ModuleError::new(
                crate::errors::ErrorCode::ReloadFailed,
                format!("Failed to create file watcher: {e}"),
            )
        })?;

        for root in &extension_roots {
            let path = std::path::Path::new(root);
            if let Err(e) = watcher.watch(path, RecursiveMode::Recursive) {
                tracing::warn!(
                    root = %root,
                    error = %e,
                    "Registry::watch failed for root, skipping"
                );
            }
        }

        let core = self.core.read();
        if core.closed {
            return Err(Self::closed_error());
        }
        let mut watcher_slot = self.filesystem_watch.watcher.lock();
        if watcher_slot.is_none() {
            let runtime = lifecycle_runtime()?;
            let handle = runtime.spawn(Self::watch_loop(rx, Arc::downgrade(self), lifecycle));
            *watcher_slot = Some(watcher);
            *self.filesystem_watch.watch_handle.lock() = Some(handle);
        }

        Ok(())
    }

    /// Background task body that consumes notify events and applies each
    /// debounced batch of changed paths. Exits when the receiver channel
    /// closes or the `Weak<Registry>` can no longer be upgraded.
    async fn watch_loop(
        mut rx: tokio::sync::mpsc::UnboundedReceiver<notify::Result<notify::Event>>,
        weak: std::sync::Weak<Self>,
        lifecycle: Option<ReloadOptions>,
    ) {
        use std::time::Duration;

        const DEBOUNCE: Duration = Duration::from_millis(300);

        fn relevant(event: &notify::Event) -> bool {
            matches!(
                event.kind,
                notify::EventKind::Create(_)
                    | notify::EventKind::Modify(_)
                    | notify::EventKind::Remove(_)
            )
        }

        while let Some(res) = rx.recv().await {
            let Ok(event) = res else {
                continue;
            };
            if !relevant(&event) {
                continue;
            }
            // Collapse a burst (editors writing through temp file + rename)
            // into one pass, keeping every path the burst touched.
            let mut paths: Vec<std::path::PathBuf> = event.paths;
            while let Ok(Some(res)) = tokio::time::timeout(DEBOUNCE, rx.recv()).await {
                if let Ok(event) = res {
                    if relevant(&event) {
                        paths.extend(event.paths);
                    }
                }
            }

            let Some(reg) = weak.upgrade() else {
                break;
            };
            let task = tokio::spawn(async move { reg.apply_file_changes(&paths, lifecycle).await });
            if let Ok(Err(e)) = task.await {
                tracing::warn!(
                    error = %e.message,
                    "Registry watch: applying file changes failed during hot-reload"
                );
            }
        }
    }

    /// Apply a batch of changed file paths: register new modules, replace the
    /// modules whose source (or `_meta.yaml` sidecar) changed, and unregister
    /// those whose source file is gone.
    async fn apply_file_changes(
        self: &Arc<Self>,
        paths: &[std::path::PathBuf],
        lifecycle: Option<ReloadOptions>,
    ) -> Result<(), ModuleError> {
        let changed: HashSet<std::path::PathBuf> =
            paths.iter().map(|p| normalize_watched_path(p)).collect();
        let touches = |source: &str| {
            let source = normalize_watched_path(std::path::Path::new(source));
            if changed.contains(&source) {
                return true;
            }
            let (Some(dir), Some(stem)) = (source.parent(), source.file_stem()) else {
                return false;
            };
            changed.contains(&dir.join(format!("{}_meta.yaml", stem.to_string_lossy())))
        };

        let discovered = self.run_discoverer().await?;

        let removed: Vec<String> = {
            let core = self.core.read();
            core.sources
                .iter()
                .filter(|(id, source)| {
                    !std::path::Path::new(source).exists()
                        && touches(source)
                        && !discovered.iter().any(|dm| &dm.name == *id)
                })
                .map(|(id, _)| id.clone())
                .collect()
        };
        for id in removed {
            self.safe_unregister(&id, 5000).await?;
        }

        let batch_ids = discovered.iter().map(|entry| entry.name.clone()).collect();
        for dm in discovered {
            if !self.has(&dm.name) {
                self.register_discovered_batch(
                    std::iter::once(dm),
                    &batch_ids,
                    &HashMap::new(),
                    true,
                )
                .await;
            } else if touches(&dm.source) {
                if let Some(options) = lifecycle {
                    self.start_managed_reload_with_candidate(
                        vec![dm.name.clone()],
                        options,
                        Some(dm),
                    )?
                    .wait(None)
                    .await?;
                } else {
                    self.replace_discovered(dm, &batch_ids).await?;
                }
            }
        }
        Ok(())
    }

    /// Replace a registered module with a freshly discovered instance,
    /// handing its `on_suspend` state to the new instance's `on_resume`. The
    /// previous instance is put back if the new one fails to register.
    async fn replace_discovered(
        &self,
        dm: DiscoveredModule,
        batch_ids: &HashSet<String>,
    ) -> Result<(), ModuleError> {
        let name = dm.name.clone();
        let Some(previous) = self.unload_for_reload(&name).await? else {
            return Ok(());
        };
        let states: HashMap<_, _> = previous
            .state
            .as_ref()
            .map(|state| (name.clone(), state.clone()))
            .into_iter()
            .collect();
        if self
            .register_discovered_batch(std::iter::once(dm), batch_ids, &states, true)
            .await
            == 0
        {
            tracing::warn!(
                module_id = %name,
                "Registry watch: the changed module failed to register; keeping the previous instance"
            );
            let restoration = self.restore_reload(&name, previous).await;
            if let Some(emitter) = self.event_emitter.read().clone() {
                emitter.emit_delivery_semantics(ApCoreEvent::with_module(
                    "apcore.registry.module_reload_failed",
                    serde_json::json!({
                        "operation_id": uuid::Uuid::new_v4().to_string(),
                        "phase": "loading", "error_code": "RELOAD_FAILED",
                        "compensation": if restoration.is_ok() { "restored" } else { "failed" },
                    }),
                    &name,
                    "error",
                ));
            }
            restoration?;

            return Ok(());
        }

        Ok(())
    }

    /// Stop filesystem watching. The background task is aborted and the
    /// platform watcher is dropped. Idempotent — calling on a non-watching
    /// Registry is a no-op.
    pub fn unwatch(&self) {
        self.stop_watching();
    }

    pub(super) fn stop_watching(&self) -> Option<tokio::task::JoinHandle<()>> {
        let (watcher, handle) = {
            let mut watcher = self.filesystem_watch.watcher.lock();
            (
                watcher.take(),
                self.filesystem_watch.watch_handle.lock().take(),
            )
        };
        drop(watcher);
        if let Some(handle) = &handle {
            handle.abort();
        }
        handle
    }
}
