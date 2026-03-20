//! Epic Games credential sync & restore.
//!
//! Based on TcNo Account Switcher's exact approach:
//!
//! LoginFiles (saved per-account, cleared+restored on switch):
//!   - %LocalAppData%\EpicGamesLauncher\Saved\Config\WindowsEditor\GameUserSettings.ini
//!   - REG:HKCU\Software\Epic Games\Unreal Engine\Identifiers:AccountId
//!
//! BackupFolders (saved per-account for completeness):
//!   - %LocalAppData%\EpicGamesLauncher\Saved\Config\  (entire directory)
//!
//! CachePaths (cleared on switch, NOT saved):
//!   - %LocalAppData%\Epic Games\Epic Online Services\UI Helper\Cache\
//!   - %LocalAppData%\Epic Games\EOSOverlay\BrowserCache\Cache\
//!
//! IMPORTANT: Data\ folder (.dat auth tokens) is NOT touched. Epic manages those
//! internally — they persist across account switches. Only GameUserSettings.ini
//! and the registry AccountId determine which account logs in.

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

/// Recursively collect all files in a directory
fn collect_dir_files(base: &PathBuf, dir: &PathBuf) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut files = Vec::new();
    if !dir.exists() {
        return Ok(files);
    }
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("Failed to read dir {:?}: {}", dir, e))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(collect_dir_files(base, &path)?);
        } else if path.is_file() {
            let relative = path.strip_prefix(base)
                .map_err(|e| format!("Path error: {}", e))?
                .to_string_lossy()
                .to_string();
            let data = std::fs::read(&path)
                .map_err(|e| format!("Failed to read {:?}: {}", path, e))?;
            files.push((relative, data));
        }
    }
    Ok(files)
}

/// Sync the currently logged-in Epic account.
/// Saves: entire Config\ folder + registry AccountId.
/// Does NOT save Data\ — those .dat files persist across switches.
pub fn sync_current() -> Result<InternalSyncResult, String> {
    let account_id = get_epic_account_id()
        .ok_or("No Epic Games account logged in. Log in to Epic first.")?;

    let saved_dir = get_epic_saved_dir()?;
    let config_dir = saved_dir.join("Config");

    if !config_dir.exists() {
        return Err("Epic Games config directory not found.".into());
    }

    let mut total_size: i64 = 0;
    let mut file_map: HashMap<String, String> = HashMap::new();

    // Collect Config\ files (includes GameUserSettings.ini + all subdirectories)
    let config_files = collect_dir_files(&config_dir, &config_dir)?;
    for (relative_path, data) in &config_files {
        total_size += data.len() as i64;
        file_map.insert(relative_path.clone(), hex_encode(data));
    }

    // Save the registry AccountId
    file_map.insert("__registry_AccountId__".into(), account_id.clone());

    let file_data = serde_json::to_vec(&file_map)
        .map_err(|e| format!("Failed to serialize: {}", e))?;

    let file_count = config_files.len() as i32;

    log::info!("[Epic Sync] Saved {} config files for account '{}' ({} bytes)",
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

/// Restore saved Epic config files + registry, clear caches.
/// Does NOT touch Data\ folder — Epic manages auth tokens there internally.
pub fn restore_and_switch(account_id: &str, file_data: &[u8]) -> Result<Vec<String>, String> {
    let mut steps = Vec::new();

    let saved_dir = get_epic_saved_dir()?;
    let config_dir = saved_dir.join("Config");

    // Parse saved files
    let file_map: HashMap<String, String> = serde_json::from_slice(file_data)
        .map_err(|e| format!("Failed to parse saved data: {}", e))?;

    // Clear Epic caches (TcNo's CachePaths)
    let local_app = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let cache_paths = vec![
        PathBuf::from(&local_app).join("Epic Games").join("Epic Online Services").join("UI Helper").join("Cache"),
        PathBuf::from(&local_app).join("Epic Games").join("EOSOverlay").join("BrowserCache").join("Cache"),
    ];
    for cache_path in &cache_paths {
        if cache_path.exists() {
            let _ = std::fs::remove_dir_all(cache_path);
        }
    }
    steps.push("Cleared Epic cache directories".into());

    // Clear Config\ directory only (NOT Data\)
    if config_dir.exists() {
        let _ = std::fs::remove_dir_all(&config_dir);
    }
    let _ = std::fs::create_dir_all(&config_dir);
    steps.push("Cleared Config directory".into());

    // Restore saved config files
    let mut restored = 0;
    for (relative_path, hex_content) in &file_map {
        if relative_path == "__registry_AccountId__" {
            continue;
        }

        // Config files go into Config\ directory
        let dest = config_dir.join(relative_path);
        if let Some(parent) = dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let data = hex_decode(hex_content)?;
        std::fs::write(&dest, &data)
            .map_err(|e| format!("Failed to write {:?}: {}", dest, e))?;
        restored += 1;
    }
    steps.push(format!("Restored {} config files", restored));

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

fn hex_encode(data: &[u8]) -> String {
    data.iter().map(|b| format!("{:02x}", b)).collect()
}

fn hex_decode(hex: &str) -> Result<Vec<u8>, String> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}
