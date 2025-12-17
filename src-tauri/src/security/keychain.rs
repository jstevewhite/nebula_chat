// src/security/keychain.rs
// Simple wrapper around the `keyring` crate for macOS keychain access.
// Provides set, get, and delete operations for provider API keys.

use keyring::Entry;
use std::error::Error;

/// Store a secret value in the OS keychain.
///
/// `service` is a namespace for the secret (e.g., "nebula_chat"),
/// `username` is an identifier for the specific key (e.g., "openai_api_key"),
/// and `secret` is the value to store.
pub fn set_secret(service: &str, username: &str, secret: &str) -> Result<(), Box<dyn Error>> {
    let entry = Entry::new(service, username)?;
    entry.set_password(secret)?;
    Ok(())
}

/// Retrieve a secret from the OS keychain.
/// Returns `None` if the entry does not exist.
pub fn get_secret(service: &str, username: &str) -> Result<Option<String>, Box<dyn Error>> {
    let entry = Entry::new(service, username)?;
    match entry.get_password() {
        Ok(pw) => Ok(Some(pw)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(Box::new(e)),
    }
}

/// Delete a secret from the OS keychain.
pub fn delete_secret(service: &str, username: &str) -> Result<(), Box<dyn Error>> {
    let entry = Entry::new(service, username)?;
    entry.delete_password()?;
    Ok(())
}
