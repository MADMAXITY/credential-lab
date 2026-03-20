//! Epic Games credential sync & restore.
//!
//! On Sync: save Config\ folder + Data\ folder (auth tokens) + registry AccountId.
//! On Switch: kill Epic → clear caches → restore saved files + registry → restart.
//!
//! Key paths:
//!   Config:  %LocalAppData%\EpicGamesLauncher\Saved\Config\
//!   Data:    %LocalAppData%\EpicGamesLauncher\Saved\Data\  (auth token .dat files)
//!   Registry: HKCU\Software\Epic Games\Unreal Engine\Identifiers:AccountId
//!   Cache (cleared on switch):
//!     %LocalAppData%\Epic Games\Epic Online Services\UI Helper\Cache\
//!     %LocalAppData%\Epic Games\EOSOverlay\BrowserCache\Cache\

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

/// Recursively collect all files in a directory, returning (relative_path, contents)
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
/// Saves: Config\ folder + Data\ folder + registry AccountId.
pub fn sync_current() -> Result<InternalSyncResult, String> {
    let account_id = get_epic_account_id()
        .ok_or("No Epic Games account logged in. Log in to Epic first.")?;

    let saved_dir = get_epic_saved_dir()?;
    let config_dir = saved_dir.join("Config");
    let data_dir = saved_dir.join("Data");

    if !config_dir.exists() {
        return Err("Epic Games config directory not found.".into());
    }

    let mut total_size: i64 = 0;
    let mut file_map: HashMap<String, String> = HashMap::new();

    // Collect Config\ files
    let config_files = collect_dir_files(&saved_dir, &config_dir)?;
    for (relative_path, data) in &config_files {
        total_size += data.len() as i64;
        file_map.insert(format!("Config/{}", relative_path), hex_encode(data));
    }

    // Collect Data\ files (auth tokens — .dat files named by account ID)
    if data_dir.exists() {
        let data_files = collect_dir_files(&saved_dir, &data_dir)?;
        for (relative_path, data) in &data_files {
            total_size += data.len() as i64;
            file_map.insert(format!("Data/{}", relative_path), hex_encode(data));
        }
        log::info!("[Epic Sync] Saved {} Data files (auth tokens)", data_files.len());
    }

    // Save the registry AccountId
    file_map.insert("__registry_AccountId__".into(), account_id.clone());

    let file_data = serde_json::to_vec(&file_map)
        .map_err(|e| format!("Failed to serialize: {}", e))?;

    let file_count = (config_files.len() + file_map.keys().filter(|k| k.starts_with("Data/")).count()) as i32;

    log::info!("[Epic Sync] Saved {} files for account '{}' ({} bytes)",
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

/// Restore saved Epic files + registry, clear caches.
pub fn restore_and_switch(account_id: &str, file_data: &[u8]) -> Result<Vec<String>, String> {
    let mut steps = Vec::new();

    let saved_dir = get_epic_saved_dir()?;
    let config_dir = saved_dir.join("Config");
    let data_dir = saved_dir.join("Data");

    // Parse saved files
    let file_map: HashMap<String, String> = serde_json::from_slice(file_data)
        .map_err(|e| format!("Failed to parse saved data: {}", e))?;

    // Clear Epic caches
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

    // Delete current Config\ contents before restoring
    if config_dir.exists() {
        let _ = std::fs::remove_dir_all(&config_dir);
    }
    let _ = std::fs::create_dir_all(&config_dir);

    // Delete current Data\ contents before restoring
    if data_dir.exists() {
        let _ = std::fs::remove_dir_all(&data_dir);
    }
    let _ = std::fs::create_dir_all(&data_dir);
    steps.push("Cleared current Epic config + data".into());

    // Restore saved files
    let mut config_restored = 0;
    let mut data_restored = 0;
    for (relative_path, hex_content) in &file_map {
        if relative_path == "__registry_AccountId__" {
            continue;
        }

        let dest = saved_dir.join(relative_path);
        if let Some(parent) = dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let data = hex_decode(hex_content)?;
        std::fs::write(&dest, &data)
            .map_err(|e| format!("Failed to write {:?}: {}", dest, e))?;

        if relative_path.starts_with("Data/") {
            data_restored += 1;
        } else {
            config_restored += 1;
        }
    }
    steps.push(format!("Restored {} config files + {} data files (auth tokens)", config_restored, data_restored));

    // Restore registry AccountId
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
