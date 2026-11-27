use std::sync::OnceLock;

use tokio::runtime::{Handle, Runtime};

/// Process-owned scheduler for work that outlives the host's current task or runtime.
pub(crate) fn background_runtime() -> Result<Handle, &'static std::io::Error> {
    static RUNTIME: OnceLock<std::io::Result<Runtime>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("apcore-background")
                .enable_all()
                .build()
        })
        .as_ref()
        .map(|runtime| runtime.handle().clone())
}
