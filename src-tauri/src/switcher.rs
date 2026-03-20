//! Account switching — kill launcher, patch credentials, restart, verify.

use serde::{Deserialize, Serialize};
use crate::AppState;

#[derive(Debug, Serialize, Deserialize)]
pub struct SwitchResult {
    pub success: bool,
    pub launcher: String,
    pub new_user: Option<String>,
    pub steps: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LauncherState {
    pub launcher: String,
    pub is_running: bool,
    pub current_user: Option<String>,
    pub active_user_id: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestAllResult {
    pub results: Vec<TestAccountResult>,
    pub passed: usize,
    pub failed: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestAccountResult {
    pub username: String,
    pub success: bool,
    pub message: String,
}

/// Switch to a saved credential by ID from our DB.
/// Auto-saves the currently logged-in account before switching (TcNo pattern).
#[tauri::command]
pub async fn switch_account(
    credential_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<SwitchResult, String> {
    let (cred, file_data) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let cred = db.get_credential(credential_id)?;
        let (_, file) = db.get_credential_data(credential_id)?;
        (cred, file)
    };

    let file_data = file_data
        .ok_or(format!("No saved file data for '{}'. Re-sync this account.", cred.username))?;

    // Auto-save the currently logged-in account BEFORE switching.
    // This ensures the outgoing account's tokens are captured fresh,
    // so switching back to it later will work.
    auto_save_current(&cred.launcher, &state)?;

    match cred.launcher.as_str() {
        "steam" => switch_steam(&cred.username, &file_data).await,
        "epic" => switch_epic(&cred.username, &file_data).await,
        _ => Err(format!("Switching not yet implemented for: {}", cred.launcher)),
    }
}

/// Auto-save the currently logged-in account's files to DB before switching away.
fn auto_save_current(launcher_id: &str, state: &tauri::State<'_, AppState>) -> Result<(), String> {
    let sync_result = match launcher_id {
        "steam" => crate::credentials::steam::sync_current(),
        "epic" => crate::credentials::epic::sync_current(),
        _ => return Ok(()), // No auto-save for unsupported launchers
    };

    match sync_result {
        Ok(result) => {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            db.save_credential(
                &result.launcher,
                &result.username,
                result.registry_data.as_deref(),
                result.file_data.as_deref(),
                result.file_count,
                result.total_size,
            )?;
            db.log("info", &format!("Auto-saved current {} account: {}", launcher_id, result.username));
            Ok(())
        }
        Err(e) => {
            // Non-fatal — if we can't save current (e.g., not logged in), proceed anyway
            log::warn!("[Auto-save] Could not save current {} account: {}", launcher_id, e);
            Ok(())
        }
    }
}

/// Test all saved credentials for a launcher — switch to each, verify, report
#[tauri::command]
pub async fn test_all_accounts(
    launcher_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<TestAllResult, String> {
    let credentials = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.list_credentials(Some(&launcher_id))?
    };

    if credentials.is_empty() {
        return Err("No saved credentials to test. Sync some accounts first.".into());
    }

    let mut results = Vec::new();
    let mut passed = 0;
    let mut failed = 0;

    // Remember which account was active before testing
    let original_user = crate::launcher_detect::get_launcher_current_user(launcher_id.clone())
        .unwrap_or(None);

    for cred in &credentials {
        log::info!("[Test] Testing account: {}", cred.username);

        let file_data = {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            let (_, file) = db.get_credential_data(cred.id)?;
            file
        };

        let result = match (launcher_id.as_str(), file_data) {
            ("steam", Some(ref fd)) => switch_steam(&cred.username, fd).await,
            ("epic", Some(ref fd)) => switch_epic(&cred.username, fd).await,
            (_, Some(_)) => Err("Not implemented".into()),
            (_, None) => Err(format!("No saved file data for '{}'", cred.username)),
        };

        match result {
            Ok(sr) if sr.success => {
                results.push(TestAccountResult {
                    username: cred.username.clone(),
                    success: true,
                    message: "Auto-login successful".into(),
                });
                passed += 1;
            }
            Ok(sr) => {
                results.push(TestAccountResult {
                    username: cred.username.clone(),
                    success: false,
                    message: sr.error.unwrap_or("Unknown failure".into()),
                });
                failed += 1;
            }
            Err(e) => {
                results.push(TestAccountResult {
                    username: cred.username.clone(),
                    success: false,
                    message: e,
                });
                failed += 1;
            }
        }
    }

    // Restore original account if possible
    if let Some(ref orig) = original_user {
        let orig_data = {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            let creds = db.list_credentials(Some(&launcher_id))?;
            creds.iter().find(|c| c.username == *orig).map(|c| {
                db.get_credential_data(c.id).ok().and_then(|(_, f)| f)
            }).flatten()
        };
        if let Some(ref fd) = orig_data {
            match launcher_id.as_str() {
                "steam" => { let _ = switch_steam(orig, fd).await; },
                "epic" => { let _ = switch_epic(orig, fd).await; },
                _ => {},
            }
        }
    }

    Ok(TestAllResult { results, passed, failed })
}

/// Get current state of a launcher
#[tauri::command]
pub fn verify_launcher_state(launcher_id: String) -> Result<LauncherState, String> {
    let current_user = crate::launcher_detect::get_launcher_current_user(launcher_id.clone())?;
    let is_running = match launcher_id.as_str() {
        "steam" => is_process_running("steam.exe"),
        "epic" => is_process_running("EpicGamesLauncher.exe"),
        "riot" => is_process_running("RiotClientServices.exe"),
        _ => false,
    };

    let active_user_id = if launcher_id == "steam" {
        get_steam_active_user_id()
    } else {
        0
    };

    Ok(LauncherState {
        launcher: launcher_id,
        is_running,
        current_user,
        active_user_id,
    })
}

// ═══════════════════════════════════════════════════════════════════════════════
// Steam Switching
// ═══════════════════════════════════════════════════════════════════════════════

async fn switch_steam(username: &str, file_data: &[u8]) -> Result<SwitchResult, String> {
    let mut steps = Vec::new();
    let launcher = "steam".to_string();

    // Step 1: Kill Steam
    let killed = kill_process("steam.exe");
    if killed > 0 {
        steps.push(format!("Killed {} Steam process(es)", killed));
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    } else {
        steps.push("Steam was not running".into());
    }

    // Step 2: Restore saved files (config.vdf + loginusers.vdf) + patch VDF + set registry
    let restore_steps = crate::credentials::steam::restore_and_switch(username, file_data)?;
    steps.extend(restore_steps);

    // Step 3: Restart Steam
    #[cfg(target_os = "windows")]
    {
        let steam_dir = steamlocate::SteamDir::locate()
            .map_err(|_| "Steam not installed".to_string())?;
        let steam_exe = steam_dir.path().join("steam.exe");
        if steam_exe.exists() {
            std::process::Command::new(&steam_exe)
                .spawn()
                .map_err(|e| format!("Failed to start Steam: {}", e))?;
            steps.push("Started Steam".into());
        }
    }

    // Step 5: Verify — poll ActiveUser registry for up to 18 seconds
    steps.push("Waiting for Steam to start...".into());
    let mut logged_in = false;
    for i in 0..6 {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let active_user = get_steam_active_user_id();
        if active_user > 0 {
            logged_in = true;
            steps.push(format!("Steam logged in (ActiveUser: {}, after {}s)", active_user, (i + 1) * 3));
            break;
        }
    }

    let current = crate::launcher_detect::get_launcher_current_user("steam".into())
        .unwrap_or(None);

    let (success, status_msg) = if logged_in {
        (true, format!("Auto-login successful — logged in as {}", current.as_deref().unwrap_or("unknown")))
    } else {
        (false, format!(
            "Auto-login FAILED — Steam showed login screen. Account '{}' needs to be logged in manually with 'Remember me' checked on this PC.",
            username
        ))
    };

    steps.push(status_msg.clone());

    Ok(SwitchResult {
        success,
        launcher,
        new_user: if success { current } else { None },
        steps,
        error: if success { None } else { Some(status_msg) },
    })
}

// ═══════════════════════════════════════════════════════════════════════════════
// Epic Switching
// ═══════════════════════════════════════════════════════════════════════════════

async fn switch_epic(account_id: &str, file_data: &[u8]) -> Result<SwitchResult, String> {
    let mut steps = Vec::new();
    let launcher = "epic".to_string();

    // Step 1: Kill Epic
    let killed = kill_process("EpicGamesLauncher.exe");
    if killed > 0 {
        steps.push(format!("Killed {} Epic process(es)", killed));
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    } else {
        steps.push("Epic was not running".into());
    }

    // Step 2: Restore saved files + registry + clear caches
    let restore_steps = crate::credentials::epic::restore_and_switch(account_id, file_data)?;
    steps.extend(restore_steps);

    // Step 3: Restart Epic
    #[cfg(target_os = "windows")]
    {
        // Find Epic exe from registry or default path
        let epic_exe = find_epic_exe();
        if let Some(exe) = epic_exe {
            std::process::Command::new(&exe)
                .spawn()
                .map_err(|e| format!("Failed to start Epic: {}", e))?;
            steps.push("Started Epic Games Launcher".into());
        } else {
            steps.push("Epic exe not found — start manually".into());
        }
    }

    // Step 4: Verify — wait and check registry AccountId
    steps.push("Waiting for Epic to start...".into());
    tokio::time::sleep(std::time::Duration::from_secs(8)).await;

    let current = crate::launcher_detect::get_launcher_current_user("epic".into())
        .unwrap_or(None);
    let success = current.as_deref() == Some(account_id);

    let status_msg = if success {
        format!("Auto-login successful — account {}", &account_id[..8.min(account_id.len())])
    } else {
        format!("Switch may have failed — current account: {:?}", current)
    };
    steps.push(status_msg.clone());

    Ok(SwitchResult {
        success,
        launcher,
        new_user: current,
        steps,
        error: if success { None } else { Some(status_msg) },
    })
}

#[cfg(target_os = "windows")]
pub fn find_epic_exe() -> Option<String> {
    // Try default path
    let default = r"C:\Program Files (x86)\Epic Games\Launcher\Portal\Binaries\Win32\EpicGamesLauncher.exe";
    if std::path::Path::new(default).exists() {
        return Some(default.to_string());
    }
    // Try Program Files
    let alt = r"C:\Program Files\Epic Games\Launcher\Portal\Binaries\Win64\EpicGamesLauncher.exe";
    if std::path::Path::new(alt).exists() {
        return Some(alt.to_string());
    }
    None
}

// ═══════════════════════════════════════════════════════════════════════════════
// VDF Patching
// ═══════════════════════════════════════════════════════════════════════════════

fn patch_vdf_most_recent(vdf_path: &std::path::Path, target_username: &str) -> Result<(), String> {
    let content = std::fs::read_to_string(vdf_path)
        .map_err(|e| format!("Failed to read VDF: {}", e))?;

    let mut output = String::new();
    let mut current_is_target = false;
    let mut current_account_name: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();

        // Detect SteamID64 block start
        if trimmed.starts_with("\"7656") {
            current_is_target = false;
            current_account_name = None;
        }

        // Detect AccountName
        if trimmed.starts_with("\"AccountName\"") {
            if let Some(name) = extract_vdf_value(trimmed) {
                current_account_name = Some(name.clone());
                current_is_target = name.eq_ignore_ascii_case(target_username);
            }
        }

        // Patch MostRecent
        if trimmed.starts_with("\"MostRecent\"") || trimmed.starts_with("\"mostrecent\"") {
            let val = if current_is_target { "1" } else { "0" };
            let indent = line.len() - line.trim_start().len();
            let indent_str: String = std::iter::repeat('\t').take(indent / 2 + 1).chain(std::iter::repeat('\t').take(1)).collect();
            output.push_str(&format!("\t\t\"MostRecent\"\t\t\"{}\"", val));
            output.push('\n');
            continue;
        }

        // Patch AllowAutoLogin (set to 1 for target, 0 for others)
        if trimmed.starts_with("\"AllowAutoLogin\"") {
            let val = if current_is_target { "1" } else { "0" };
            output.push_str(&format!("\t\t\"AllowAutoLogin\"\t\t\"{}\"", val));
            output.push('\n');
            continue;
        }

        output.push_str(line);
        output.push('\n');
    }

    std::fs::write(vdf_path, &output)
        .map_err(|e| format!("Failed to write patched VDF: {}", e))?;

    Ok(())
}

fn extract_vdf_value(line: &str) -> Option<String> {
    let parts: Vec<&str> = line.split('"').collect();
    if parts.len() >= 4 {
        Some(parts[3].to_string())
    } else {
        None
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════════

fn get_steam_active_user_id() -> u32 {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(key) = hkcu.open_subkey(r"Software\Valve\Steam\ActiveProcess") {
            if let Ok(val) = key.get_value::<u32, _>("ActiveUser") {
                return val;
            }
        }
    }
    0
}

fn kill_process(name: &str) -> u32 {
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        let output = Command::new("taskkill")
            .args(["/F", "/IM", name])
            .output();
        match output {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                stdout.matches("SUCCESS").count() as u32
            }
            Err(_) => 0,
        }
    }
    #[cfg(not(target_os = "windows"))]
    { 0 }
}

fn is_process_running(name: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        if let Ok(output) = Command::new("tasklist")
            .args(["/FI", &format!("IMAGENAME eq {}", name), "/NH"])
            .output()
        {
            return String::from_utf8_lossy(&output.stdout).contains(name);
        }
    }
    false
}
