//! Secure storage for the VNDB API token.
//!
//! Rules from AGENTS.md §6, enforced here:
//!
//! * the token lives in the OS credential store, never in the database, never in
//!   a log, never in a panic message;
//! * it is sent only to `api.vndb.org`, by the VNDB client, and nowhere else;
//! * `Debug` never prints it.
//!
//! Desktop uses the OS credential store. Android uses a non-exportable Keystore
//! AES-GCM key and authenticated ciphertext in noBackupFilesDir. Credentials never
//! enter the library database, sync payloads, or backups.

use std::fmt;

use thiserror::Error;

// Desktop keychain service and entry names; Android uses its own storage plugin.
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
const SERVICE: &str = "com.atho.vnasyik";
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
const VNDB_TOKEN_USER: &str = "vndb-api-token";
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
const DISCORD_TOKEN_USER: &str = "discord-user-token";
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
const IMGBB_KEY_USER: &str = "imgbb-api-key";

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("secure storage is not available on this platform yet")]
    Unsupported,
    #[error("no token is stored")]
    NotFound,
    #[error("the credential store failed: {0}")]
    Store(String),
}

/// A VNDB API token.
///
/// The inner string is deliberately private and `Debug` is hand-written, so a
/// token cannot leak through a log line or a panic payload.
#[derive(Clone, PartialEq, Eq)]
pub struct VndbToken(String);

impl VndbToken {
    /// Wrap a token, validating the documented shape.
    ///
    /// VNDB documents tokens as `xxxx-xxxxx-xxxxx-xxxx-xxxxx-xxxxx-xxxx` where
    /// each `x` is a lowercase z-base-32 character, with optional dashes.
    ///
    /// # Errors
    /// Returns the reason when the shape is wrong, so the UI can explain it.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("the token is empty".to_owned());
        }
        let compact: String = trimmed.chars().filter(|ch| *ch != '-').collect();
        if !compact.chars().all(is_zbase32) {
            return Err("the token contains characters VNDB tokens do not use".to_owned());
        }
        // The documented example is 29 characters without dashes; accept the
        // documented range rather than pinning one exact length.
        if !(20..=64).contains(&compact.len()) {
            return Err(format!(
                "the token is {} characters long, which does not look like a VNDB token",
                compact.len()
            ));
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// The token, for the Authorization header only.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// A masked form safe to show in the UI, e.g. `hsoo-…-bfyk`.
    #[must_use]
    pub fn masked(&self) -> String {
        let chars: Vec<char> = self.0.chars().collect();
        if chars.len() <= 9 {
            return "…".to_owned();
        }
        let head: String = chars.iter().take(4).collect();
        let tail: String = chars.iter().skip(chars.len() - 4).collect();
        format!("{head}-…-{tail}")
    }
}

impl fmt::Debug for VndbToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print the secret, not even truncated.
        f.write_str("VndbToken(<redacted>)")
    }
}

/// z-base-32 alphabet as documented for tokens.
fn is_zbase32(ch: char) -> bool {
    matches!(ch, 'a'..='z' | '0'..='9')
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
pub fn store_vndb_token(token: &VndbToken) -> Result<(), SecretError> {
    let entry = entry()?;
    entry
        .set_password(token.expose())
        .map_err(|err| SecretError::Store(err.to_string()))
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "linux",
    target_os = "android"
)))]
pub fn store_vndb_token(_token: &VndbToken) -> Result<(), SecretError> {
    Err(SecretError::Unsupported)
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
pub fn load_vndb_token() -> Result<VndbToken, SecretError> {
    let entry = entry()?;
    match entry.get_password() {
        Ok(raw) => VndbToken::parse(&raw).map_err(SecretError::Store),
        Err(err) => Err(map_store_error(&err)),
    }
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "linux",
    target_os = "android"
)))]
pub fn load_vndb_token() -> Result<VndbToken, SecretError> {
    Err(SecretError::Unsupported)
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
pub fn clear_vndb_token() -> Result<(), SecretError> {
    let entry = entry()?;
    entry
        .delete_credential()
        .map_err(|err| map_store_error(&err))
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "linux",
    target_os = "android"
)))]
pub fn clear_vndb_token() -> Result<(), SecretError> {
    Err(SecretError::Unsupported)
}

/// A Discord user token for connecting to the Discord Gateway on Android.
///
/// Wrapped so it cannot be printed by accident: `Debug` is redacted, and there is
/// no `Display`. The token is only ever used to authenticate Gateway connections.
#[derive(Clone, PartialEq, Eq)]
pub struct DiscordToken(String);

impl fmt::Debug for DiscordToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DiscordToken(<redacted>)")
    }
}

impl DiscordToken {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("token must not be empty".to_owned());
        }
        if trimmed.len() < 50 || trimmed.len() > 128 {
            return Err(format!(
                "the token is {} characters long, which does not look like a Discord token (expected 50-128 characters)",
                trimmed.len()
            ));
        }
        if trimmed.chars().any(char::is_whitespace) {
            return Err("token must not contain spaces".to_owned());
        }
        if !trimmed.chars().all(|c| c.is_ascii_graphic()) {
            return Err("token contains invalid characters".to_owned());
        }
        Ok(Self(trimmed.to_owned()))
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn masked(&self) -> String {
        let chars: Vec<char> = self.0.chars().collect();
        if chars.len() <= 8 {
            return "****".to_owned();
        }
        let head: String = chars.iter().take(4).collect();
        let tail: String = chars.iter().skip(chars.len() - 4).collect();
        format!("{head}...{tail}")
    }
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
pub fn store_discord_token(token: &DiscordToken) -> Result<(), SecretError> {
    let entry = discord_entry()?;
    entry
        .set_password(token.expose())
        .map_err(|err| SecretError::Store(err.to_string()))
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "linux",
    target_os = "android"
)))]
pub fn store_discord_token(_token: &DiscordToken) -> Result<(), SecretError> {
    Err(SecretError::Unsupported)
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
pub fn load_discord_token() -> Result<DiscordToken, SecretError> {
    let entry = discord_entry()?;
    match entry.get_password() {
        Ok(raw) => DiscordToken::parse(&raw).map_err(SecretError::Store),
        Err(err) => Err(map_store_error(&err)),
    }
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "linux",
    target_os = "android"
)))]
pub fn load_discord_token() -> Result<DiscordToken, SecretError> {
    Err(SecretError::Unsupported)
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
pub fn clear_discord_token() -> Result<(), SecretError> {
    let entry = discord_entry()?;
    entry
        .delete_credential()
        .map_err(|err| map_store_error(&err))
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "linux",
    target_os = "android"
)))]
pub fn clear_discord_token() -> Result<(), SecretError> {
    Err(SecretError::Unsupported)
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
fn discord_entry() -> Result<keyring::Entry, SecretError> {
    if keyring::Entry::store_status().is_err() {
        return Err(SecretError::Unsupported);
    }
    keyring::Entry::new(SERVICE, DISCORD_TOKEN_USER)
        .map_err(|err| SecretError::Store(err.to_string()))
}

/// An imgbb API key for the presence image host.
///
/// Wrapped like the other secrets so it cannot leak through a log line.
#[derive(Clone, PartialEq, Eq)]
pub struct ImgbbApiKey(String);

impl fmt::Debug for ImgbbApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ImgbbApiKey(<redacted>)")
    }
}

impl ImgbbApiKey {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("API key must not be empty".to_owned());
        }
        if trimmed.len() < 20 || trimmed.len() > 64 {
            return Err(format!(
                "the key is {} characters long, which does not look like an imgbb key (expected 20-64 characters)",
                trimmed.len()
            ));
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err("the key contains invalid characters".to_owned());
        }
        Ok(Self(trimmed.to_owned()))
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn masked(&self) -> String {
        let chars: Vec<char> = self.0.chars().collect();
        if chars.len() <= 8 {
            return "****".to_owned();
        }
        let head: String = chars.iter().take(4).collect();
        let tail: String = chars.iter().skip(chars.len() - 4).collect();
        format!("{head}...{tail}")
    }
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
pub fn store_imgbb_key(key: &ImgbbApiKey) -> Result<(), SecretError> {
    let entry = imgbb_entry()?;
    entry
        .set_password(key.expose())
        .map_err(|err| SecretError::Store(err.to_string()))
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "linux",
    target_os = "android"
)))]
pub fn store_imgbb_key(_key: &ImgbbApiKey) -> Result<(), SecretError> {
    Err(SecretError::Unsupported)
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
pub fn load_imgbb_key() -> Result<ImgbbApiKey, SecretError> {
    let entry = imgbb_entry()?;
    match entry.get_password() {
        Ok(raw) => ImgbbApiKey::parse(&raw).map_err(SecretError::Store),
        Err(err) => Err(map_store_error(&err)),
    }
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "linux",
    target_os = "android"
)))]
pub fn load_imgbb_key() -> Result<ImgbbApiKey, SecretError> {
    Err(SecretError::Unsupported)
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
pub fn clear_imgbb_key() -> Result<(), SecretError> {
    let entry = imgbb_entry()?;
    entry
        .delete_credential()
        .map_err(|err| map_store_error(&err))
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "linux",
    target_os = "android"
)))]
pub fn clear_imgbb_key() -> Result<(), SecretError> {
    Err(SecretError::Unsupported)
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
fn imgbb_entry() -> Result<keyring::Entry, SecretError> {
    if keyring::Entry::store_status().is_err() {
        return Err(SecretError::Unsupported);
    }
    keyring::Entry::new(SERVICE, IMGBB_KEY_USER).map_err(|err| SecretError::Store(err.to_string()))
}

#[must_use]
pub fn is_supported() -> bool {
    cfg!(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "linux",
        target_os = "android"
    ))
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
fn entry() -> Result<keyring::Entry, SecretError> {
    if keyring::Entry::store_status().is_err() {
        return Err(SecretError::Unsupported);
    }
    keyring::Entry::new(SERVICE, VNDB_TOKEN_USER).map_err(|err| SecretError::Store(err.to_string()))
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
fn map_store_error(err: &keyring::Error) -> SecretError {
    if matches!(err, keyring::Error::NoEntry) {
        SecretError::NotFound
    } else {
        SecretError::Store(err.to_string())
    }
}

#[cfg(target_os = "android")]
fn android_secret(name: &str) -> Result<String, SecretError> {
    crate::platform::storage::read_secret(name)
        .map_err(SecretError::Store)?
        .ok_or(SecretError::NotFound)
}

#[cfg(target_os = "android")]
pub fn store_vndb_token(token: &VndbToken) -> Result<(), SecretError> {
    crate::platform::storage::write_secret("vndb", token.expose()).map_err(SecretError::Store)
}

#[cfg(target_os = "android")]
pub fn load_vndb_token() -> Result<VndbToken, SecretError> {
    VndbToken::parse(&android_secret("vndb")?).map_err(SecretError::Store)
}

#[cfg(target_os = "android")]
pub fn clear_vndb_token() -> Result<(), SecretError> {
    crate::platform::storage::remove_secret("vndb").map_err(SecretError::Store)
}

#[cfg(target_os = "android")]
pub fn store_discord_token(token: &DiscordToken) -> Result<(), SecretError> {
    crate::platform::storage::write_secret("discord", token.expose()).map_err(SecretError::Store)
}

#[cfg(target_os = "android")]
pub fn load_discord_token() -> Result<DiscordToken, SecretError> {
    DiscordToken::parse(&android_secret("discord")?).map_err(SecretError::Store)
}

#[cfg(target_os = "android")]
pub fn clear_discord_token() -> Result<(), SecretError> {
    crate::platform::storage::remove_secret("discord").map_err(SecretError::Store)
}

#[cfg(target_os = "android")]
pub fn store_imgbb_key(key: &ImgbbApiKey) -> Result<(), SecretError> {
    crate::platform::storage::write_secret("imgbb", key.expose()).map_err(SecretError::Store)
}

#[cfg(target_os = "android")]
pub fn load_imgbb_key() -> Result<ImgbbApiKey, SecretError> {
    ImgbbApiKey::parse(&android_secret("imgbb")?).map_err(SecretError::Store)
}

#[cfg(target_os = "android")]
pub fn clear_imgbb_key() -> Result<(), SecretError> {
    crate::platform::storage::remove_secret("imgbb").map_err(SecretError::Store)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_documented_token_shape() {
        let token = VndbToken::parse("hsoo-ybws4-j8yb9-qxkw-5obay-px8to-bfyk")
            .expect("documented example must parse");
        assert_eq!(token.expose(), "hsoo-ybws4-j8yb9-qxkw-5obay-px8to-bfyk");
    }

    #[test]
    fn accepts_a_token_without_dashes() {
        assert!(VndbToken::parse("hsooybws4j8yb9qxkw5obaypx8tobfyk").is_ok());
    }

    #[test]
    fn trims_whitespace_pasted_from_the_website() {
        let token = VndbToken::parse("  hsoo-ybws4-j8yb9-qxkw-5obay-px8to-bfyk \n").expect("parse");
        assert!(!token.expose().starts_with(' '));
        assert!(!token.expose().ends_with('\n'));
    }

    #[test]
    fn rejects_an_empty_token() {
        assert!(VndbToken::parse("").is_err());
        assert!(VndbToken::parse("   ").is_err());
    }

    #[test]
    fn rejects_non_zbase32_characters() {
        assert!(VndbToken::parse("HSOO-YBWS4-J8YB9-QXKW").is_err());
        assert!(VndbToken::parse("hsoo_ybws4_j8yb9").is_err());
        assert!(VndbToken::parse("hsoo ybws4 j8yb9").is_err());
    }

    #[test]
    fn rejects_implausible_lengths() {
        assert!(VndbToken::parse("abc").is_err());
        assert!(VndbToken::parse(&"a".repeat(200)).is_err());
    }

    #[test]
    fn debug_never_reveals_the_secret() {
        let token = VndbToken::parse("hsoo-ybws4-j8yb9-qxkw-5obay-px8to-bfyk").expect("parse");
        let printed = format!("{token:?}");
        assert!(!printed.contains("hsoo"));
        assert!(!printed.contains("bfyk"));
        assert!(printed.contains("redacted"));
    }

    #[test]
    fn masked_form_shows_only_the_edges() {
        let token = VndbToken::parse("hsoo-ybws4-j8yb9-qxkw-5obay-px8to-bfyk").expect("parse");
        let masked = token.masked();
        assert!(masked.starts_with("hsoo"));
        assert!(masked.ends_with("bfyk"));
        assert!(!masked.contains("ybws4"));
    }

    #[test]
    fn masking_a_short_value_does_not_leak_it() {
        let token = VndbToken("abcdefgh".to_owned());
        assert_eq!(token.masked(), "…");
    }

    #[test]
    fn a_discord_token_never_appears_in_debug_output() {
        let raw = "mfa.W_test_discord_user_token_long_enough_to_pass_validation_1234567890";
        let token = DiscordToken::parse(raw).expect("parse");
        let printed = format!("{token:?}");
        assert!(printed.contains("redacted"), "got {printed}");
        assert!(!printed.contains("test_discord"), "got {printed}");
    }

    #[test]
    fn a_discord_token_masks_to_edges() {
        let raw = "mfa.W_test_discord_user_token_long_enough_to_pass_validation_1234567890";
        let token = DiscordToken::parse(raw).expect("parse");
        let masked = token.masked();
        assert!(masked.starts_with("mfa."), "got {masked}");
        assert!(masked.ends_with("7890"), "got {masked}");
        assert!(!masked.contains("test_discord"), "got {masked}");
    }

    #[test]
    fn a_discord_token_rejects_empty_or_whitespace_or_bad_length() {
        assert!(DiscordToken::parse("").is_err());
        assert!(DiscordToken::parse("   ").is_err());
        assert!(DiscordToken::parse("too_short_token").is_err());
        assert!(
            DiscordToken::parse(
                "mfa.W_test_discord user_token_long_enough_with_space_inside_1234567890"
            )
            .is_err()
        );
        let too_long = "a".repeat(200);
        assert!(DiscordToken::parse(&too_long).is_err());
    }

    #[test]
    fn an_imgbb_key_rejects_empty_short_and_non_alphanumeric_input() {
        assert!(ImgbbApiKey::parse("").is_err());
        assert!(ImgbbApiKey::parse("short").is_err());
        assert!(ImgbbApiKey::parse("has a space in the middle of it ok").is_err());
        let good = "aB3_-".repeat(8);
        assert!(ImgbbApiKey::parse(&good).is_ok());
        let too_long = "a".repeat(80);
        assert!(ImgbbApiKey::parse(&too_long).is_err());
    }

    #[test]
    fn platform_support_matches_available_secure_backends() {
        // Desktop keyring plus the Android Keystore bridge; no plaintext fallback.
        let expected = cfg!(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "linux",
            target_os = "android"
        ));
        assert_eq!(is_supported(), expected);
        if !expected {
            assert!(matches!(load_vndb_token(), Err(SecretError::Unsupported)));
        }
    }
}
