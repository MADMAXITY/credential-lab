//! Steam credential sync & restore.
//!
//! On Sync: save config.vdf + loginusers.vdf snapshot for the current account.
//! On Switch: restore saved config.vdf + loginusers.vdf, patch MostRecent, set registry.

use super::InternalSyncResult;
use std::collections::HashMap;

/// Sync the currently logged-in Steam account.
/// Saves: config.vdf + loginusers.vdf as they are right now (with valid auth tokens for this account).
pub fn sync_current() -> Result<InternalSyncResult, String> {
    let steam_dir = steamlocate::SteamDir::locate()
        .map_err(|_| "Steam not installed".to_string())?;
    let steam_path = steam_dir.path();

    // Read current user from registry
    let username = read_auto_login_user()?
        .ok_or("No Steam account is currently logged in.")?;

    // Verify Steam is actually logged in (not on login screen)
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(key) = hkcu.open_subkey(r"Software\Valve\Steam\ActiveProcess") {
            let active_user: u32 = key.get_value("ActiveUser").unwrap_or(0);
            if active_user == 0 {
                return Err("Steam is on the login screen. Log in first, then sync.".into());
            }
        }
    }

    // Save config.vdf + loginusers.vdf
    let mut files: Vec<(&str, Vec<u8>)> = vec![];
    let mut total_size: i64 = 0;

    let config_path = steam_path.join("config").join("config.vdf");
    if config_path.exists() {
        let data = std::fs::read(&config_path)
            .map_err(|e| format!("Failed to read config.vdf: {}", e))?;
        total_size += data.len() as i64;
        files.push(("config.vdf", data));
    }

    let vdf_path = steam_path.join("config").join("loginusers.vdf");
    if vdf_path.exists() {
        let data = std::fs::read(&vdf_path)
            .map_err(|e| format!("Failed to read loginusers.vdf: {}", e))?;
        total_size += data.len() as i64;
        files.push(("loginusers.vdf", data));
    }

    let file_count = files.len() as i32;

    // Package as JSON: filename → hex content
    let file_map: HashMap<String, String> = files.iter()
        .map(|(name, data)| (name.to_string(), hex_encode(data)))
        .collect();
    let file_data = serde_json::to_vec(&file_map)
        .map_err(|e| format!("Failed to serialize files: {}", e))?;

    log::info!("[Steam Sync] Saved config.vdf + loginusers.vdf for '{}' ({} bytes)", username, total_size);

    Ok(InternalSyncResult {
        launcher: "steam".into(),
        username,
        registry_data: None,
        file_data: Some(file_data),
        file_count,
        total_size,
    })
}

/// Restore saved Steam files for an account, patch VDF, set registry.
/// Called by the switcher.
pub fn restore_and_switch(username: &str, file_data: &[u8]) -> Result<Vec<String>, String> {
    let mut steps = Vec::new();

    let steam_dir = steamlocate::SteamDir::locate()
        .map_err(|_| "Steam not installed".to_string())?;
    let steam_path = steam_dir.path();

    // Parse saved files
    let file_map: HashMap<String, String> = serde_json::from_slice(file_data)
        .map_err(|e| format!("Failed to parse saved file data: {}", e))?;

    // Restore config.vdf (contains Authentication JWT tokens)
    if let Some(config_hex) = file_map.get("config.vdf") {
        let config_data = hex_decode(config_hex)?;
        let dest = steam_path.join("config").join("config.vdf");
        std::fs::write(&dest, &config_data)
            .map_err(|e| format!("Failed to write config.vdf: {}", e))?;
        steps.push("Restored config.vdf (auth tokens)".into());
    }

    // Restore loginusers.vdf
    if let Some(vdf_hex) = file_map.get("loginusers.vdf") {
        let vdf_data = hex_decode(vdf_hex)?;
        let dest = steam_path.join("config").join("loginusers.vdf");
        std::fs::write(&dest, &vdf_data)
            .map_err(|e| format!("Failed to write loginusers.vdf: {}", e))?;
        steps.push("Restored loginusers.vdf".into());

        // Patch MostRecent=1 for target account
        patch_vdf_most_recent(&dest, username)?;
        steps.push(format!("Set MostRecent=1 for '{}'", username));
    }

    // Set registry
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(r"Software\Valve\Steam")
            .map_err(|e| format!("Failed to open Steam registry: {}", e))?;
        key.set_value("AutoLoginUser", &username)
            .map_err(|e| format!("Failed to set AutoLoginUser: {}", e))?;
        key.set_value("RememberPassword", &1u32)
            .map_err(|e| format!("Failed to set RememberPassword: {}", e))?;
        steps.push(format!("Set registry AutoLoginUser = {}", username));
    }

    Ok(steps)
}

// ═══════════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════════

fn read_auto_login_user() -> Result<Option<String>, String> {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(key) = hkcu.open_subkey(r"Software\Valve\Steam") {
            if let Ok(user) = key.get_value::<String, _>("AutoLoginUser") {
                if !user.is_empty() {
                    return Ok(Some(user));
                }
            }
        }
        Ok(None)
    }
    #[cfg(not(target_os = "windows"))]
    { Ok(None) }
}

fn patch_vdf_most_recent(vdf_path: &std::path::Path, target_username: &str) -> Result<(), String> {
    let content = std::fs::read_to_string(vdf_path)
        .map_err(|e| format!("Failed to read VDF: {}", e))?;

    let mut output = String::new();
    let mut current_is_target = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("\"7656") {
            current_is_target = false;
        }

        if trimmed.starts_with("\"AccountName\"") {
            if let Some(name) = extract_vdf_value(trimmed) {
                current_is_target = name.eq_ignore_ascii_case(target_username);
            }
        }

        if trimmed.starts_with("\"MostRecent\"") || trimmed.starts_with("\"mostrecent\"") {
            let val = if current_is_target { "1" } else { "0" };
            output.push_str(&format!("\t\t\"MostRecent\"\t\t\"{}\"", val));
            output.push('\n');
            continue;
        }

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
    if parts.len() >= 4 { Some(parts[3].to_string()) } else { None }
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
