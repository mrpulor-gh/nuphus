use nuphus::skill::*;
use std::sync::{LazyLock, Mutex};

fn create_skill_registry() -> SkillRegistry {
    SkillRegistry::new()
}

static SKILL_REGISTRY: LazyLock<Mutex<SkillRegistry>> =
    LazyLock::new(|| Mutex::new(create_skill_registry()));

#[tauri::command]
pub fn skill_install(path: String) -> Result<SkillManifest, String> {
    let reg = SKILL_REGISTRY.lock().map_err(|e| e.to_string())?;
    reg.install_from_path(&path)
}

#[tauri::command]
pub fn skill_remove(name: String) -> Result<(), String> {
    let reg = SKILL_REGISTRY.lock().map_err(|e| e.to_string())?;
    reg.remove(&name)
}

#[tauri::command]
pub fn skill_list() -> Result<Vec<SkillEntry>, String> {
    let reg = SKILL_REGISTRY.lock().map_err(|e| e.to_string())?;
    Ok(reg.list())
}

#[tauri::command]
pub fn skill_install_git(url: String) -> Result<SkillManifest, String> {
    let reg = SKILL_REGISTRY.lock().map_err(|e| e.to_string())?;
    reg.install_from_git(&url)
}
