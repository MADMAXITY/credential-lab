//! Game detection — scan installed games per launcher.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DetectedGame {
    pub game_id: String,
    pub name: String,
    pub launcher: String,
    pub install_path: Option<String>,
}

/// Detect installed games for a specific launcher
#[tauri::command]
pub fn detect_games(launcher_id: String) -> Result<Vec<DetectedGame>, String> {
    match launcher_id.as_str() {
        "steam" => detect_steam_games(),
        "epic" => detect_epic_games(),
        _ => Ok(Vec::new()),
    }
}

fn detect_steam_games() -> Result<Vec<DetectedGame>, String> {
    let mut games: Vec<DetectedGame> = Vec::new();

    let steam_dir = steamlocate::SteamDir::locate()
        .map_err(|_| "Steam not found".to_string())?;
    let steam_path = steam_dir.path();

    // Scan steamapps directories for appmanifest files
    let steamapps = steam_path.join("steamapps");
    scan_steamapps_dir(&steamapps, &mut games);

    // Also check library folders via libraryfolders.vdf
    let lib_vdf = steamapps.join("libraryfolders.vdf");
    if let Ok(content) = std::fs::read_to_string(&lib_vdf) {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("\"path\"") {
                if let Some(path) = extract_vdf_value(trimmed) {
                    let lib_apps = PathBuf::from(&path).join("steamapps");
                    if lib_apps.exists() && lib_apps != steamapps {
                        scan_steamapps_dir(&lib_apps, &mut games);
                    }
                }
            }
        }
    }

    games.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(games)
}

fn scan_steamapps_dir(dir: &PathBuf, games: &mut Vec<DetectedGame>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with("appmanifest_") && name.ends_with(".acf") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let app_id = name.replace("appmanifest_", "").replace(".acf", "");
                    let game_name = extract_acf_field(&content, "name")
                        .unwrap_or_else(|| format!("App {}", app_id));
                    let install_dir = extract_acf_field(&content, "installdir");

                    // Skip tools and redistributables
                    let state = extract_acf_field(&content, "StateFlags").unwrap_or_default();
                    if state == "0" {
                        continue; // Not installed
                    }

                    games.push(DetectedGame {
                        game_id: format!("steam_{}", app_id),
                        name: game_name,
                        launcher: "steam".into(),
                        install_path: install_dir.map(|d| dir.join("common").join(d).to_string_lossy().to_string()),
                    });
                }
            }
        }
    }
}

fn extract_acf_field(content: &str, field: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(&format!("\"{}\"", field)) {
            return extract_vdf_value(trimmed);
        }
    }
    None
}

fn extract_vdf_value(line: &str) -> Option<String> {
    let parts: Vec<&str> = line.split('"').collect();
    if parts.len() >= 4 {
        Some(parts[3].to_string())
    } else {
        None
    }
}

fn detect_epic_games() -> Result<Vec<DetectedGame>, String> {
    let mut games = Vec::new();

    let program_data = std::env::var("PROGRAMDATA")
        .map_err(|_| "PROGRAMDATA not set".to_string())?;

    let manifests_dir = PathBuf::from(&program_data)
        .join("Epic")
        .join("EpicGamesLauncher")
        .join("Data")
        .join("Manifests");

    if !manifests_dir.exists() {
        return Ok(games);
    }

    if let Ok(entries) = std::fs::read_dir(&manifests_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "item") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&content) {
                        let name = manifest.get("DisplayName")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown");
                        let app_name = manifest.get("AppName")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let install_loc = manifest.get("InstallLocation")
                            .and_then(|v| v.as_str());

                        games.push(DetectedGame {
                            game_id: format!("epic_{}", app_name),
                            name: name.to_string(),
                            launcher: "epic".into(),
                            install_path: install_loc.map(|s| s.to_string()),
                        });
                    }
                }
            }
        }
    }

    games.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(games)
}
