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
            // Skip LOCK files (LevelDB locks) — they're not needed for auth
            if relative.ends_with("LOCK") {
                continue;
            }
            match std::fs::read(&path) {
                Ok(data) => files.push((relative, data)),
                Err(e) => {
                    // Skip locked files instead of failing
                    log::warn!("[Epic] Skipping locked file {:?}: {}", path, e);
                }
            }
        }
    }
    Ok(files)
}

/// Sync the currently logged-in Epic account.
/// Saves: Config\ folder + webcache (Cookies, Local Storage, Session Storage) + registry.
/// Kills Epic first to release file locks on webcache.
pub fn sync_current() -> Result<InternalSyncResult, String> {
    sync_current_inner(true)
}

/// Auto-sync variant: kills Epic, reads files. Called before switch.
pub fn sync_current_for_auto_save() -> Result<InternalSyncResult, String> {
    sync_current_inner(false) // Don't restart after auto-save (switch will restart later)
}

fn sync_current_inner(restart_after: bool) -> Result<InternalSyncResult, String> {
    let account_id = get_epic_account_id()
        .ok_or("No Epic Games account logged in. Log in to Epic first.")?;

    // Kill Epic to release file locks on webcache (Chromium locks Cookies/LevelDB)
    kill_epic();
    std::thread::sleep(std::time::Duration::from_secs(2));

    let saved_dir = get_epic_saved_dir()?;
    let config_dir = saved_dir.join("Config");

    if !config_dir.exists() {
        return Err("Epic Games config directory not found.".into());
    }

    let mut total_size: i64 = 0;
    let mut file_map: HashMap<String, String> = HashMap::new();
    let mut file_count = 0;

    // 1. Collect Config\ files
    let config_files = collect_dir_files(&config_dir, &config_dir)?;
    for (relative_path, data) in &config_files {
        total_size += data.len() as i64;
        file_map.insert(format!("__config__/{}", relative_path), hex_encode(data));
        file_count += 1;
    }

    // 2. Save registry AccountId
    // Note: webcache is NOT saved — it's deleted on switch and rebuilt by Epic
    // from [RememberMe] tokens in GameUserSettings.ini
    file_map.insert("__registry_AccountId__".into(), account_id.clone());

    let file_data = serde_json::to_vec(&file_map)
        .map_err(|e| format!("Failed to serialize: {}", e))?;

    // Restart Epic after manual sync (not after auto-save — switch handles restart)
    if restart_after {
        #[cfg(target_os = "windows")]
        {
            if let Some(exe) = crate::switcher::find_epic_exe() {
                let _ = std::process::Command::new(&exe).spawn();
                log::info!("[Epic Sync] Restarted Epic Games Launcher");
            }
        }
    }

    log::info!("[Epic Sync] Saved {} files for account '{}' ({} bytes)",
        file_count, &account_id[..8.min(account_id.len())], total_size);

    Ok(InternalSyncResult {
        launcher: "epic".into(),
        username: account_id,
        registry_data: None,
        file_data: Some(file_data),
        file_count: file_count as i32,
        total_size,
    })
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

/// Restore saved Epic config + webcache auth files + registry.
pub fn restore_and_switch(account_id: &str, file_data: &[u8]) -> Result<Vec<String>, String> {
    let mut steps = Vec::new();

    let saved_dir = get_epic_saved_dir()?;
    let config_dir = saved_dir.join("Config");

    let file_map: HashMap<String, String> = serde_json::from_slice(file_data)
        .map_err(|e| format!("Failed to parse saved data: {}", e))?;

    // Clear EOS caches (not webcache — we restore that)
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
    steps.push("Cleared EOS cache".into());

    // Clear and restore Config\
    if config_dir.exists() {
        let _ = std::fs::remove_dir_all(&config_dir);
    }
    let _ = std::fs::create_dir_all(&config_dir);

    let mut config_restored = 0;
    for (key, hex_content) in &file_map {
        if !key.starts_with("__config__/") { continue; }
        let relative = key.strip_prefix("__config__/").unwrap();
        let dest = config_dir.join(relative);
        if let Some(parent) = dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let data = hex_decode(hex_content)?;
        std::fs::write(&dest, &data)
            .map_err(|e| format!("Failed to write {:?}: {}", dest, e))?;
        config_restored += 1;
    }
    steps.push(format!("Restored {} config files", config_restored));

    // Delete webcache entirely — force Epic to rebuild from GameUserSettings.ini [RememberMe] tokens.
    // GameUserSettings.ini has auth tokens for ALL remembered accounts.
    // The webcache Cookies only caches one session and causes stale-session issues.
    let webcache_dir = find_webcache_dir(&saved_dir);
    if let Some(ref wc_dir) = webcache_dir {
        let _ = std::fs::remove_dir_all(wc_dir);
        steps.push("Cleared webcache (will rebuild from RememberMe tokens)".into());
    }

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

fn kill_epic() {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "EpicGamesLauncher.exe"])
            .output();
        // Also kill EpicWebHelper which locks webcache files
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "EpicWebHelper.exe"])
            .output();
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
