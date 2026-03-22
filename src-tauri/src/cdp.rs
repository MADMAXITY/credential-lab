//! Launcher auto-login via PostMessage (WM_CHAR / WM_KEYDOWN).
//!
//! Epic Games Launcher and EA Desktop use CEF (Chromium Embedded Framework).
//! Their UI is rendered in a `Chrome_RenderWidgetHostHWND` child window.
//! We send keystrokes directly to this HWND via PostMessage — no coordinates,
//! no pixel checks, no window focus required.
//!
//! Flow:
//! 1. Kill launcher, wipe credentials
//! 2. Start launcher (shows login screen)
//! 3. Find Chrome_RenderWidgetHostHWND
//! 4. PostMessage: type email → Tab → type password → Enter
//! 5. Verify login via registry

use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;

#[cfg(target_os = "windows")]
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM, BOOL};
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, FindWindowExW, GetClassNameW,
    SetForegroundWindow, ShowWindow, GetAncestor,
    SW_RESTORE, GA_ROOT,
};
#[cfg(target_os = "windows")]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_TAB, VK_RETURN, SendInput, INPUT, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_UNICODE, KEYEVENTF_KEYUP, VIRTUAL_KEY,
};
#[cfg(target_os = "windows")]
use windows::core::PCWSTR;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AutoLoginResult {
    pub success: bool,
    pub launcher: String,
    pub steps: Vec<String>,
    pub error: Option<String>,
}

// ═══════════════════════════════════════════════════════════════════════════════
// Tauri Commands
// ═══════════════════════════════════════════════════════════════════════════════

/// Probe: find the CEF render widget in a running launcher.
#[tauri::command]
pub async fn cdp_probe_launcher(launcher_id: String) -> Result<AutoLoginResult, String> {
    match launcher_id.as_str() {
        "epic" => probe_epic().await,
        "ea" => probe_ea().await,
        _ => Err(format!("Auto-login not supported for: {}", launcher_id)),
    }
}

/// Full auto-login flow: wipe → start → type credentials → verify.
#[tauri::command]
pub async fn cdp_login(
    launcher_id: String,
    username: String,
    password: String,
) -> Result<AutoLoginResult, String> {
    match launcher_id.as_str() {
        "epic" => autologin_epic(username, password).await,
        "ea" => Err("EA auto-login not yet implemented".into()),
        _ => Err(format!("Auto-login not supported for: {}", launcher_id)),
    }
}

/// Save a login account (username + password) for auto-login.
#[tauri::command]
pub fn save_login_account(
    launcher_id: String,
    label: String,
    username: String,
    password: String,
    state: tauri::State<crate::AppState>,
) -> Result<i64, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.save_login_account(&launcher_id, &label, &username, &password)
}

/// List saved login accounts for a launcher.
#[tauri::command]
pub fn list_login_accounts(
    launcher_id: String,
    state: tauri::State<crate::AppState>,
) -> Result<Vec<crate::db::LoginAccount>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.list_login_accounts(&launcher_id)
}

/// Remove a login account.
#[tauri::command]
pub fn remove_login_account(
    account_id: i64,
    state: tauri::State<crate::AppState>,
) -> Result<bool, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.remove_login_account(account_id)
}

/// Auto-login with a saved account by ID.
#[tauri::command]
pub async fn autologin_with_account(
    account_id: i64,
    state: tauri::State<'_, crate::AppState>,
) -> Result<AutoLoginResult, String> {
    let account = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_login_account(account_id)?
    };
    match account.launcher.as_str() {
        "epic" => autologin_epic(account.username, account.password).await,
        _ => Err(format!("Auto-login not supported for: {}", account.launcher)),
    }
}

/// List info about the CEF widget (diagnostic).
#[tauri::command]
pub async fn cdp_list_pages(launcher_id: String) -> Result<Vec<CdpPageInfo>, String> {
    let mut results = Vec::new();
    #[cfg(target_os = "windows")]
    {
        let process_name = match launcher_id.as_str() {
            "epic" => "EpicGamesLauncher",
            "ea" => "EADesktop",
            _ => return Err("Unknown launcher".into()),
        };
        let hwnds = find_cef_widgets(process_name);
        for (i, hwnd) in hwnds.iter().enumerate() {
            results.push(CdpPageInfo {
                title: format!("CEF Widget #{}", i),
                url: format!("HWND: {:?}", hwnd),
                page_type: "Chrome_RenderWidgetHostHWND".into(),
            });
        }
    }
    Ok(results)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CdpPageInfo {
    pub title: String,
    pub url: String,
    pub page_type: String,
}

// ═══════════════════════════════════════════════════════════════════════════════
// Epic Games — Probe
// ═══════════════════════════════════════════════════════════════════════════════

async fn probe_epic() -> Result<AutoLoginResult, String> {
    let mut steps = Vec::new();

    // Check if Epic is running
    let running = crate::switcher::is_process_running("EpicGamesLauncher.exe");
    steps.push(format!("Epic running: {}", running));

    if !running {
        let epic_exe = crate::switcher::find_epic_exe()
            .ok_or("Epic Games Launcher not found")?;
        steps.push(format!("Starting Epic: {}", epic_exe));
        std::process::Command::new(&epic_exe)
            .spawn()
            .map_err(|e| format!("Failed to start: {}", e))?;
        steps.push("Waiting for Epic to start...".into());
        sleep(Duration::from_secs(8)).await;
    }

    // Find CEF widget — all Win32 work in blocking task
    steps.push("Searching for Chrome_RenderWidgetHostHWND...".into());

    for attempt in 1..=15 {
        let probe_result = tokio::task::spawn_blocking(|| {
            probe_cef_sync("EpicGamesLauncher")
        })
        .await
        .map_err(|e| e.to_string())?;

        if probe_result.found {
            steps.extend(probe_result.steps);
            return Ok(AutoLoginResult {
                success: true,
                launcher: "epic".into(),
                steps,
                error: None,
            });
        }

        if attempt == 15 {
            steps.extend(probe_result.steps);
        }

        sleep(Duration::from_secs(1)).await;
    }

    steps.push("CEF widget not found after 15 attempts".into());

    // Get diagnostic info
    let diag = tokio::task::spawn_blocking(|| {
        enumerate_window_classes("EpicGamesLauncher")
    })
    .await
    .map_err(|e| e.to_string())?;

    for (class, title) in &diag {
        steps.push(format!("  Window: class='{}' title='{}'", class, title));
    }

    Ok(AutoLoginResult {
        success: false,
        launcher: "epic".into(),
        steps,
        error: Some("No CEF render widget found".into()),
    })
}

#[cfg(target_os = "windows")]
struct ProbeResult {
    found: bool,
    steps: Vec<String>,
}

#[cfg(target_os = "windows")]
fn probe_cef_sync(process_name: &str) -> ProbeResult {
    let mut steps = Vec::new();
    let widgets = find_cef_widgets(process_name);

    if widgets.is_empty() {
        let all = find_all_cef_widgets();
        if all.is_empty() {
            return ProbeResult { found: false, steps };
        }
        steps.push(format!("Found {} system-wide CEF widget(s)", all.len()));
        steps.push("CEF render widget ready for SendInput".into());
        return ProbeResult { found: true, steps };
    }

    steps.push(format!("Found {} CEF widget(s) for {}", widgets.len(), process_name));
    steps.push("CEF render widget ready for SendInput".into());
    ProbeResult { found: true, steps }
}

async fn probe_ea() -> Result<AutoLoginResult, String> {
    Err("EA probe not yet implemented — validate Epic first".into())
}

// ═══════════════════════════════════════════════════════════════════════════════
// Epic Games — Full Auto-Login
// ═══════════════════════════════════════════════════════════════════════════════

async fn autologin_epic(username: String, password: String) -> Result<AutoLoginResult, String> {
    let mut steps = Vec::new();

    // Step 1: Kill Epic and wipe credentials
    crate::switcher::kill_process("EpicGamesLauncher.exe");
    crate::switcher::kill_process("EpicWebHelper.exe");
    steps.push("Killed Epic processes".into());
    sleep(Duration::from_secs(3)).await;

    let wipe_steps = crate::credentials::epic::wipe_login_state()?;
    for s in &wipe_steps {
        steps.push(format!("  {}", s));
    }

    // Step 2: Start Epic
    let epic_exe = crate::switcher::find_epic_exe()
        .ok_or("Epic Games Launcher not found")?;
    std::process::Command::new(&epic_exe)
        .spawn()
        .map_err(|e| format!("Failed to start: {}", e))?;
    steps.push("Started Epic — waiting for login screen...".into());

    // Step 3: Wait for Epic's main window (UnrealWindow class)
    steps.push("Searching for Epic main window (UnrealWindow)...".into());
    let mut found_hwnd: Option<isize> = None;

    for attempt in 1..=30 {
        let result = tokio::task::spawn_blocking(|| -> Option<isize> {
            find_epic_main_window()
        })
        .await
        .map_err(|e| e.to_string())?;

        if let Some(h) = result {
            found_hwnd = Some(h);
            steps.push(format!("Found Epic main window on attempt {}", attempt));
            break;
        }

        sleep(Duration::from_secs(1)).await;
    }

    let hwnd_val = match found_hwnd {
        Some(h) => h,
        None => {
            steps.push("Epic main window not found after 30s".into());
            let diag = tokio::task::spawn_blocking(|| {
                enumerate_window_classes("EpicGamesLauncher")
            }).await.map_err(|e| e.to_string())?;
            for (class, title) in &diag {
                steps.push(format!("  Window: class='{}' title='{}'", class, title));
            }
            return Ok(AutoLoginResult {
                success: false,
                launcher: "epic".into(),
                steps,
                error: Some("Could not find Epic main window".into()),
            });
        }
    };

    // Step 4: Wait for login page to fully render
    // Epic shows a splash/loading screen first, then the login form loads
    steps.push("Waiting 10s for login page to fully render...".into());
    sleep(Duration::from_secs(10)).await;

    // Step 5-8: Type credentials (all Win32 in blocking task)
    // Step 5: Focus launcher, click email field, type email
    let user = username.clone();
    let email_steps = tokio::task::spawn_blocking(move || -> Vec<String> {
        let hwnd = HWND(hwnd_val as *mut _);
        let mut s = Vec::new();

        // Diagnostics on the window we found
        let wnd_class = get_window_class(hwnd);
        let wnd_title = get_window_title(hwnd);
        s.push(format!("Target window: class='{}' title='{}'", wnd_class, wnd_title));

        // Get window rect
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
        let mut rect = RECT::default();
        let got_rect = unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok();
        if got_rect {
            s.push(format!("Window rect: ({},{}) to ({},{}) — {}x{}",
                rect.left, rect.top, rect.right, rect.bottom,
                rect.right - rect.left, rect.bottom - rect.top));
        } else {
            s.push("Could not get window rect!".into());
        }

        // Check what's the current foreground window
        use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        let fg = unsafe { GetForegroundWindow() };
        let fg_class = get_window_class(fg);
        let fg_title = get_window_title(fg);
        s.push(format!("Current foreground: class='{}' title='{}'", fg_class, fg_title));

        // Bring Epic to foreground
        s.push("Setting Epic as foreground...".into());
        focus_launcher(hwnd);
        std::thread::sleep(Duration::from_millis(500));

        // Verify foreground changed
        let fg2 = unsafe { GetForegroundWindow() };
        let fg2_class = get_window_class(fg2);
        let fg2_title = get_window_title(fg2);
        s.push(format!("Foreground after focus: class='{}' title='{}'", fg2_class, fg2_title));

        // Check if Epic is foreground
        let epic_is_fg = fg2.0 == hwnd.0;
        s.push(format!("Epic is foreground: {}", epic_is_fg));

        if !epic_is_fg {
            s.push("Trying Alt key trick to force foreground...".into());
            send_key(hwnd, 0x12); // VK_MENU (Alt)
            std::thread::sleep(Duration::from_millis(100));
            let _ = unsafe { SetForegroundWindow(hwnd) };
            std::thread::sleep(Duration::from_millis(300));

            let fg3 = unsafe { GetForegroundWindow() };
            s.push(format!("Foreground after Alt trick: class='{}'", get_window_class(fg3)));
        }

        // Click email field — center of window, 30% from top
        if got_rect {
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            let click_x = rect.left + width / 2;
            let click_y = rect.top + (height * 30) / 100;
            s.push(format!("Clicking at ({}, {}) for email field...", click_x, click_y));
            click_at(click_x, click_y);
            std::thread::sleep(Duration::from_millis(500));
        }

        // Type email
        s.push(format!("Typing email: {}...", &user[..user.len().min(5)]));
        send_text(hwnd, &user);
        std::thread::sleep(Duration::from_millis(500));

        // Press Enter for "Continue"
        s.push("Pressing Enter (Continue)...".into());
        send_key(hwnd, VK_RETURN.0);

        s
    })
    .await
    .map_err(|e| e.to_string())?;
    steps.extend(email_steps);

    // Step 6: Wait for password page to load
    steps.push("Waiting 5s for password page...".into());
    sleep(Duration::from_secs(5)).await;

    // Step 7: Click password field, type password, submit
    let pass = password.clone();
    let pw_steps = tokio::task::spawn_blocking(move || -> Vec<String> {
        let hwnd = HWND(hwnd_val as *mut _);
        let mut s = Vec::new();

        // Re-focus Epic
        focus_launcher(hwnd);
        std::thread::sleep(Duration::from_millis(300));

        // Click the password field — it's roughly center, ~55% from top
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok() {
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            let click_x = rect.left + width / 2;
            let click_y = rect.top + (height * 55) / 100;
            s.push(format!("Clicking password field at ({}, {})...", click_x, click_y));
            click_at(click_x, click_y);
            std::thread::sleep(Duration::from_millis(500));
        }

        // Type password
        s.push("Typing password...".into());
        send_text(hwnd, &pass);
        std::thread::sleep(Duration::from_millis(300));

        // Enter to submit login
        s.push("Pressing Enter to sign in...".into());
        send_key(hwnd, VK_RETURN.0);

        s
    })
    .await
    .map_err(|e| e.to_string())?;
    steps.extend(pw_steps);

    // Step 8: Wait for potential "Is this the right account?" confirmation page
    steps.push("Waiting 5s for confirmation page...".into());
    sleep(Duration::from_secs(5)).await;

    // Press Enter to click "Yes, continue" (or Tab+Enter if not focused)
    // Some accounts show this confirmation, some go straight to logged-in state
    let confirm_steps = tokio::task::spawn_blocking(move || -> Vec<String> {
        let hwnd = HWND(hwnd_val as *mut _);
        let mut s = Vec::new();

        // Re-focus Epic
        focus_launcher(hwnd);
        std::thread::sleep(Duration::from_millis(300));

        // Click "Yes, continue" button — it's a big blue button roughly center, ~78% from top
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok() {
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            let click_x = rect.left + width / 2;
            let click_y = rect.top + (height * 78) / 100;
            s.push(format!("Clicking 'Yes, continue' at ({}, {})...", click_x, click_y));
            click_at(click_x, click_y);
            std::thread::sleep(Duration::from_millis(300));
        }

        // Also press Enter as backup
        s.push("Pressing Enter (confirm)...".into());
        send_key(hwnd, VK_RETURN.0);

        s
    })
    .await
    .map_err(|e| e.to_string())?;
    steps.extend(confirm_steps);

    // Step 9: Wait for login to fully complete
    steps.push("Waiting 10s for login to complete...".into());
    sleep(Duration::from_secs(10)).await;

    // Step 10: Verify
    let account_id = get_epic_account_id();
    match account_id {
        Some(id) => {
            steps.push(format!("Login successful! AccountId: {}...", &id[..id.len().min(8)]));
            Ok(AutoLoginResult {
                success: true,
                launcher: "epic".into(),
                steps,
                error: None,
            })
        }
        None => {
            steps.push("No AccountId in registry — login may have failed".into());
            steps.push("Possible causes:".into());
            steps.push("  - CAPTCHA was shown".into());
            steps.push("  - 2FA was required".into());
            steps.push("  - Epic showed a 'Choose sign-in method' screen first".into());
            steps.push("  - Email field was not focused on page load".into());
            Ok(AutoLoginResult {
                success: false,
                launcher: "epic".into(),
                steps,
                error: Some("Login verification failed — check steps".into()),
            })
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Win32 Helpers — PostMessage + Window Finding
// ═══════════════════════════════════════════════════════════════════════════════

/// Bring the launcher window to the foreground and click the email field.
#[cfg(target_os = "windows")]
fn focus_launcher(cef_hwnd: HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, SetWindowPos};
    use windows::Win32::Foundation::RECT;

    unsafe {
        // cef_hwnd is now the main UnrealWindow itself
        let target = cef_hwnd;

        // Restore and bring to front
        let _ = ShowWindow(target, SW_RESTORE);
        std::thread::sleep(Duration::from_millis(200));
        let _ = SetForegroundWindow(target);
        std::thread::sleep(Duration::from_millis(500));

        // Get window position to calculate click target
        let mut rect = RECT::default();
        if GetWindowRect(target, &mut rect).is_ok() {
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;

            // Click roughly where the email field is:
            // Horizontally centered, about 30% down from top
            let click_x = rect.left + width / 2;
            let click_y = rect.top + (height * 30) / 100;

            click_at(click_x, click_y);
            std::thread::sleep(Duration::from_millis(300));
        }
    }
}

/// Click at absolute screen coordinates using SendInput.
#[cfg(target_os = "windows")]
fn click_at(x: i32, y: i32) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT_MOUSE, MOUSEINPUT, MOUSE_EVENT_FLAGS,
        MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_MOVE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics;
    use windows::Win32::UI::WindowsAndMessaging::{SM_CXSCREEN, SM_CYSCREEN};

    unsafe {
        // Convert to absolute coordinates (0-65535 range)
        let screen_w = GetSystemMetrics(SM_CXSCREEN) as f64;
        let screen_h = GetSystemMetrics(SM_CYSCREEN) as f64;
        let abs_x = ((x as f64 * 65535.0) / screen_w) as i32;
        let abs_y = ((y as f64 * 65535.0) / screen_h) as i32;

        // Move + click down
        let mut input_down = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: std::mem::zeroed(),
        };
        input_down.Anonymous.mi = MOUSEINPUT {
            dx: abs_x,
            dy: abs_y,
            mouseData: 0,
            dwFlags: MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_MOVE | MOUSEEVENTF_LEFTDOWN,
            time: 0,
            dwExtraInfo: 0,
        };

        // Click up
        let mut input_up = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: std::mem::zeroed(),
        };
        input_up.Anonymous.mi = MOUSEINPUT {
            dx: abs_x,
            dy: abs_y,
            mouseData: 0,
            dwFlags: MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_MOVE | MOUSEEVENTF_LEFTUP,
            time: 0,
            dwExtraInfo: 0,
        };

        let inputs = [input_down, input_up];
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Type text using SendInput with KEYEVENTF_UNICODE — works with CEF.
/// Each character is sent as a Unicode scancode, bypassing keyboard layout issues.
#[cfg(target_os = "windows")]
fn send_text(_hwnd: HWND, text: &str) {
    for ch in text.chars() {
        let scan = ch as u16;

        // Key down
        let mut input_down = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: unsafe { std::mem::zeroed() },
        };
        unsafe {
            input_down.Anonymous.ki = KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: scan,
                dwFlags: KEYEVENTF_UNICODE,
                time: 0,
                dwExtraInfo: 0,
            };
        }

        // Key up
        let mut input_up = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: unsafe { std::mem::zeroed() },
        };
        unsafe {
            input_up.Anonymous.ki = KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: scan,
                dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                time: 0,
                dwExtraInfo: 0,
            };
        }

        let inputs = [input_down, input_up];
        unsafe {
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }

        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Send a virtual key press (Tab, Enter, etc.) using SendInput.
#[cfg(target_os = "windows")]
fn send_key(_hwnd: HWND, vk: u16) {
    let mut input_down = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: unsafe { std::mem::zeroed() },
    };
    unsafe {
        input_down.Anonymous.ki = KEYBDINPUT {
            wVk: VIRTUAL_KEY(vk),
            wScan: 0,
            dwFlags: windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS(0),
            time: 0,
            dwExtraInfo: 0,
        };
    }

    let mut input_up = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: unsafe { std::mem::zeroed() },
    };
    unsafe {
        input_up.Anonymous.ki = KEYBDINPUT {
            wVk: VIRTUAL_KEY(vk),
            wScan: 0,
            dwFlags: KEYEVENTF_KEYUP,
            time: 0,
            dwExtraInfo: 0,
        };
    }

    let inputs = [input_down, input_up];
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }

    std::thread::sleep(Duration::from_millis(50));
}

/// Find Epic's main window — class "UnrealWindow", title "Epic Games Launcher".
#[cfg(target_os = "windows")]
fn find_epic_main_window() -> Option<isize> {
    let target_class: Vec<u16> = "UnrealWindow".encode_utf16().chain(std::iter::once(0)).collect();
    let target_title: Vec<u16> = "Epic Games Launcher".encode_utf16().chain(std::iter::once(0)).collect();

    let hwnd = unsafe {
        FindWindowExW(
            HWND::default(),
            HWND::default(),
            PCWSTR(target_class.as_ptr()),
            PCWSTR(target_title.as_ptr()),
        )
    }.unwrap_or(HWND::default());

    if hwnd.0.is_null() {
        None
    } else {
        Some(hwnd.0 as isize)
    }
}

/// Find Chrome_RenderWidgetHostHWND child windows under windows belonging to a process.
#[cfg(target_os = "windows")]
fn find_cef_widgets(process_name: &str) -> Vec<HWND> {
    let mut results = Vec::new();

    // Get all PIDs for this process ONCE (fast — single tasklist call)
    let pids = get_pids_for_process(process_name);
    if pids.is_empty() {
        return results;
    }

    // Enumerate top-level windows, filter by PID set
    let top_windows = enumerate_top_level_windows();
    for top_hwnd in top_windows {
        if !window_belongs_to_pids(top_hwnd, &pids) {
            continue;
        }
        let children = find_child_by_class(top_hwnd, "Chrome_RenderWidgetHostHWND");
        results.extend(children);
    }

    results
}

/// Find ALL Chrome_RenderWidgetHostHWND windows on the system (fallback).
#[cfg(target_os = "windows")]
fn find_all_cef_widgets() -> Vec<HWND> {
    let mut results = Vec::new();
    let top_windows = enumerate_top_level_windows();
    for top_hwnd in top_windows {
        let children = find_child_by_class(top_hwnd, "Chrome_RenderWidgetHostHWND");
        results.extend(children);
    }
    results
}


/// Enumerate all top-level windows.
#[cfg(target_os = "windows")]
fn enumerate_top_level_windows() -> Vec<HWND> {
    use std::sync::Mutex;

    // Use isize (Send-safe) to store HWND values
    static RESULTS: Mutex<Vec<isize>> = Mutex::new(Vec::new());

    if let Ok(mut r) = RESULTS.lock() {
        r.clear();
    }

    unsafe extern "system" fn enum_callback(hwnd: HWND, _: LPARAM) -> BOOL {
        if let Ok(mut r) = RESULTS.lock() {
            r.push(hwnd.0 as isize);
        }
        BOOL(1)
    }

    unsafe {
        let _ = EnumChildWindows(
            HWND::default(),
            Some(enum_callback),
            LPARAM(0),
        );
    }

    RESULTS.lock()
        .map(|r| r.iter().map(|&v| HWND(v as *mut _)).collect())
        .unwrap_or_default()
}

/// Get set of PIDs for a process name (one tasklist call for all).
#[cfg(target_os = "windows")]
fn get_pids_for_process(process_name: &str) -> std::collections::HashSet<u32> {
    let mut pids = std::collections::HashSet::new();
    let exe_name = if process_name.contains('.') {
        process_name.to_string()
    } else {
        format!("{}.exe", process_name)
    };
    if let Ok(output) = std::process::Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {}", exe_name), "/NH", "/FO", "CSV"])
        .output()
    {
        let out = String::from_utf8_lossy(&output.stdout);
        for line in out.lines() {
            // CSV format: "name.exe","PID","Session Name","Session#","Mem Usage"
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() >= 2 {
                let pid_str = parts[1].trim_matches('"').trim();
                if let Ok(pid) = pid_str.parse::<u32>() {
                    pids.insert(pid);
                }
            }
        }
    }
    pids
}

/// Check if a window belongs to a set of PIDs.
#[cfg(target_os = "windows")]
fn window_belongs_to_pids(hwnd: HWND, pids: &std::collections::HashSet<u32>) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)); }
    pids.contains(&pid)
}

/// Find child windows with a specific class name.
#[cfg(target_os = "windows")]
fn find_child_by_class(parent: HWND, class_name: &str) -> Vec<HWND> {
    let mut results = Vec::new();
    let target: Vec<u16> = class_name.encode_utf16().chain(std::iter::once(0)).collect();

    let mut child = HWND::default();
    loop {
        child = unsafe {
            FindWindowExW(parent, child, PCWSTR(target.as_ptr()), PCWSTR::null())
        }.unwrap_or(HWND::default());
        if child.0.is_null() {
            break;
        }
        results.push(child);
    }

    // Also recurse into children
    if results.is_empty() {
        let mut next_child = HWND::default();
        loop {
            next_child = unsafe {
                FindWindowExW(parent, next_child, PCWSTR::null(), PCWSTR::null())
            }.unwrap_or(HWND::default());
            if next_child.0.is_null() {
                break;
            }
            let sub = find_child_by_class(next_child, class_name);
            results.extend(sub);
            if !results.is_empty() {
                break;
            }
        }
    }

    results
}

/// Enumerate window classes for a process (diagnostic).
#[cfg(target_os = "windows")]
fn enumerate_window_classes(process_name: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();
    let pids = get_pids_for_process(process_name);
    if pids.is_empty() { return results; }
    let top_windows = enumerate_top_level_windows();

    for hwnd in top_windows {
        if !window_belongs_to_pids(hwnd, &pids) {
            continue;
        }
        let class = get_window_class(hwnd);
        let title = get_window_title(hwnd);
        results.push((class, title));

        // Also enumerate direct children
        let mut child = HWND::default();
        loop {
            child = unsafe {
                FindWindowExW(hwnd, child, PCWSTR::null(), PCWSTR::null())
            }.unwrap_or(HWND::default());
            if child.0.is_null() {
                break;
            }
            let class = get_window_class(child);
            let title = get_window_title(child);
            results.push((format!("  └ {}", class), title));
        }
    }

    results
}

#[cfg(target_os = "windows")]
fn get_window_class(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..len])
}

#[cfg(target_os = "windows")]
fn get_window_title(hwnd: HWND) -> String {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowTextW;
    let mut buf = [0u16; 256];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..len])
}

/// Read Epic AccountId from registry.
#[cfg(target_os = "windows")]
fn get_epic_account_id() -> Option<String> {
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
    None
}
