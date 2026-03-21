//! Epic Games API-based authentication.
//!
//! Uses Epic's OAuth API with device_auth grant type for persistent credentials.
//! This avoids file-swapping entirely — each switch gets a fresh exchange code
//! from the API and launches Epic with it.
//!
//! Flow:
//! 1. Setup: authorization_code → access_token → create device_auth → save {account_id, device_id, secret}
//! 2. Switch: device_auth → access_token → exchange_code → launch Epic with -AUTH_PASSWORD=<code>

use serde::{Deserialize, Serialize};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

const EPIC_TOKEN_URL: &str = "https://account-public-service-prod.ol.epicgames.com/account/api/oauth/token";
const EPIC_EXCHANGE_URL: &str = "https://account-public-service-prod.ol.epicgames.com/account/api/oauth/exchange";
const EPIC_DEVICE_AUTH_URL: &str = "https://account-public-service-prod.ol.epicgames.com/account/api/public/account";

// Using launcherAppClient2 — the official Epic Games Launcher client
const CLIENT_ID: &str = "34a02cf8f4414e29b15921876da36f9a";
const CLIENT_SECRET: &str = "daafbccc737745039dffe53d94fc76cf";

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
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    account_id: Option<String>,
    displayName: Option<String>,
    #[allow(dead_code)]
    expires_in: Option<i64>,
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

fn basic_auth() -> String {
    let credentials = format!("{}:{}", CLIENT_ID, CLIENT_SECRET);
    format!("basic {}", BASE64.encode(credentials.as_bytes()))
}

/// Step 1: Get access token using authorization code (from Epic login flow)
/// The user gets this code from Epic's web login
async fn get_token_from_auth_code(auth_code: &str) -> Result<TokenResponse, String> {
    let client = reqwest::Client::new();
    let response = client
        .post(EPIC_TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Authorization", basic_auth())
        .body(format!("grant_type=authorization_code&code={}", auth_code))
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Token request failed ({}): {}", status, &body[..500.min(body.len())]));
    }

    response.json::<TokenResponse>().await
        .map_err(|e| format!("Failed to parse token response: {}", e))
}

/// Step 2: Create device auth credentials (one-time per account)
async fn create_device_auth(access_token: &str, account_id: &str) -> Result<DeviceAuthResponse, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/{}/deviceAuth", EPIC_DEVICE_AUTH_URL, account_id);
    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Device auth creation failed ({}): {}", status, &body[..500.min(body.len())]));
    }

    response.json::<DeviceAuthResponse>().await
        .map_err(|e| format!("Failed to parse device auth response: {}", e))
}

/// Step 3: Get access token using device auth (for subsequent logins)
async fn get_token_from_device_auth(creds: &DeviceAuthCredentials) -> Result<TokenResponse, String> {
    let client = reqwest::Client::new();
    let response = client
        .post(EPIC_TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Authorization", basic_auth())
        .body(format!(
            "grant_type=device_auth&account_id={}&device_id={}&secret={}",
            creds.account_id, creds.device_id, creds.secret
        ))
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Device auth login failed ({}): {}", status, &body[..500.min(body.len())]));
    }

    response.json::<TokenResponse>().await
        .map_err(|e| format!("Failed to parse token response: {}", e))
}

/// Step 4: Get exchange code from access token
async fn get_exchange_code(access_token: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let response = client
        .get(EPIC_EXCHANGE_URL)
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Exchange code request failed ({}): {}", status, &body[..500.min(body.len())]));
    }

    let exchange: ExchangeCodeResponse = response.json().await
        .map_err(|e| format!("Failed to parse exchange response: {}", e))?;

    Ok(exchange.code)
}

// ═══════════════════════════════════════════════════════════════════════════════
// Tauri Commands
// ═══════════════════════════════════════════════════════════════════════════════

/// Setup: Create device auth credentials from an authorization code.
/// User gets the auth code from Epic's web login flow.
#[tauri::command]
pub async fn epic_setup_device_auth(auth_code: String) -> Result<EpicApiResult, String> {
    let mut steps = Vec::new();

    // Step 1: Exchange auth code for access token
    steps.push("Exchanging authorization code for access token...".into());
    let token = get_token_from_auth_code(&auth_code).await?;
    let account_id = token.account_id.as_deref()
        .ok_or("No account_id in token response")?;
    let display_name = token.displayName.clone().unwrap_or_else(|| account_id.to_string());
    steps.push(format!("Got access token for: {} ({})", display_name, &account_id[..8.min(account_id.len())]));

    // Step 2: Create device auth credentials
    steps.push("Creating device auth credentials...".into());
    let device = create_device_auth(&token.access_token, account_id).await?;
    steps.push(format!("Device auth created: device_id={}", &device.device_id[..8.min(device.device_id.len())]));

    let creds = DeviceAuthCredentials {
        account_id: device.account_id,
        device_id: device.device_id,
        secret: device.secret,
        display_name: display_name.clone(),
    };

    steps.push(format!("Setup complete for '{}'. These credentials never expire.", display_name));

    Ok(EpicApiResult {
        success: true,
        steps,
        device_auth: Some(creds),
        error: None,
    })
}

/// Switch: Use saved device auth to get exchange code and launch Epic
#[tauri::command]
pub async fn epic_api_switch(
    account_id: String,
    device_id: String,
    secret: String,
    display_name: String,
) -> Result<EpicApiResult, String> {
    let mut steps = Vec::new();
    let creds = DeviceAuthCredentials { account_id, device_id, secret, display_name: display_name.clone() };

    // Step 1: Kill Epic
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "EpicGamesLauncher.exe"]).output();
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "EpicWebHelper.exe"]).output();
    }
    steps.push("Killed Epic processes".into());
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Step 2: Get access token via device auth
    steps.push(format!("Authenticating as '{}'...", display_name));
    let token = get_token_from_device_auth(&creds).await?;
    steps.push("Got access token via device auth".into());

    // Step 3: Get exchange code
    steps.push("Getting exchange code...".into());
    let exchange_code = get_exchange_code(&token.access_token).await?;
    steps.push(format!("Got exchange code: {}...", &exchange_code[..8.min(exchange_code.len())]));

    // Step 4: Launch Epic with exchange code
    #[cfg(target_os = "windows")]
    {
        let epic_exe = crate::switcher::find_epic_exe()
            .ok_or("Epic Games Launcher not found")?;

        std::process::Command::new(&epic_exe)
            .args([
                &format!("-AUTH_PASSWORD={}", exchange_code),
                "-AUTH_TYPE=exchangecode",
            ])
            .spawn()
            .map_err(|e| format!("Failed to start Epic: {}", e))?;

        steps.push("Launched Epic with exchange code".into());
    }

    // Step 5: Wait and verify
    steps.push("Waiting for Epic to log in (10s)...".into());
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    let current = crate::launcher_detect::get_launcher_current_user("epic".into())
        .unwrap_or(None);

    if let Some(ref id) = current {
        steps.push(format!("Epic logged in as: {}", &id[..12.min(id.len())]));
    } else {
        steps.push("Check Epic window to verify login".into());
    }

    Ok(EpicApiResult {
        success: true,
        steps,
        device_auth: None,
        error: None,
    })
}

/// Get authorization code URL — user opens this in browser to log in.
/// Uses the launcher client which returns the code directly on the page.
#[tauri::command]
pub fn epic_get_auth_url() -> String {
    // This URL shows the auth code directly after login
    "https://www.epicgames.com/id/api/redirect?clientId=34a02cf8f4414e29b15921876da36f9a&responseType=code".to_string()
}
