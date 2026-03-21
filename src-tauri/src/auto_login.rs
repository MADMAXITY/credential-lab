//! Auto-login via Windows UI Automation.
//!
//! Uses the Windows UIAutomation COM API to find and fill login forms
//! in game launchers (Epic, EA). This is the password-based approach
//! that iCafeCloud/ggLeap likely use — a real login every time,
//! avoiding the token invalidation problem of file swapping.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct AutoLoginResult {
    pub success: bool,
    pub steps: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UiElement {
    pub name: String,
    pub control_type: String,
    pub automation_id: String,
    pub class_name: String,
    pub is_enabled: bool,
}

/// Scan a window for all UI elements (for debugging/discovery)
#[tauri::command]
pub async fn scan_window_elements(window_title: String) -> Result<Vec<UiElement>, String> {
    #[cfg(target_os = "windows")]
    {
        scan_elements_windows(&window_title)
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("UI Automation only available on Windows".into())
    }
}

/// Attempt to auto-login to Epic Games Launcher
#[tauri::command]
pub async fn auto_login_epic(username: String, password: String) -> Result<AutoLoginResult, String> {
    #[cfg(target_os = "windows")]
    {
        auto_login_epic_windows(&username, &password).await
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("Auto-login only available on Windows".into())
    }
}

#[cfg(target_os = "windows")]
fn scan_elements_windows(window_title: &str) -> Result<Vec<UiElement>, String> {
    use windows::Win32::System::Com::*;
    use windows::Win32::UI::Accessibility::*;
    use windows::core::*;

    unsafe {
        // Initialize COM
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|e| format!("COM init failed: {}", e))?;

        let automation: IUIAutomation = CoCreateInstance(
            &CUIAutomation,
            None,
            CLSCTX_INPROC_SERVER,
        ).map_err(|e| format!("Failed to create UIAutomation: {}", e))?;

        let root = automation.GetRootElement()
            .map_err(|e| format!("Failed to get root: {}", e))?;

        // Find the target window
        let name_variant = VARIANT::from(BSTR::from(window_title));
        let condition = automation.CreatePropertyCondition(
            UIA_NamePropertyId,
            &name_variant,
        ).map_err(|e| format!("Failed to create condition: {}", e))?;

        let window = root.FindFirst(TreeScope_Children, &condition);

        let window = match window {
            Ok(w) => w,
            Err(_) => {
                // Try partial match — search all windows
                let true_condition = automation.CreateTrueCondition()
                    .map_err(|e| format!("Failed to create true condition: {}", e))?;
                let all_windows = root.FindAll(TreeScope_Children, &true_condition)
                    .map_err(|e| format!("Failed to find windows: {}", e))?;

                let count = all_windows.Length().unwrap_or(0);
                let mut found = None;
                for i in 0..count {
                    if let Ok(elem) = all_windows.GetElement(i) {
                        if let Ok(name) = elem.CurrentName() {
                            let name_str = name.to_string();
                            if name_str.to_lowercase().contains(&window_title.to_lowercase()) {
                                found = Some(elem);
                                break;
                            }
                        }
                    }
                }
                found.ok_or(format!("Window '{}' not found", window_title))?
            }
        };

        // Get all descendant elements
        let true_condition = automation.CreateTrueCondition()
            .map_err(|e| format!("Failed to create condition: {}", e))?;
        let elements = window.FindAll(TreeScope_Descendants, &true_condition)
            .map_err(|e| format!("Failed to find elements: {}", e))?;

        let count = elements.Length().unwrap_or(0);
        let mut result = Vec::new();

        for i in 0..count {
            if let Ok(elem) = elements.GetElement(i) {
                let name = elem.CurrentName().unwrap_or_default().to_string();
                let automation_id = elem.CurrentAutomationId().unwrap_or_default().to_string();
                let class_name = elem.CurrentClassName().unwrap_or_default().to_string();
                let control_type = elem.CurrentControlType().unwrap_or_default();
                let control_type_id = control_type.0;
                let is_enabled = elem.CurrentIsEnabled().unwrap_or_default().as_bool();

                let control_type_name = match control_type_id {
                    50000 => "Button",
                    50004 => "Edit",
                    50020 => "Text",
                    50025 => "Document",
                    50026 => "Pane",
                    50033 => "Window",
                    _ => "Other",
                };

                // Only include interesting elements (not empty Panes)
                if !name.is_empty() || !automation_id.is_empty() || control_type_id == 50004 {
                    result.push(UiElement {
                        name,
                        control_type: format!("{} ({})", control_type_name, control_type_id),
                        automation_id,
                        class_name,
                        is_enabled,
                    });
                }
            }
        }

        CoUninitialize();
        Ok(result)
    }
}

#[cfg(target_os = "windows")]
async fn auto_login_epic_windows(username: &str, password: &str) -> Result<AutoLoginResult, String> {
    use windows::Win32::System::Com::*;
    use windows::Win32::UI::Accessibility::*;
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::Win32::Foundation::*;
    use windows::core::*;

    let mut steps = Vec::new();

    // Step 1: Kill Epic
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/IM", "EpicGamesLauncher.exe"]).output();
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/IM", "EpicWebHelper.exe"]).output();
    steps.push("Killed Epic processes".into());
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Clear any saved login state so Epic shows the login form
    let local_app = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let webcache_dir = std::path::PathBuf::from(&local_app)
        .join("EpicGamesLauncher")
        .join("Saved");
    // Find and delete webcache
    if let Ok(entries) = std::fs::read_dir(&webcache_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("webcache") {
                let _ = std::fs::remove_dir_all(entry.path());
                steps.push(format!("Cleared {}", name));
            }
        }
    }
    // Clear the GameUserSettings.ini RememberMe
    let ini_paths = vec![
        webcache_dir.join("Config").join("WindowsEditor").join("GameUserSettings.ini"),
        webcache_dir.join("Config").join("Config").join("WindowsEditor").join("GameUserSettings.ini"),
        webcache_dir.join("Config").join("Config").join("Windows").join("GameUserSettings.ini"),
    ];
    for ini_path in &ini_paths {
        if ini_path.exists() {
            if let Ok(content) = std::fs::read_to_string(ini_path) {
                // Remove RememberMe data so Epic shows login
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
                let _ = std::fs::write(ini_path, output);
            }
        }
    }
    steps.push("Cleared RememberMe tokens".into());

    // Step 2: Start Epic
    let epic_exe = crate::switcher::find_epic_exe()
        .ok_or("Epic Games Launcher not found")?;
    std::process::Command::new(&epic_exe)
        .spawn()
        .map_err(|e| format!("Failed to start Epic: {}", e))?;
    steps.push("Started Epic Games Launcher".into());

    // Step 3: Wait for login window
    steps.push("Waiting for login window...".into());
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    // Step 4: Scan for UI elements
    steps.push("Scanning for UI elements...".into());

    let scan_result = scan_elements_windows("Epic");
    match scan_result {
        Ok(elements) => {
            steps.push(format!("Found {} UI elements", elements.len()));

            // Log interesting elements
            for elem in &elements {
                if elem.control_type.contains("Edit") || elem.control_type.contains("Button") {
                    steps.push(format!(
                        "  {} | name='{}' | id='{}' | class='{}'",
                        elem.control_type, elem.name, elem.automation_id, elem.class_name
                    ));
                }
            }

            // Try to find email and password fields
            let edit_fields: Vec<&UiElement> = elements.iter()
                .filter(|e| e.control_type.contains("Edit"))
                .collect();

            let buttons: Vec<&UiElement> = elements.iter()
                .filter(|e| e.control_type.contains("Button"))
                .collect();

            steps.push(format!("Edit fields: {}, Buttons: {}", edit_fields.len(), buttons.len()));

            if edit_fields.is_empty() {
                steps.push("No edit fields found — Epic may use a web-based login (Chromium)".into());
                steps.push("UI Automation cannot see inside Chromium web views".into());
                steps.push("Will need SendInput keyboard simulation instead".into());
            }

            Ok(AutoLoginResult {
                success: false, // Discovery mode — not actually logging in yet
                steps,
                error: Some("Discovery mode — logged UI elements for analysis".into()),
            })
        }
        Err(e) => {
            steps.push(format!("Scan failed: {}", e));
            Ok(AutoLoginResult {
                success: false,
                steps,
                error: Some(e),
            })
        }
    }
}
