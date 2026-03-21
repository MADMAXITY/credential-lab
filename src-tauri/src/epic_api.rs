//! Epic Games API-based authentication via device_code + device_auth.
//!
//! Based on xMistt/DeviceAuthGenerator (v1.3.0, Dec 2024).
//!
//! Setup flow:
//! 1. Switch client → client_credentials → client access token
//! 2. Client token → create device_code → user approves in browser
//! 3. Poll device_code → get user token (Switch client)
//! 4. Switch token → exchange code → Android client token
//! 5. Android client token → create device_auth → save credentials
//!
//! Switch flow:
//! 1. device_auth → Android client token
//! 2. Android token → exchange code
//! 3. Launch Epic with -AUTH_PASSWORD=<code> -AUTH_TYPE=exchangecode

use serde::{Deserialize, Serialize};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

const EPIC_TOKEN_URL: &str = "https://account-public-service-prod.ol.epicgames.com/account/api/oauth/token";
const EPIC_TOKEN_URL_03: &str = "https://account-public-service-prod03.ol.epicgames.com/account/api/oauth/token";
const EPIC_EXCHANGE_URL: &str = "https://account-public-service-prod03.ol.epicgames.com/account/api/oauth/exchange";
const EPIC_DEVICE_CODE_URL: &str = "https://account-public-service-prod03.ol.epicgames.com/account/api/oauth/deviceAuthorization";
const EPIC_DEVICE_AUTH_URL: &str = "https://account-public-service-prod.ol.epicgames.com/account/api/public/account";

// Nintendo Switch client — can create device codes
const SWITCH_TOKEN: &str = "OThmN2U0MmMyZTNhNGY4NmE3NGViNDNmYmI0MWVkMzk6MGEyNDQ5YTItMDAxYS00NTFlLWFmZWMtM2U4MTI5MDFjNGQ3";

// Android client — can create device auths
const ANDROID_TOKEN: &str = "M2Y2OWU1NmM3NjQ5NDkyYzhjYzI5ZjFhZjA4YThhMTI6YjUxZWU5Y2IxMjIzNGY1MGE2OWVmYTY3ZWY1MzgxMmU=";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeviceAuthCredentials {
    pub account_id: String,
    pub device_id: String,
    pub secret: String,
    pub display_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EpicApiResult {
    pub success: bool,
    pub steps: Vec<String>,
    pub device_auth: Option<DeviceAuthCredentials>,
    pub verification_url: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    account_id: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    verification_uri_complete: String,
    device_code: String,
}

#[derive(Debug, Deserialize)]
struct DeviceAuthResponse {
    #[serde(rename = "accountId")]
    account_id: String,
    #[serde(rename = "deviceId")]
    device_id: String,
    secret: String,
}

#[derive(Debug, Deserialize)]
struct ExchangeCodeResponse {
    code: String,
}

// ═══════════════════════════════════════════════════════════════════════════════
// Step 1: Start device code flow — returns URL for user to open in browser
// ═══════════════════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn epic_start_device_code() -> Result<EpicApiResult, String> {
    let mut steps = Vec::new();
    let client = reqwest::Client::new();

    // Get client access token using Switch client
    steps.push("Getting client token (Switch client)...".into());
    let token_resp = client
        .post(EPIC_TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Authorization", format!("basic {}", SWITCH_TOKEN))
        .body("grant_type=client_credentials")
        .send().await.map_err(|e| format!("Request failed: {}", e))?;

    if !token_resp.status().is_success() {
        let body = token_resp.text().await.unwrap_or_default();
        return Err(format!("Client credentials failed: {}", &body[..300.min(body.len())]));
    }

    let client_token: TokenResponse = token_resp.json().await
        .map_err(|e| format!("Parse failed: {}", e))?;
    steps.push("Got client access token".into());

    // Create device code
    steps.push("Creating device code...".into());
    let dc_resp = client
        .post(EPIC_DEVICE_CODE_URL)
        .header("Authorization", format!("bearer {}", client_token.access_token))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send().await.map_err(|e| format!("Request failed: {}", e))?;

    if !dc_resp.status().is_success() {
        let body = dc_resp.text().await.unwrap_or_default();
        return Err(format!("Device code creation failed: {}", &body[..300.min(body.len())]));
    }

    let device_code: DeviceCodeResponse = dc_resp.json().await
        .map_err(|e| format!("Parse failed: {}", e))?;

    steps.push(format!("Open this URL in your browser to log in:"));
    steps.push(device_code.verification_uri_complete.clone());

    // Store the device_code for polling — return it to frontend
    // Frontend will call epic_poll_device_code with this code
    Ok(EpicApiResult {
        success: true,
        steps,
        device_auth: None,
        verification_url: Some(device_code.verification_uri_complete),
        error: Some(device_code.device_code), // Reuse error field to pass device_code to frontend
    })
}

// ═══════════════════════════════════════════════════════════════════════════════
// Step 2: Poll device code until user approves, then create device auth
// ═══════════════════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn epic_poll_device_code(device_code: String) -> Result<EpicApiResult, String> {
    let mut steps = Vec::new();
    let client = reqwest::Client::new();

    // Poll for user approval (up to 60 seconds)
    steps.push("Waiting for you to approve in browser...".into());
    let mut user_token: Option<TokenResponse> = None;

    for i in 0..18 {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;

        let poll_resp = client
            .post(EPIC_TOKEN_URL_03)
            .header("Authorization", format!("basic {}", SWITCH_TOKEN))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(format!("grant_type=device_code&device_code={}", device_code))
            .send().await.map_err(|e| format!("Poll failed: {}", e))?;

        if poll_resp.status().is_success() {
            user_token = Some(poll_resp.json().await
                .map_err(|e| format!("Parse failed: {}", e))?);
            steps.push(format!("Login approved after {}s", (i + 1) * 10));
            break;
        } else {
            let body = poll_resp.text().await.unwrap_or_default();
            if body.contains("authorization_pending") || body.contains("slow_down") {
                steps.push(format!("Waiting... ({}s)", (i + 1) * 10));
            } else {
                return Err(format!("Poll error: {}", &body[..300.min(body.len())]));
            }
        }
    }

    let switch_token = user_token.ok_or("Timeout — user didn't approve in 3 minutes")?;
    let display_name = switch_token.display_name.clone().unwrap_or_default();
    steps.push(format!("Logged in as: {}", display_name));

    // Exchange Switch token → Android client token (for device auth creation)
    steps.push("Getting exchange code...".into());
    let exchange_resp = client
        .get(EPIC_EXCHANGE_URL)
        .header("Authorization", format!("bearer {}", switch_token.access_token))
        .send().await.map_err(|e| format!("Exchange failed: {}", e))?;

    if !exchange_resp.status().is_success() {
        let body = exchange_resp.text().await.unwrap_or_default();
        return Err(format!("Exchange code failed: {}", &body[..300.min(body.len())]));
    }

    let exchange: ExchangeCodeResponse = exchange_resp.json().await
        .map_err(|e| format!("Parse failed: {}", e))?;
    steps.push("Got exchange code".into());

    // Exchange code → Android client token
    steps.push("Hopping to Android client...".into());
    let android_resp = client
        .post(EPIC_TOKEN_URL_03)
        .header("Authorization", format!("basic {}", ANDROID_TOKEN))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(format!("grant_type=exchange_code&exchange_code={}", exchange.code))
        .send().await.map_err(|e| format!("Android token failed: {}", e))?;

    if !android_resp.status().is_success() {
        let body = android_resp.text().await.unwrap_or_default();
        return Err(format!("Android client token failed: {}", &body[..300.min(body.len())]));
    }

    let android_token: TokenResponse = android_resp.json().await
        .map_err(|e| format!("Parse failed: {}", e))?;
    let account_id = android_token.account_id.clone().unwrap_or_default();
    steps.push("Got Android client token".into());

    // Create device auth
    steps.push("Creating device auth credentials...".into());
    let da_url = format!("{}/{}/deviceAuth", EPIC_DEVICE_AUTH_URL, account_id);
    let da_resp = client
        .post(&da_url)
        .header("Authorization", format!("bearer {}", android_token.access_token))
        .header("Content-Type", "application/json")
        .send().await.map_err(|e| format!("Device auth creation failed: {}", e))?;

    if !da_resp.status().is_success() {
        let body = da_resp.text().await.unwrap_or_default();
        return Err(format!("Device auth failed: {}", &body[..300.min(body.len())]));
    }

    let device_auth: DeviceAuthResponse = da_resp.json().await
        .map_err(|e| format!("Parse failed: {}", e))?;

    let creds = DeviceAuthCredentials {
        account_id: device_auth.account_id,
        device_id: device_auth.device_id,
        secret: device_auth.secret,
        display_name,
    };

    steps.push(format!("Device auth created for '{}'! These never expire.", creds.display_name));

    Ok(EpicApiResult {
        success: true,
        steps,
        device_auth: Some(creds),
        verification_url: None,
        error: None,
    })
}

// ═══════════════════════════════════════════════════════════════════════════════
// Switch: device_auth → token → exchange code → launch Epic
// ═══════════════════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn epic_api_switch(
    account_id: String,
    device_id: String,
    secret: String,
    display_name: String,
) -> Result<EpicApiResult, String> {
    let mut steps = Vec::new();
    let http = reqwest::Client::new();

    // Kill Epic
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "EpicGamesLauncher.exe"]).output();
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "EpicWebHelper.exe"]).output();
    }
    steps.push("Killed Epic processes".into());
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Device auth → Android token
    steps.push(format!("Authenticating as '{}'...", display_name));
    let token_resp = http
        .post(EPIC_TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Authorization", format!("basic {}", ANDROID_TOKEN))
        .body(format!(
            "grant_type=device_auth&account_id={}&device_id={}&secret={}",
            account_id, device_id, secret
        ))
        .send().await.map_err(|e| format!("Auth failed: {}", e))?;

    if !token_resp.status().is_success() {
        let body = token_resp.text().await.unwrap_or_default();
        return Err(format!("Device auth login failed: {}", &body[..300.min(body.len())]));
    }

    let token: TokenResponse = token_resp.json().await
        .map_err(|e| format!("Parse failed: {}", e))?;
    steps.push("Got access token".into());

    // Token → exchange code
    let exchange_resp = http
        .get(EPIC_EXCHANGE_URL)
        .header("Authorization", format!("bearer {}", token.access_token))
        .send().await.map_err(|e| format!("Exchange failed: {}", e))?;

    if !exchange_resp.status().is_success() {
        let body = exchange_resp.text().await.unwrap_or_default();
        return Err(format!("Exchange code failed: {}", &body[..300.min(body.len())]));
    }

    let exchange: ExchangeCodeResponse = exchange_resp.json().await
        .map_err(|e| format!("Parse failed: {}", e))?;
    steps.push(format!("Got exchange code: {}...", &exchange.code[..8.min(exchange.code.len())]));

    // Launch Epic with exchange code
    #[cfg(target_os = "windows")]
    {
        let epic_exe = crate::switcher::find_epic_exe()
            .ok_or("Epic Games Launcher not found")?;

        std::process::Command::new(&epic_exe)
            .args([
                &format!("-AUTH_PASSWORD={}", exchange.code),
                "-AUTH_TYPE=exchangecode",
            ])
            .spawn()
            .map_err(|e| format!("Failed to start Epic: {}", e))?;

        steps.push("Launched Epic with exchange code".into());
    }

    steps.push("Waiting for Epic to log in...".into());
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    steps.push(format!("Switch complete for '{}'", display_name));

    Ok(EpicApiResult {
        success: true,
        steps,
        device_auth: None,
        verification_url: None,
        error: None,
    })
}
