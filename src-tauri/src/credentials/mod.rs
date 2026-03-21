//! Credential management — sync current, list, remove.

pub mod steam;
pub mod epic;
pub mod ea;

use serde::{Deserialize, Serialize};
use crate::AppState;
use crate::db::SavedCredential;

#[derive(Debug, Serialize, Deserialize)]
pub struct SyncResult {
    pub success: bool,
    pub launcher: String,
    pub username: String,
    pub message: String,
}

/// Sync (save) the currently logged-in credential for a launcher
#[tauri::command]
pub fn sync_current_credential(
    launcher_id: String,
    state: tauri::State<AppState>,
) -> Result<SyncResult, String> {
    let result = match launcher_id.as_str() {
        "steam" => steam::sync_current()?,
        "epic" => epic::sync_current()?,
        "ea" => ea::sync_current()?,
        _ => return Err(format!("Sync not yet implemented for: {}", launcher_id)),
    };

    let db = state.db.lock().map_err(|e| e.to_string())?;

    // For Epic/EA: single account only — remove any existing credential before saving
    if result.launcher == "epic" || result.launcher == "ea" {
        let existing = db.list_credentials(Some(&result.launcher))?;
        for cred in existing {
            db.remove_credential(cred.id)?;
        }
    }

    db.save_credential(
        &result.launcher,
        &result.username,
        result.registry_data.as_deref(),
        result.file_data.as_deref(),
        result.file_count,
        result.total_size,
    )?;

    db.log("info", &format!("Synced {} credential: {}", result.launcher, result.username));

    Ok(SyncResult {
        success: true,
        launcher: result.launcher,
        username: result.username.clone(),
        message: format!("Saved account '{}'", result.username),
    })
}

/// List all saved credentials
#[tauri::command]
pub fn list_credentials(
    launcher_id: Option<String>,
    state: tauri::State<AppState>,
) -> Result<Vec<SavedCredential>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.list_credentials(launcher_id.as_deref())
}

/// Remove a saved credential
#[tauri::command]
pub fn remove_credential(
    credential_id: i64,
    state: tauri::State<AppState>,
) -> Result<bool, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;

    if let Ok(cred) = db.get_credential(credential_id) {
        db.log("info", &format!("Removed {} credential: {}", cred.launcher, cred.username));
    }

    db.remove_credential(credential_id)
}

/// Internal result from per-launcher sync
pub struct InternalSyncResult {
    pub launcher: String,
    pub username: String,
    pub registry_data: Option<Vec<u8>>,
    pub file_data: Option<Vec<u8>>,
    pub file_count: i32,
    pub total_size: i64,
}
