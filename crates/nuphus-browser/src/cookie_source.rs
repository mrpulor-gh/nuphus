//! Pluggable cookie source for `BrowserClient::import_cookies`.
//!
//! The browser module itself does not care where cookies come from (Chrome SQLite / mock / other data sources).
//! Host applications (Nuphus main app / nuphus-mcp) register a data source via [`set_loader`];
//! `BrowserClient::import_cookies` fetches data through [`load`] and injects it into the current page via CDP `Network.setCookie`.
//!
//! Extraction background: in the original main crate, `import_cookies` directly referenced `crate::cookies::vault()`
//! (Chrome default profile's Cookies SQLite + DPAPI decryption + in-memory cache). Once extracted into a standalone
//! crate, that data source becomes a host-side capability, so it was turned into a pluggable loader — the main crate
//! registers it once at startup, while nuphus-mcp does not register it (`load` returns a clear error).

use std::sync::OnceLock;

/// A cookie entry compatible with CDP `Network.setCookie`.
#[derive(Debug, Clone)]
pub struct CookieData {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: Option<String>,
    pub expires: Option<f64>,
}

type Loader = fn(Option<&str>) -> Result<Vec<CookieData>, String>;

static LOADER: OnceLock<Loader> = OnceLock::new();

/// Register a cookie data source (can only be registered once per process; duplicate registration returns Err).
pub fn set_loader(loader: Loader) -> Result<(), String> {
    LOADER
        .set(loader)
        .map_err(|_| "cookie source already registered".to_string())
}

/// Whether a cookie data source has been registered.
pub fn is_registered() -> bool {
    LOADER.get().is_some()
}

/// Fetch cookies through the registered data source (all cookies when `domain_filter` is None).
pub fn load(domain_filter: Option<&str>) -> Result<Vec<CookieData>, String> {
    match LOADER.get() {
        Some(f) => f(domain_filter),
        None => Err(
            "no cookie source registered — BrowserClient::import_cookies unavailable".to_string(),
        ),
    }
}
