//! Epic Games credential sync & restore.
//!
//! Key discovery: Epic's GameUserSettings.ini [RememberMe] section holds the auth
//! token, but only for ONE account at a time. Each login overwrites the previous.
//!
//! Solution: save the [RememberMe] Data= token per account. On switch, merge ALL
//! saved tokens back into the file, set registry AccountId, delete webcache.
//! Epic picks the right token based on AccountId.
//!
//! Files:
//!   Config: %LocalAppData%\EpicGamesLauncher\Saved\Config\
//!   Registry: HKCU\Software\Epic Games\Unreal Engine\Identifiers:AccountId
//!   Webcache: deleted on switch (rebuilt by Epic from [RememberMe] tokens)

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

fn get_game_user_settings_path() -> Result<PathBuf, String> {
    let saved = get_epic_saved_dir()?;
    // Try WindowsEditor first, then Windows
    let wepath = saved.join("Config").join("WindowsEditor").join("GameUserSettings.ini");
    if wepath.exists() { return Ok(wepath); }
    let wpath = saved.join("Config").join("Windows").join("GameUserSettings.ini");
    if wpath.exists() { return Ok(wpath); }
    Err("GameUserSettings.ini not found".into())
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

/// Extract [RememberMe] Data= token from GameUserSettings.ini
fn extract_remember_me_token(ini_path: &PathBuf) -> Option<String> {
    let content = std::fs::read_to_string(ini_path).ok()?;
    let mut in_section = false;
    let mut token_lines = Vec::new();

    for line in content.lines() {
        if line.trim() == "[RememberMe]" {
            in_section = true;
            continue;
        }
        if in_section && line.starts_with('[') {
            break;
        }
        if in_section && line.starts_with("Data=") {
            token_lines.push(line.to_string());
        }
    }

    if token_lines.is_empty() { None } else { Some(token_lines.join("\n")) }
}

/// Replace [RememberMe] section in GameUserSettings.ini with merged tokens
fn write_remember_me_tokens(ini_path: &PathBuf, tokens: &[String]) -> Result<(), String> {
    let content = std::fs::read_to_string(ini_path)
        .map_err(|e| format!("Failed to read ini: {}", e))?;

    let mut output = String::new();
    let mut in_section = false;
    let mut section_written = false;

    for line in content.lines() {
        if line.trim() == "[RememberMe]" {
            in_section = true;
            output.push_str("[RememberMe]\n");
            output.push_str("Enable=True\n");
            for token in tokens {
                for token_line in token.lines() {
                    output.push_str(token_line);
                    output.push('\n');
                }
            }
            section_written = true;
            continue;
        }
        if in_section {
            if line.starts_with('[') {
                // End of RememberMe section
                in_section = false;
                output.push_str(line);
                output.push('\n');
            }
            // Skip old RememberMe content (already replaced above)
            continue;
        }
        output.push_str(line);
        output.push('\n');
    }

    // If [RememberMe] section didn't exist, add it
    if !section_written {
        output.push_str("\n[RememberMe]\n");
        output.push_str("Enable=True\n");
        for token in tokens {
            for token_line in token.lines() {
                output.push_str(token_line);
                output.push('\n');
            }
        }
    }

    std::fs::write(ini_path, &output)
        .map_err(|e| format!("Failed to write ini: {}", e))
}

/// Sync the currently logged-in Epic account.
/// Kills Epic, saves the [RememberMe] token + registry AccountId.
pub fn sync_current() -> Result<InternalSyncResult, String> {
    sync_current_inner(true)
}

pub fn sync_current_for_auto_save() -> Result<InternalSyncResult, String> {
    sync_current_inner(false)
}

fn sync_current_inner(restart_after: bool) -> Result<InternalSyncResult, String> {
    let account_id = get_epic_account_id()
        .ok_or("No Epic Games account logged in.")?;

    // Kill Epic to release file locks
    kill_epic();
    std::thread::sleep(std::time::Duration::from_secs(2));

    let ini_path = get_game_user_settings_path()?;

    // Extract the [RememberMe] Data= token
    let token = extract_remember_me_token(&ini_path)
        .ok_or("No [RememberMe] token found in GameUserSettings.ini. Make sure you're logged in.")?;

    // Save: token + account ID
    let mut file_map: HashMap<String, String> = HashMap::new();
    file_map.insert("__remember_me_token__".into(), token.clone());
    file_map.insert("__registry_AccountId__".into(), account_id.clone());

    // Also save the full GameUserSettings.ini (for other settings)
    let ini_data = std::fs::read(&ini_path)
        .map_err(|e| format!("Failed to read ini: {}", e))?;
    file_map.insert("__game_user_settings__".into(), hex_encode(&ini_data));

    let file_data = serde_json::to_vec(&file_map)
        .map_err(|e| format!("Failed to serialize: {}", e))?;

    let total_size = token.len() as i64 + ini_data.len() as i64;

    if restart_after {
        #[cfg(target_os = "windows")]
        {
            if let Some(exe) = crate::switcher::find_epic_exe() {
                let _ = std::process::Command::new(&exe).spawn();
            }
        }
    }

    log::info!("[Epic Sync] Saved RememberMe token for '{}'", &account_id[..8.min(account_id.len())]);

    Ok(InternalSyncResult {
        launcher: "epic".into(),
        username: account_id,
        registry_data: None,
        file_data: Some(file_data),
        file_count: 1,
        total_size,
    })
}

/// Restore: merge ALL saved accounts' [RememberMe] tokens into GameUserSettings.ini,
/// set registry AccountId, delete webcache.
pub fn restore_and_switch(account_id: &str, file_data: &[u8]) -> Result<Vec<String>, String> {
    let mut steps = Vec::new();

    let saved_dir = get_epic_saved_dir()?;

    let file_map: HashMap<String, String> = serde_json::from_slice(file_data)
        .map_err(|e| format!("Failed to parse saved data: {}", e))?;

    // Get the target account's RememberMe token
    let target_token = file_map.get("__remember_me_token__")
        .ok_or("No RememberMe token in saved data")?;

    // Restore the GameUserSettings.ini base file
    let ini_path = get_game_user_settings_path()
        .or_else(|_| {
            // If file doesn't exist, create the directory and use default path
            let default = saved_dir.join("Config").join("WindowsEditor");
            let _ = std::fs::create_dir_all(&default);
            Ok::<PathBuf, String>(default.join("GameUserSettings.ini"))
        })?;

    if let Some(ini_hex) = file_map.get("__game_user_settings__") {
        let ini_data = hex_decode(ini_hex)?;
        std::fs::write(&ini_path, &ini_data)
            .map_err(|e| format!("Failed to write GameUserSettings.ini: {}", e))?;
        steps.push("Restored GameUserSettings.ini".into());
    }

    // Write the target account's [RememberMe] token into the ini
    write_remember_me_tokens(&ini_path, &[target_token.clone()])?;
    steps.push(format!("Set [RememberMe] token for {}", &account_id[..8.min(account_id.len())]));

    // Delete webcache — force Epic to rebuild from [RememberMe] token
    let webcache_dir = find_webcache_dir(&saved_dir);
    if let Some(ref wc_dir) = webcache_dir {
        let _ = std::fs::remove_dir_all(wc_dir);
        steps.push("Cleared webcache".into());
    }

    // Clear EOS caches
    let local_app = std::env::var("LOCALAPPDATA").unwrap_or_default();
    for sub in &["Epic Games/Epic Online Services/UI Helper/Cache", "Epic Games/EOSOverlay/BrowserCache/Cache"] {
        let p = PathBuf::from(&local_app).join(sub);
        if p.exists() { let _ = std::fs::remove_dir_all(&p); }
    }

    // Set registry AccountId
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok((key, _)) = hkcu.create_subkey(r"Software\Epic Games\Unreal Engine\Identifiers") {
            key.set_value("AccountId", &account_id)
                .map_err(|e| format!("Failed to set AccountId: {}", e))?;
            steps.push(format!("Set registry AccountId = {}", &account_id[..8.min(account_id.len())]));
        }
    }

    Ok(steps)
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

fn kill_epic() {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "EpicGamesLauncher.exe"]).output();
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "EpicWebHelper.exe"]).output();
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
