//! EA Desktop credential sync & restore.
//!
//! Based on TcNo Account Switcher's EA Desktop platform config.
//!
//! The entire %LocalAppData%\Electronic Arts\EA Desktop\ directory is the auth state.
//! Key files:
//!   cookie.ini    — encrypted auth token
//!   user_*.ini    — user preferences with userid
//!   telemetry.ini — correlation ID (unique per-session)
//!   OTT/OTT       — one-time token
//!   CEF/          — Chromium embedded browser cache
//!
//! EA Desktop MUST be killed before reading/writing (ExitBeforeInteract=true in TcNo).
//!
//! On Sync: kill EA → save entire EA Desktop dir → restart EA
//! On Switch: kill EA → clear dir → restore saved dir → clear caches → restart EA

use super::InternalSyncResult;
use std::collections::HashMap;
use std::path::PathBuf;

fn get_ea_dir() -> Result<PathBuf, String> {
    let local_app = std::env::var("LOCALAPPDATA")
        .map_err(|_| "LOCALAPPDATA not set".to_string())?;
    Ok(PathBuf::from(local_app)
        .join("Electronic Arts")
        .join("EA Desktop"))
}

fn get_ea_account_id() -> Option<String> {
    // EA uses telemetry.ini correlationId as unique ID
    // Also check user_*.ini for user.userid
    let ea_dir = get_ea_dir().ok()?;

    // Try user_*.ini for userid
    if let Ok(entries) = std::fs::read_dir(&ea_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("user_") && name.ends_with(".ini") {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    for line in content.lines() {
                        if line.starts_with("user.userid=") {
                            return Some(line.strip_prefix("user.userid=")?.to_string());
                        }
                    }
                }
                // Fallback: use the number from filename user_XXXXX.ini
                let id = name.strip_prefix("user_")?.strip_suffix(".ini")?;
                return Some(id.to_string());
            }
        }
    }
    None
}

/// Recursively collect all files in a directory
fn collect_dir_files(base: &PathBuf, dir: &PathBuf) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    if !dir.exists() { return files; }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return files,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip Logs directory (not needed for auth)
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "Logs" { continue; }
            files.extend(collect_dir_files(base, &path));
        } else if path.is_file() {
            let relative = match path.strip_prefix(base) {
                Ok(r) => r.to_string_lossy().to_string(),
                Err(_) => continue,
            };
            // Skip log files
            if relative.ends_with(".log") { continue; }
            // Skip LOCK files
            if relative.ends_with("LOCK") { continue; }
            match std::fs::read(&path) {
                Ok(data) => files.push((relative, data)),
                Err(e) => log::warn!("[EA] Skipping locked file {:?}: {}", path, e),
            }
        }
    }
    files
}

/// Sync the currently logged-in EA account.
/// Kills EA → saves entire EA Desktop directory → restarts EA.
pub fn sync_current() -> Result<InternalSyncResult, String> {
    sync_current_inner(true)
}

pub fn sync_current_for_auto_save() -> Result<InternalSyncResult, String> {
    sync_current_inner(false)
}

fn sync_current_inner(restart_after: bool) -> Result<InternalSyncResult, String> {
    let account_id = get_ea_account_id()
        .ok_or("No EA account logged in. Open EA App and log in first.")?;

    let ea_dir = get_ea_dir()?;
    if !ea_dir.exists() {
        return Err("EA Desktop directory not found.".into());
    }

    // Kill EA to release file locks
    kill_ea();
    std::thread::sleep(std::time::Duration::from_secs(2));

    // Collect all files
    let all_files = collect_dir_files(&ea_dir, &ea_dir);

    let mut total_size: i64 = 0;
    let mut file_map: HashMap<String, String> = HashMap::new();

    for (relative_path, data) in &all_files {
        total_size += data.len() as i64;
        file_map.insert(relative_path.clone(), hex_encode(data));
    }

    // Save account ID for reference
    file_map.insert("__account_id__".into(), account_id.clone());

    let file_data = serde_json::to_vec(&file_map)
        .map_err(|e| format!("Failed to serialize: {}", e))?;

    let file_count = all_files.len() as i32;

    if restart_after {
        start_ea();
    }

    log::info!("[EA Sync] Saved {} files for account '{}' ({} bytes)",
        file_count, account_id, total_size);

    Ok(InternalSyncResult {
        launcher: "ea".into(),
        username: account_id,
        registry_data: None,
        file_data: Some(file_data),
        file_count,
        total_size,
    })
}

/// Restore saved EA Desktop directory.
pub fn restore_and_switch(account_id: &str, file_data: &[u8]) -> Result<Vec<String>, String> {
    let mut steps = Vec::new();

    let ea_dir = get_ea_dir()?;

    let file_map: HashMap<String, String> = serde_json::from_slice(file_data)
        .map_err(|e| format!("Failed to parse saved data: {}", e))?;

    // Clear current EA Desktop directory (except Logs)
    if ea_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&ea_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name == "Logs" { continue; }
                let path = entry.path();
                if path.is_dir() {
                    let _ = std::fs::remove_dir_all(&path);
                } else {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    } else {
        let _ = std::fs::create_dir_all(&ea_dir);
    }
    steps.push("Cleared EA Desktop directory".into());

    // Restore saved files
    let mut restored = 0;
    for (relative_path, hex_content) in &file_map {
        if relative_path.starts_with("__") { continue; }

        let dest = ea_dir.join(relative_path);
        if let Some(parent) = dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let data = hex_decode(hex_content)?;
        std::fs::write(&dest, &data)
            .map_err(|e| format!("Failed to write {:?}: {}", dest, e))?;
        restored += 1;
    }
    steps.push(format!("Restored {} files", restored));

    // Clear EA caches (TcNo's CachePaths)
    let local_app = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let cache_paths = vec![
        PathBuf::from(&local_app).join("EADesktop").join("QtWebEngine").join("Default").join("GPUCache"),
        PathBuf::from(&local_app).join("EADesktop").join("cache").join("QtWebEngine").join("Default").join("Cache"),
        PathBuf::from(&local_app).join("EADesktop").join("cache").join("qmlcache"),
    ];
    for cache_path in &cache_paths {
        if cache_path.exists() {
            let _ = std::fs::remove_dir_all(cache_path);
        }
    }
    steps.push("Cleared EA caches".into());

    Ok(steps)
}

fn kill_ea() {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "EADesktop.exe"]).output();
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "EABackgroundService.exe"]).output();
    }
}

fn start_ea() {
    #[cfg(target_os = "windows")]
    {
        let exe = r"C:\Program Files\Electronic Arts\EA Desktop\EA Desktop\EADesktop.exe";
        if std::path::Path::new(exe).exists() {
            let _ = std::process::Command::new(exe).spawn();
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
