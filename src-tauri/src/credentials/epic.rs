//! Epic Games credential management.
//!
//! Epic does NOT support multi-account switching (server-side session invalidation).
//! Instead, we support single cafe account mode:
//!
//! 1. Admin syncs the cafe's Epic account (saved permanently)
//! 2. "Wipe Login" clears credentials so gamer can log into personal account
//! 3. "Restore Cafe Account" switches back to the saved cafe account
//!
//! This matches how the PC Client handles session isolation:
//! backup cafe → let gamer login → restore cafe on session end.

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

/// Recursively find all GameUserSettings.ini files
fn find_all_ini_files(dir: &std::path::Path, results: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                find_all_ini_files(&path, results);
            } else if path.file_name().map_or(false, |n| n == "GameUserSettings.ini") {
                results.push(path);
            }
        }
    }
}

/// Sync the currently logged-in Epic account (cafe account).
/// Saves ALL GameUserSettings.ini files + registry AccountId.
pub fn sync_current() -> Result<InternalSyncResult, String> {
    let account_id = get_epic_account_id()
        .ok_or("No Epic Games account logged in.")?;

    log::info!("[Epic Sync] Killing Epic to capture files...");
    kill_epic();
    std::thread::sleep(std::time::Duration::from_secs(2));

    let saved_dir = get_epic_saved_dir()?;
    let config_dir = saved_dir.join("Config");
    let mut file_map: HashMap<String, String> = HashMap::new();
    let mut total_size: i64 = 0;
    let mut file_count: i32 = 0;

    // Save ALL GameUserSettings.ini files in the config hierarchy
    let mut ini_files = Vec::new();
    find_all_ini_files(&config_dir, &mut ini_files);

    for ini_path in &ini_files {
        let data = std::fs::read(ini_path)
            .map_err(|e| format!("Failed to read {:?}: {}", ini_path, e))?;

        let rel = ini_path.strip_prefix(&config_dir)
            .map_err(|e| format!("Path error: {}", e))?
            .to_string_lossy()
            .to_string();

        total_size += data.len() as i64;
        file_map.insert(format!("ini:{}", rel), hex_encode(&data));
        file_count += 1;
        log::info!("[Epic Sync] Saved {} ({} bytes)", rel, data.len());
    }

    if file_count == 0 {
        return Err("No GameUserSettings.ini files found.".into());
    }

    // Validate at least one has a RememberMe token
    let has_token = ini_files.iter().any(|path| {
        std::fs::read_to_string(path)
            .map(|c| c.contains("[RememberMe]") && c.contains("Data="))
            .unwrap_or(false)
    });
    if !has_token {
        return Err("No RememberMe token found. Make sure you're fully logged in.".into());
    }

    // Save registry AccountId
    file_map.insert("__registry_AccountId__".into(), account_id.clone());

    let file_data = serde_json::to_vec(&file_map)
        .map_err(|e| format!("Failed to serialize: {}", e))?;

    // Restart Epic
    start_epic();

    log::info!("[Epic Sync] Saved {} ini files for '{}' ({} bytes)",
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

/// Restore saved cafe account — writes ALL saved GameUserSettings.ini files + registry.
pub fn restore_and_switch(account_id: &str, file_data: &[u8]) -> Result<Vec<String>, String> {
    let mut steps = Vec::new();
    let saved_dir = get_epic_saved_dir()?;
    let config_dir = saved_dir.join("Config");

    let file_map: HashMap<String, String> = serde_json::from_slice(file_data)
        .map_err(|e| format!("Failed to parse saved data: {}", e))?;

    // Restore ALL saved GameUserSettings.ini files
    let mut restored = 0;
    for (key, hex_content) in &file_map {
        if !key.starts_with("ini:") { continue; }
        let rel = key.strip_prefix("ini:").unwrap();
        let dest = config_dir.join(rel);
        if let Some(parent) = dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let data = hex_decode(hex_content)?;
        std::fs::write(&dest, &data)
            .map_err(|e| format!("Failed to write {:?}: {}", dest, e))?;
        restored += 1;
    }
    steps.push(format!("Restored {} GameUserSettings.ini files", restored));

    // Set registry AccountId
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

/// Wipe Epic login — clears ALL GameUserSettings.ini RememberMe tokens + registry.
/// Does NOT call Epic's logout API, so saved cafe credentials remain valid.
pub fn wipe_login_state() -> Result<Vec<String>, String> {
    let mut steps = Vec::new();

    kill_epic();
    std::thread::sleep(std::time::Duration::from_secs(2));
    steps.push("Killed Epic".into());

    let saved_dir = get_epic_saved_dir()?;
    let config_dir = saved_dir.join("Config");

    // Clear RememberMe in ALL GameUserSettings.ini files
    let mut ini_files = Vec::new();
    find_all_ini_files(&config_dir, &mut ini_files);

    for ini_path in &ini_files {
        if let Ok(content) = std::fs::read_to_string(ini_path) {
            let mut output = String::new();
            let mut in_section = false;
            for line in content.lines() {
                if line.trim() == "[RememberMe]" {
                    in_section = true;
                    output.push_str("[RememberMe]\nEnable=False\n");
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
            let _ = std::fs::write(ini_path, output);
        }
    }
    steps.push(format!("Cleared RememberMe in {} ini files", ini_files.len()));

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

    steps.push("Login wiped. Epic will show login screen on next launch.".into());
    Ok(steps)
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
