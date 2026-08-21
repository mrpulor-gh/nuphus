//! Nuphus Memory System

pub mod entry;
pub mod tenets;
pub use entry::{
    build_entry_id, normalize_tags, AgentType, MemoryEntry, MemoryKind, PersistedStep, TimeWindow,
};
pub use tenets::{
    CapacityStatus, EnforceLevel, Tenet, TenetAlert, TenetPriority, TenetSource, TenetStore,
    TenetStoreError,
};

use std::path::PathBuf;
use std::sync::OnceLock;

/// Memory system storage configuration
#[derive(Debug, Clone)]
pub struct MemoryConfig {
    base_path: PathBuf,
}

impl MemoryConfig {
    /// Initialize from `NUPHUS_MEMORY_DIR` env var, fallback to `dirs::data_local_dir()/nuphus/memory`
    pub fn from_env() -> Self {
        let base_path = match std::env::var("NUPHUS_MEMORY_DIR") {
            Ok(dir) => {
                let p = PathBuf::from(dir);
                tracing::info!("[MemoryConfig] using NUPHUS_MEMORY_DIR={}", p.display());
                p
            }
            Err(_) => {
                let mut path = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
                path.push("nuphus");
                path.push("memory");
                tracing::info!("[MemoryConfig] using default path={}", path.display());
                path
            }
        };
        if let Err(e) = std::fs::create_dir_all(&base_path) {
            tracing::warn!("[MemoryConfig] failed to create base dir: {}", e);
        }
        Self { base_path }
    }

    /// Initialize with a custom path
    pub fn new(path: PathBuf) -> Self {
        tracing::info!("[MemoryConfig] using custom path={}", path.display());
        if let Err(e) = std::fs::create_dir_all(&path) {
            tracing::warn!("[MemoryConfig] failed to create base dir: {}", e);
        }
        Self { base_path: path }
    }

    pub fn base_path(&self) -> &PathBuf {
        &self.base_path
    }

    pub fn index_path(&self) -> PathBuf {
        self.base_path.join("timeline_index.json")
    }

    pub fn entries_path(&self) -> PathBuf {
        self.base_path.join("entries.jsonl")
    }

    pub fn entries_index_path(&self) -> PathBuf {
        self.base_path.join("entries_index.json")
    }
}

// ── Global config instance ──

static MEMORY_CONFIG: OnceLock<MemoryConfig> = OnceLock::new();

/// Initialize global memory config (can be called early in Tauri setup)
pub fn init_memory_config(config: MemoryConfig) {
    let _ = MEMORY_CONFIG.set(config);
}

/// Get global memory config (auto-initializes from env on first access)
pub fn get_memory_config() -> &'static MemoryConfig {
    MEMORY_CONFIG.get_or_init(|| {
        tracing::debug!("[MemoryConfig] auto-initializing from env");
        MemoryConfig::from_env()
    })
}
