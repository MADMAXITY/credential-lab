//! Epic Games credential sync & restore.
//!
//! Based on file diff analysis between two logged-in accounts:
//!
//! Account-specific files (must save/restore per account):
//!   1. Config/WindowsEditor/GameUserSettings.ini — RememberMe token
//!   2. webcache_XXXX/Cookies — Chromium session cookies (SQLite DB)
//!   3. webcache_XXXX/Cookies-journal — WAL journal for Cookies
//!   4. Registry: HKCU\Software\Epic Games\Unreal Engine\Identifiers:AccountId
//!
//! NOT account-specific (leave alone):
//!   - webcache Cache files (f_000XXX) — static assets
//!   - Data/*.dat files — ownership cache, regenerated
//!   - Config/Config/ subdirectories — base config, not auth

use super::InternalSyncResult;
use std::collections::HashMap;
use std::path::PathBuf;

fn get_epic_saved_dir() -> Result<PathBuf, String> {
    let local_app = std::env::var("LOCALAPPDATA")
        .map_err(|_| "LOCALAPPDATA not set".to_string())?;
    Ok(PathBuf::from(local_app)
        .join("EpicGamesLauncher")
        .join("Saved"))
}

fn get_epic_account_id() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(key) = hkcu.open_subkey(r"Software\Epic Games\Unreal Engine\Identifiers") {
            if let Ok(id) = key.get_value::<String, _>("AccountId") {
                if !id.is_empty() {
                    return Some(id);
                }
            }
        }
    }
    None
}

/// Find the webcache directory (named webcache_XXXX)
fn find_webcache_dir(saved_dir: &PathBuf) -> Option<PathBuf> {
    if let Ok(entries) = std::fs::read_dir(saved_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("webcache") && entry.path().is_dir() {
                return Some(entry.path());
            }
        }
    }
    None
}

/// Sync the currently logged-in Epic account.
/// Kills Epic first to release file locks on Cookies DB.
/// Saves: GameUserSettings.ini + Cookies + Cookies-journal + registry AccountId.
pub fn sync_current() -> Result<InternalSyncResult, String> {
    let account_id = get_epic_account_id()
        .ok_or("No Epic Games account logged in.")?;

    // Wait a moment to ensure Epic has flushed all auth data to disk
    // before we kill it. The RememberMe token in GameUserSettings.ini
    // may not be fully written if we kill too fast.
    log::info!("[Epic Sync] Waiting 3s for Epic to flush auth data...");
    std::thread::sleep(std::time::Duration::from_secs(3));

    // Kill Epic to release file locks on Cookies
    kill_epic();
    std::thread::sleep(std::time::Duration::from_secs(2));

    let saved_dir = get_epic_saved_dir()?;
    let mut file_map: HashMap<String, String> = HashMap::new();
    let mut total_size: i64 = 0;
    let mut file_count: i32 = 0;

    // 1. GameUserSettings.ini
    let ini_path = saved_dir.join("Config").join("WindowsEditor").join("GameUserSettings.ini");
    if ini_path.exists() {
        let data = std::fs::read(&ini_path)
            .map_err(|e| format!("Failed to read GameUserSettings.ini: {}", e))?;

        // Validate: INI must contain a RememberMe token and be a reasonable size
        let content = String::from_utf8_lossy(&data);
        if !content.contains("[RememberMe]") || !content.contains("Data=") {
            return Err("GameUserSettings.ini does not contain RememberMe token. Make sure you're fully logged in.".into());
        }
        if data.len() < 2500 {
            return Err(format!(
                "GameUserSettings.ini is too small ({} bytes) — Epic may not have finished writing auth data. Wait a few seconds and try again.",
                data.len()
            ));
        }

        total_size += data.len() as i64;
        file_map.insert("GameUserSettings.ini".into(), hex_encode(&data));
        file_count += 1;
        log::info!("[Epic Sync] Saved GameUserSettings.ini ({} bytes)", data.len());
    } else {
        return Err("GameUserSettings.ini not found".into());
    }

    // 2. Cookies + Cookies-journal from webcache
    let webcache_dir = find_webcache_dir(&saved_dir);
    if let Some(ref wc_dir) = webcache_dir {
        let cookies_path = wc_dir.join("Cookies");
        if cookies_path.exists() {
            match std::fs::read(&cookies_path) {
                Ok(data) => {
                    total_size += data.len() as i64;
                    file_map.insert("Cookies".into(), hex_encode(&data));
                    file_count += 1;
                    log::info!("[Epic Sync] Saved Cookies ({} bytes)", data.len());
                }
                Err(e) => log::warn!("[Epic Sync] Could not read Cookies: {}", e),
            }
        }

        let journal_path = wc_dir.join("Cookies-journal");
        if journal_path.exists() {
            match std::fs::read(&journal_path) {
                Ok(data) => {
                    total_size += data.len() as i64;
                    file_map.insert("Cookies-journal".into(), hex_encode(&data));
                    file_count += 1;
                }
                Err(e) => log::warn!("[Epic Sync] Could not read Cookies-journal: {}", e),
            }
        }
    }

    // 3. Registry AccountId
    file_map.insert("__registry_AccountId__".into(), account_id.clone());

    let file_data = serde_json::to_vec(&file_map)
        .map_err(|e| format!("Failed to serialize: {}", e))?;

    // Restart Epic
    start_epic();

    log::info!("[Epic Sync] Saved {} files for '{}' ({} bytes)",
        file_count, &account_id[..8.min(account_id.len())], total_size);

    Ok(InternalSyncResult {
        launcher: "epic".into(),
        username: account_id,
        registry_data: None,
        file_data: Some(file_data),
        file_count,
        total_size,
    })
}

/// Restore saved Epic credentials.
/// Replaces: GameUserSettings.ini + Cookies + Cookies-journal + registry AccountId.
/// Does NOT touch anything else in webcache.
pub fn restore_and_switch(account_id: &str, file_data: &[u8]) -> Result<Vec<String>, String> {
    let mut steps = Vec::new();

    let saved_dir = get_epic_saved_dir()?;

    let file_map: HashMap<String, String> = serde_json::from_slice(file_data)
        .map_err(|e| format!("Failed to parse saved data: {}", e))?;

    // 1. Restore GameUserSettings.ini
    if let Some(ini_hex) = file_map.get("GameUserSettings.ini") {
        let data = hex_decode(ini_hex)?;
        let ini_path = saved_dir.join("Config").join("WindowsEditor").join("GameUserSettings.ini");
        if let Some(parent) = ini_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&ini_path, &data)
            .map_err(|e| format!("Failed to write GameUserSettings.ini: {}", e))?;
        steps.push(format!("Restored GameUserSettings.ini ({} bytes)", data.len()));
    }

    // 2. Restore Cookies + Cookies-journal in webcache
    let webcache_dir = find_webcache_dir(&saved_dir);
    if let Some(ref wc_dir) = webcache_dir {
        if let Some(cookies_hex) = file_map.get("Cookies") {
            let data = hex_decode(cookies_hex)?;
            std::fs::write(wc_dir.join("Cookies"), &data)
                .map_err(|e| format!("Failed to write Cookies: {}", e))?;
            steps.push(format!("Restored Cookies ({} bytes)", data.len()));
        }

        if let Some(journal_hex) = file_map.get("Cookies-journal") {
            let data = hex_decode(journal_hex)?;
            std::fs::write(wc_dir.join("Cookies-journal"), &data)
                .map_err(|e| format!("Failed to write Cookies-journal: {}", e))?;
            steps.push("Restored Cookies-journal".into());
        } else {
            // If no journal was saved, delete current one to avoid mismatch
            let journal_path = wc_dir.join("Cookies-journal");
            if journal_path.exists() {
                let _ = std::fs::remove_file(&journal_path);
            }
        }
    } else {
        steps.push("Warning: webcache directory not found".into());
    }

    // 3. Set registry AccountId
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok((key, _)) = hkcu.create_subkey(r"Software\Epic Games\Unreal Engine\Identifiers") {
            let saved_id = file_map.get("__registry_AccountId__")
                .map(|s| s.as_str())
                .unwrap_or(account_id);
            key.set_value("AccountId", &saved_id)
                .map_err(|e| format!("Failed to set AccountId: {}", e))?;
            steps.push(format!("Set registry AccountId = {}", &saved_id[..8.min(saved_id.len())]));
        }
    }

    Ok(steps)
}

/// Wipe Epic login state WITHOUT going through Epic's logout.
/// Kills Epic, clears GameUserSettings.ini RememberMe + registry AccountId.
/// This does NOT revoke the token server-side — it just removes it from disk.
/// Next Epic launch will show login screen.
pub fn wipe_login_state() -> Result<Vec<String>, String> {
    let mut steps = Vec::new();

    kill_epic();
    std::thread::sleep(std::time::Duration::from_secs(2));
    steps.push("Killed Epic".into());

    let saved_dir = get_epic_saved_dir()?;

    // Clear RememberMe in GameUserSettings.ini (but keep the file)
    let ini_path = saved_dir.join("Config").join("WindowsEditor").join("GameUserSettings.ini");
    if ini_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&ini_path) {
            let mut output = String::new();
            let mut in_section = false;
            for line in content.lines() {
                if line.trim() == "[RememberMe]" {
                    in_section = true;
                    output.push_str("[RememberMe]\n");
                    output.push_str("Enable=False\n");
                    continue;
                }
                if in_section && line.starts_with('[') {
                    in_section = false;
                }
                if !in_section {
                    output.push_str(line);
                    output.push('\n');
                }
            }
            let _ = std::fs::write(&ini_path, output);
            steps.push("Cleared RememberMe token".into());
        }
    }

    // Clear registry AccountId
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok((key, _)) = hkcu.create_subkey(r"Software\Epic Games\Unreal Engine\Identifiers") {
            let _ = key.set_value("AccountId", &"");
            steps.push("Cleared registry AccountId".into());
        }
    }

    // Delete webcache Cookies (session data)
    if let Some(wc_dir) = find_webcache_dir(&saved_dir) {
        let _ = std::fs::remove_file(wc_dir.join("Cookies"));
        let _ = std::fs::remove_file(wc_dir.join("Cookies-journal"));
        steps.push("Cleared webcache Cookies".into());
    }

    steps.push("Login wiped. Epic will show login screen on next launch.".into());
    Ok(steps)
}

pub fn sync_current_for_auto_save() -> Result<InternalSyncResult, String> {
    // For auto-save, same as sync but don't restart Epic (switch will handle it)
    let account_id = get_epic_account_id()
        .ok_or("No Epic Games account logged in.")?;

    kill_epic();
    std::thread::sleep(std::time::Duration::from_secs(2));

    let saved_dir = get_epic_saved_dir()?;
    let mut file_map: HashMap<String, String> = HashMap::new();
    let mut total_size: i64 = 0;
    let mut file_count: i32 = 0;

    let ini_path = saved_dir.join("Config").join("WindowsEditor").join("GameUserSettings.ini");
    if ini_path.exists() {
        if let Ok(data) = std::fs::read(&ini_path) {
            total_size += data.len() as i64;
            file_map.insert("GameUserSettings.ini".into(), hex_encode(&data));
            file_count += 1;
        }
    }

    let webcache_dir = find_webcache_dir(&saved_dir);
    if let Some(ref wc_dir) = webcache_dir {
        if let Ok(data) = std::fs::read(wc_dir.join("Cookies")) {
            total_size += data.len() as i64;
            file_map.insert("Cookies".into(), hex_encode(&data));
            file_count += 1;
        }
        if let Ok(data) = std::fs::read(wc_dir.join("Cookies-journal")) {
            total_size += data.len() as i64;
            file_map.insert("Cookies-journal".into(), hex_encode(&data));
            file_count += 1;
        }
    }

    file_map.insert("__registry_AccountId__".into(), account_id.clone());

    let file_data = serde_json::to_vec(&file_map)
        .map_err(|e| format!("Failed to serialize: {}", e))?;

    // Don't restart Epic for auto-save

    Ok(InternalSyncResult {
        launcher: "epic".into(),
        username: account_id,
        registry_data: None,
        file_data: Some(file_data),
        file_count,
        total_size,
    })
}

fn kill_epic() {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "EpicGamesLauncher.exe"]).output();
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "EpicWebHelper.exe"]).output();
    }
}

fn start_epic() {
    #[cfg(target_os = "windows")]
    {
        if let Some(exe) = crate::switcher::find_epic_exe() {
            let _ = std::process::Command::new(&exe).spawn();
        }
    }
}

fn hex_encode(data: &[u8]) -> String {
    data.iter().map(|b| format!("{:02x}", b)).collect()
}

fn hex_decode(hex: &str) -> Result<Vec<u8>, String> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}
