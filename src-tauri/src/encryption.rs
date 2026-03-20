//! Credential encryption — placeholder for now.
//! TODO: Add DPAPI encryption when integrating with production.
//!
//! For the lab, we store credentials as-is in SQLite.
//! This is fine for a dev-only tool on a single machine.

pub fn encrypt(data: &[u8]) -> Result<Vec<u8>, String> {
    // TODO: DPAPI CryptProtectData
    Ok(data.to_vec())
}

pub fn decrypt(data: &[u8]) -> Result<Vec<u8>, String> {
    // TODO: DPAPI CryptUnprotectData
    Ok(data.to_vec())
}
