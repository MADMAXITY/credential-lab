//! Launcher detection — find installed launchers and their current login state.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LauncherInfo {
    pub id: String,
    pub name: String,
    pub is_installed: bool,
    pub install_path: Option<String>,
    pub current_user: Option<String>,
    pub is_running: bool,
    pub remembered_accounts: Vec<String>,
}

/// Detect all supported launchers
#[tauri::command]
pub fn detect_launchers() -> Result<Vec<LauncherInfo>, String> {
    let mut launchers = Vec::new();

    launchers.push(detect_steam());
    launchers.push(detect_epic());
    launchers.push(detect_riot());
    launchers.push(detect_ea());
    launchers.push(detect_ubisoft());
    launchers.push(detect_gog());

    Ok(launchers)
}

/// Get current logged-in user for a specific launcher
#[tauri::command]
pub fn get_launcher_current_user(launcher_id: String) -> Result<Option<String>, String> {
    match launcher_id.as_str() {
        "steam" => Ok(get_steam_current_user()),
        "epic" => Ok(get_epic_current_user()),
        "riot" => Ok(None), // TODO
        "ea" => Ok(None),   // TODO
        _ => Ok(None),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Steam Detection
// ═══════════════════════════════════════════════════════════════════════════════

fn detect_steam() -> LauncherInfo {
    let mut info = LauncherInfo {
        id: "steam".into(),
        name: "Steam".into(),
        is_installed: false,
        install_path: None,
        current_user: None,
        is_running: false,
        remembered_accounts: Vec::new(),
    };

    // Check if Steam is installed via steamlocate
    if let Ok(steam_dir) = steamlocate::SteamDir::locate() {
        info.is_installed = true;
        info.install_path = Some(steam_dir.path().to_string_lossy().to_string());

        // Parse loginusers.vdf — only show accounts with RememberPassword=1
        let vdf_path = steam_dir.path().join("config").join("loginusers.vdf");
        if let Ok(content) = std::fs::read_to_string(&vdf_path) {
            info.remembered_accounts = parse_steam_remembered_accounts(&content);
        }
    }

    // Read current user from registry
    info.current_user = get_steam_current_user();

    // Check if Steam is running
    info.is_running = is_process_running("steam.exe");

    info
}

fn get_steam_current_user() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(key) = hkcu.open_subkey(r"Software\Valve\Steam") {
            if let Ok(user) = key.get_value::<String, _>("AutoLoginUser") {
                if !user.is_empty() {
                    return Some(user);
                }
            }
        }
    }
    None
}

/// Parse loginusers.vdf to extract all account names.
/// If `only_remembered` is true, only returns accounts with RememberPassword=1.
pub fn parse_steam_accounts(content: &str) -> Vec<String> {
    parse_steam_accounts_filtered(content, false)
}

/// Parse loginusers.vdf — only accounts with RememberPassword=1 (switchable)
pub fn parse_steam_remembered_accounts(content: &str) -> Vec<String> {
    parse_steam_accounts_filtered(content, true)
}

fn parse_steam_accounts_filtered(content: &str, only_remembered: bool) -> Vec<String> {
    let mut accounts = Vec::new();
    let mut current_account: Option<String> = None;
    let mut current_remembered = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // SteamID64 line starts a new account block
        if trimmed.starts_with("\"7656") && trimmed.ends_with("\"") {
            // Save previous account if valid
            if let Some(ref name) = current_account {
                if !only_remembered || current_remembered {
                    accounts.push(name.clone());
                }
            }
            current_account = None;
            current_remembered = false;
        }

        if trimmed.starts_with("\"AccountName\"") {
            if let Some(name) = extract_vdf_value(trimmed) {
                current_account = Some(name);
            }
        }

        if trimmed.starts_with("\"RememberPassword\"") {
            if let Some(val) = extract_vdf_value(trimmed) {
                current_remembered = val == "1";
            }
        }
    }

    // Don't forget the last account block
    if let Some(ref name) = current_account {
        if !only_remembered || current_remembered {
            accounts.push(name.clone());
        }
    }

    accounts
}

fn extract_vdf_value(line: &str) -> Option<String> {
    let parts: Vec<&str> = line.split('"').collect();
    if parts.len() >= 4 {
        return Some(parts[3].to_string());
    }
    None
}

// ═══════════════════════════════════════════════════════════════════════════════
// Epic Detection
// ═══════════════════════════════════════════════════════════════════════════════

fn detect_epic() -> LauncherInfo {
    let mut info = LauncherInfo {
        id: "epic".into(),
        name: "Epic Games".into(),
        is_installed: false,
        install_path: None,
        current_user: None,
        is_running: false,
        remembered_accounts: Vec::new(),
    };

    if let Ok(local_app) = std::env::var("LOCALAPPDATA") {
        let epic_dir = std::path::PathBuf::from(&local_app).join("EpicGamesLauncher");
        if epic_dir.exists() {
            info.is_installed = true;
            info.install_path = Some(epic_dir.to_string_lossy().to_string());
        }
    }

    info.current_user = get_epic_current_user();
    info.is_running = is_process_running("EpicGamesLauncher.exe");

    info
}

fn get_epic_current_user() -> Option<String> {
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

// ═══════════════════════════════════════════════════════════════════════════════
// Riot Detection
// ═══════════════════════════════════════════════════════════════════════════════

fn detect_riot() -> LauncherInfo {
    let mut info = LauncherInfo {
        id: "riot".into(),
        name: "Riot Games".into(),
        is_installed: false,
        install_path: None,
        current_user: None,
        is_running: false,
        remembered_accounts: Vec::new(),
    };

    if let Ok(local_app) = std::env::var("LOCALAPPDATA") {
        let riot_dir = std::path::PathBuf::from(&local_app).join("Riot Games");
        if riot_dir.exists() {
            info.is_installed = true;
            info.install_path = Some(riot_dir.to_string_lossy().to_string());
        }
    }

    info.is_running = is_process_running("RiotClientServices.exe")
        || is_process_running("VALORANT.exe");

    info
}

// ═══════════════════════════════════════════════════════════════════════════════
// EA Detection
// ═══════════════════════════════════════════════════════════════════════════════

fn detect_ea() -> LauncherInfo {
    let mut info = LauncherInfo {
        id: "ea".into(),
        name: "EA App".into(),
        is_installed: false,
        install_path: None,
        current_user: None,
        is_running: false,
        remembered_accounts: Vec::new(),
    };

    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if hkcu.open_subkey(r"Software\Electronic Arts\EA Desktop").is_ok() {
            info.is_installed = true;
        }
    }

    info.is_running = is_process_running("EADesktop.exe");
    info
}

// ═══════════════════════════════════════════════════════════════════════════════
// Ubisoft Detection
// ═══════════════════════════════════════════════════════════════════════════════

fn detect_ubisoft() -> LauncherInfo {
    let mut info = LauncherInfo {
        id: "ubisoft".into(),
        name: "Ubisoft Connect".into(),
        is_installed: false,
        install_path: None,
        current_user: None,
        is_running: false,
        remembered_accounts: Vec::new(),
    };

    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        if let Ok(key) = hklm.open_subkey(r"SOFTWARE\WOW6432Node\Ubisoft\Launcher") {
            if let Ok(path) = key.get_value::<String, _>("InstallDir") {
                info.is_installed = true;
                info.install_path = Some(path);
            }
        }
    }

    info.is_running = is_process_running("upc.exe");
    info
}

// ═══════════════════════════════════════════════════════════════════════════════
// GOG Detection
// ═══════════════════════════════════════════════════════════════════════════════

fn detect_gog() -> LauncherInfo {
    let mut info = LauncherInfo {
        id: "gog".into(),
        name: "GOG Galaxy".into(),
        is_installed: false,
        install_path: None,
        current_user: None,
        is_running: false,
        remembered_accounts: Vec::new(),
    };

    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        if let Ok(key) = hklm.open_subkey(r"SOFTWARE\WOW6432Node\GOG.com\GalaxyClient\paths") {
            if let Ok(path) = key.get_value::<String, _>("client") {
                info.is_installed = true;
                info.install_path = Some(path);
            }
        }
    }

    info.is_running = is_process_running("GalaxyClient.exe");
    info
}

// ═══════════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════════

fn is_process_running(process_name: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        if let Ok(output) = Command::new("tasklist")
            .args(["/FI", &format!("IMAGENAME eq {}", process_name), "/NH"])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            return stdout.contains(process_name);
        }
    }
    false
}
