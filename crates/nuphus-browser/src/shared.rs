//! Process-level shared BrowserClient (singleton).
//!
//! All browser consumers (browser_tools / web CDP rendering / cookies CDP data source)
//! reuse the same instance: the embedded browser uses a fixed profile_dir, and multiple
//! instances concurrently launching the same profile would conflict with each other;
//! a singleton + mutex eliminates the problem at the root.
//!
//! Runtime note: chromiumoxide's CDP handler is `tokio::spawn`ed onto the ambient runtime at
//! launch, and its lifetime must match BrowserClient. Any browser operation initiated from a
//! synchronous context runs on [`runtime()`], the process-level resident runtime; temporary
//! runtimes are forbidden (dropping one kills the handler, leaving a zombie browser connection).

use std::sync::{Arc, OnceLock};
use tokio::sync::{Mutex, OwnedMutexGuard};

use super::client::BrowserClient;

/// Browser-specific process-level resident tokio runtime (see module docs).
pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Runtime::new().expect("failed to create browser runtime"))
}

/// Process-level shared BrowserClient handle (lazily initialized; not launched initially).
pub fn shared_client() -> Arc<Mutex<Option<BrowserClient>>> {
    static INSTANCE: OnceLock<Arc<Mutex<Option<BrowserClient>>>> = OnceLock::new();
    INSTANCE.get_or_init(|| Arc::new(Mutex::new(None))).clone()
}

/// get-or-launch: hold the lock to ensure a client exists and launch it per the requested mode, returning an owned guard.
///
/// While the caller holds the guard, the browser is exclusively owned, guaranteeing that
/// navigate/evaluate/cookies etc. CDP operation sequences do not interleave with other consumers;
/// dropping the guard releases it.
pub async fn get_or_launch(
    headless: bool,
) -> Result<OwnedMutexGuard<Option<BrowserClient>>, String> {
    let mut guard = shared_client().lock_owned().await;
    if guard.is_none() {
        let client =
            BrowserClient::new().map_err(|e| format!("Browser automation unavailable: {}", e))?;
        *guard = Some(client);
    }
    let client = guard.as_mut().expect("client just initialized");
    client
        .launch(headless)
        .await
        .map_err(|e| format!("Failed to launch browser: {}", e))?;
    Ok(guard)
}
