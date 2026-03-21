//! Auto-login via keyboard simulation (SendInput).
//!
//! Epic/EA use Chromium-based login forms that are invisible to Windows
//! UI Automation. Instead, we use SendInput to type credentials directly
//! into the focused login window.
//!
//! Flow:
//! 1. Kill launcher, clear saved login state
//! 2. Start launcher (shows login screen)
//! 3. Find and focus the launcher window
//! 4. Send keystrokes: Ctrl+A → type email → Tab → type password → Enter
//! 5. Wait for login to complete

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct AutoLoginResult {
    pub success: bool,
    pub steps: Vec<String>,
    pub error: Option<String>,
}

/// Attempt to auto-login to Epic Games Launcher via keyboard simulation
#[tauri::command]
pub async fn auto_login_epic(username: String, password: String) -> Result<AutoLoginResult, String> {
    #[cfg(target_os = "windows")]
    {
        auto_login_epic_impl(&username, &password).await
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("Auto-login only available on Windows".into())
    }
}

#[cfg(target_os = "windows")]
async fn auto_login_epic_impl(email: &str, password: &str) -> Result<AutoLoginResult, String> {
    let mut steps = Vec::new();

    // Step 1: Kill Epic
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/IM", "EpicGamesLauncher.exe"]).output();
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/IM", "EpicWebHelper.exe"]).output();
    steps.push("Killed Epic processes".into());
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Step 2: Clear saved login so Epic shows the login form
    clear_epic_login_state(&mut steps);

    // Step 3: Start Epic
    let epic_exe = crate::switcher::find_epic_exe()
        .ok_or("Epic Games Launcher not found")?;
    std::process::Command::new(&epic_exe)
        .spawn()
        .map_err(|e| format!("Failed to start Epic: {}", e))?;
    steps.push("Started Epic Games Launcher".into());

    // Step 4: Wait for Epic to start, then poll until window appears
    steps.push("Waiting for Epic window...".into());
    let mut found = false;
    for i in 0..15 {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let check = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command",
                "if (Get-Process EpicGamesLauncher -ErrorAction SilentlyContinue) { 'FOUND' } else { 'WAITING' }"])
            .output();
        if let Ok(out) = check {
            let result = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if result == "FOUND" {
                steps.push(format!("Epic window found after {}s", (i + 1) * 2));
                found = true;
                break;
            }
        }
    }

    if !found {
        return Ok(AutoLoginResult {
            success: false,
            steps,
            error: Some("Epic Games did not start within 30 seconds".into()),
        });
    }

    // Give it a few more seconds for the login page to fully render
    steps.push("Waiting for login page to render (8s)...".into());
    tokio::time::sleep(std::time::Duration::from_secs(8)).await;

    // Step 5: Use PowerShell to activate window, click email field, type credentials
    let email_owned = email.to_string();
    let password_owned = password.to_string();

    let activate_result = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &format!(
            r#"
            Add-Type -AssemblyName System.Windows.Forms

            $sig = @'
            [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
            [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
            [DllImport("user32.dll")] public static extern void SetCursorPos(int x, int y);
            [DllImport("user32.dll")] public static extern void mouse_event(int dwFlags, int dx, int dy, int dwData, int dwExtraInfo);

            public struct RECT {{ public int Left; public int Top; public int Right; public int Bottom; }}
'@
            $win = Add-Type -MemberDefinition $sig -Name 'WinAPI2' -Namespace 'Win32' -PassThru

            $epic = Get-Process EpicGamesLauncher -ErrorAction SilentlyContinue | Where-Object {{ $_.MainWindowHandle -ne 0 }} | Select-Object -First 1
            if (-not $epic) {{
                Write-Output "EPIC_NO_WINDOW"
                exit
            }}

            # Activate window
            $win::SetForegroundWindow($epic.MainWindowHandle)
            Start-Sleep -Milliseconds 1000

            # Get window position and click on the email field area
            # Email field is roughly centered horizontally, about 40% down from top
            $rect = New-Object Win32.WinAPI2+RECT
            $win::GetWindowRect($epic.MainWindowHandle, [ref]$rect)
            $centerX = [int](($rect.Left + $rect.Right) / 2)
            $emailY = [int]($rect.Top + ($rect.Bottom - $rect.Top) * 0.38)

            # Click on email field
            $win::SetCursorPos($centerX, $emailY)
            Start-Sleep -Milliseconds 100
            $win::mouse_event(0x0002, 0, 0, 0, 0)  # MOUSEEVENTF_LEFTDOWN
            $win::mouse_event(0x0004, 0, 0, 0, 0)  # MOUSEEVENTF_LEFTUP
            Start-Sleep -Milliseconds 500

            # Triple-click to select all text in the field
            $win::mouse_event(0x0002, 0, 0, 0, 0)
            $win::mouse_event(0x0004, 0, 0, 0, 0)
            Start-Sleep -Milliseconds 50
            $win::mouse_event(0x0002, 0, 0, 0, 0)
            $win::mouse_event(0x0004, 0, 0, 0, 0)
            Start-Sleep -Milliseconds 50
            $win::mouse_event(0x0002, 0, 0, 0, 0)
            $win::mouse_event(0x0004, 0, 0, 0, 0)
            Start-Sleep -Milliseconds 300

            # Type email
            [System.Windows.Forms.SendKeys]::SendWait("{0}")
            Start-Sleep -Milliseconds 500

            # Press Tab to move to Continue or press Enter
            [System.Windows.Forms.SendKeys]::SendWait("{{ENTER}}")
            Write-Output "EMAIL_SENT"

            # Wait for password page
            Start-Sleep -Seconds 5

            # Type password (password field should auto-focus)
            [System.Windows.Forms.SendKeys]::SendWait("{1}")
            Start-Sleep -Milliseconds 500

            # Submit
            [System.Windows.Forms.SendKeys]::SendWait("{{ENTER}}")
            Write-Output "DONE"
            "#,
            escape_sendkeys(&email_owned),
            escape_sendkeys(&password_owned),
        )])
        .output()
        .map_err(|e| format!("PowerShell failed: {}", e))?;

    let ps_output = String::from_utf8_lossy(&activate_result.stdout).trim().to_string();
    let ps_error = String::from_utf8_lossy(&activate_result.stderr).trim().to_string();

    if !ps_error.is_empty() {
        steps.push(format!("PS error: {}", &ps_error[..300.min(ps_error.len())]));
    }

    for line in ps_output.lines() {
        steps.push(format!("PS: {}", line));
    }

    // Wait for login
    steps.push("Waiting for login to complete (10s)...".into());
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    // Check if login succeeded
    let account_id = crate::launcher_detect::get_launcher_current_user("epic".into())
        .unwrap_or(None);

    if let Some(ref id) = account_id {
        steps.push(format!("Registry AccountId: {}", &id[..8.min(id.len())]));
    }

    Ok(AutoLoginResult {
        success: false, // Always report false — user must visually verify
        steps,
        error: Some("Check Epic window to verify if login succeeded".into()),
    })
}

/// Escape special characters for PowerShell SendKeys
/// +^%~(){}[] are special in SendKeys and need to be wrapped in braces
#[cfg(target_os = "windows")]
fn escape_sendkeys(text: &str) -> String {
    let mut result = String::new();
    for ch in text.chars() {
        match ch {
            '+' | '^' | '%' | '~' | '(' | ')' | '{' | '}' | '[' | ']' => {
                result.push('{');
                result.push(ch);
                result.push('}');
            }
            _ => result.push(ch),
        }
    }
    result
}

#[cfg(target_os = "windows")]
fn clear_epic_login_state(steps: &mut Vec<String>) {
    let local_app = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let saved_dir = std::path::PathBuf::from(&local_app)
        .join("EpicGamesLauncher")
        .join("Saved");

    // Delete webcache
    if let Ok(entries) = std::fs::read_dir(&saved_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("webcache") {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }

    // Clear RememberMe in all GameUserSettings.ini files
    fn clear_remember_me(dir: &std::path::Path) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    clear_remember_me(&path);
                } else if path.file_name().map_or(false, |n| n == "GameUserSettings.ini") {
                    if let Ok(content) = std::fs::read_to_string(&path) {
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
                        let _ = std::fs::write(&path, output);
                    }
                }
            }
        }
    }

    let config_dir = saved_dir.join("Config");
    if config_dir.exists() {
        clear_remember_me(&config_dir);
    }

    steps.push("Cleared Epic login state (webcache + RememberMe)".into());
}

#[cfg(target_os = "windows")]
fn find_window_by_partial_title(title: &str) -> Option<windows::Win32::Foundation::HWND> {
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::Win32::Foundation::*;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::sync::Mutex;

    static FOUND: AtomicIsize = AtomicIsize::new(0);
    static SEARCH: Mutex<String> = Mutex::new(String::new());

    *SEARCH.lock().unwrap() = title.to_lowercase();
    FOUND.store(0, Ordering::SeqCst);

    unsafe extern "system" fn enum_callback(hwnd: HWND, _: LPARAM) -> BOOL {
        let mut buf = [0u16; 512];
        let len = GetWindowTextW(hwnd, &mut buf);
        if len > 0 {
            let window_title = String::from_utf16_lossy(&buf[..len as usize]).to_lowercase();
            let search = SEARCH.lock().unwrap().clone();
            if window_title.contains(&search) {
                FOUND.store(hwnd.0 as isize, Ordering::SeqCst);
                return FALSE;
            }
        }
        TRUE
    }

    unsafe {
        let _ = EnumWindows(Some(enum_callback), LPARAM(0));
    }

    let val = FOUND.load(Ordering::SeqCst);
    if val != 0 {
        Some(HWND(val as *mut std::ffi::c_void))
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
fn send_key(vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY, _steps: &mut Vec<String>, _log: bool) {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;

    let inputs = [
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: KEYBD_EVENT_FLAGS(0),
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
    ];

    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(target_os = "windows")]
fn send_key_combo(modifier: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY, key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY, _steps: &mut Vec<String>) {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;

    let inputs = [
        // Modifier down
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: modifier,
                    wScan: 0,
                    dwFlags: KEYBD_EVENT_FLAGS(0),
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
        // Key down
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    wScan: 0,
                    dwFlags: KEYBD_EVENT_FLAGS(0),
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
        // Key up
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    wScan: 0,
                    dwFlags: KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
        // Modifier up
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: modifier,
                    wScan: 0,
                    dwFlags: KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
    ];

    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(target_os = "windows")]
fn type_string(text: &str) {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;

    for ch in text.chars() {
        let mut inputs = Vec::new();

        // Use KEYEVENTF_UNICODE for each character
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: ch as u16,
                    dwFlags: KEYEVENTF_UNICODE,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: ch as u16,
                    dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });

        unsafe {
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }

        // Small delay between characters to avoid missed inputs
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
