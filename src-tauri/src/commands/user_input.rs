use nuphus::security::user_input;
use tauri::State;

#[tauri::command]
pub async fn submit_user_input(
    state: State<'_, crate::state::AppState>,
    action_id: String,
    value: String,
) -> Result<String, String> {
    let pending = user_input::get(&state.signals, &action_id)
        .ok_or_else(|| format!("输入请求不存在或已过期: {}", action_id))?;

    user_input::submit(&state.signals, &action_id, value);

    tracing::info!(
        "[USER_INPUT] submitted: title={}, sensitive={}",
        pending.title,
        pending.sensitive
    );
    Ok("submitted".to_string())
}

#[tauri::command]
pub async fn reject_user_input(
    state: State<'_, crate::state::AppState>,
    action_id: String,
) -> Result<String, String> {
    user_input::cancel(&state.signals, &action_id);
    tracing::info!("[USER_INPUT] cancelled: {}", action_id);
    Ok("cancelled".to_string())
}
