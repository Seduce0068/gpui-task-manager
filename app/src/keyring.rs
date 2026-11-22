// SPDX-License-Identifier: AGPL-3.0-or-later
//! OS keyring token storage for the native shell (C1-KEYRING).
//!
//! The interface in [`TokenStore`] is deliberately platform-independent so a
//! browser/wasm shell (no D-Bus) or a test can supply its own implementation.
//! The real backend is gated on `all(unix, target_os = "linux")` because
//! `org.freedesktop.secrets` is only reachable from a native Linux session
//! (Kubuntu provides it via `ksecretd`); there is no silent fallback to an
//! unencrypted or settings-file store.
//!
//! Design decisions recorded here so they survive review:
//!
//! - `secret-service` 5.2.0 is pinned with `default-features = false` and the
//!   explicit `rt-async-io-crypto-rust` feature. The crate raises
//!   `compile_error!` when no crypto feature is selected, and `rt-async-io`
//!   provides the zbus 5 runtime its blocking API needs. The `keyring` crate
//!   was rejected as a redundant wrapper around this same backend.
//! - Every operation opens a fresh blocking connection and closes it at the
//!   end of the call. This avoids storing a self-referential pair of
//!   `SecretService` / `Collection` values and keeps `TokenStore`'s `&self`
//!   methods simple; no connection or credential is kept in process memory.
//! - Attributes and the label are fixed constants. They identify exactly one
//!   item, so `replace = true` cannot create duplicates and a search cannot
//!   accidentally match an item belonging to another application.
//! - Error text is always one of the fixed constants below. D-Bus detail,
//!   object paths and the token itself are never formatted into an error, a
//!   log line or a notice. Callers detect a backend failure with
//!   [`should_fallback`] and then explicitly use the pre-existing
//!   `TM_AGENT_TOKEN` / `--token-file` sources; tokens are never written to
//!   the settings JSON schema.

#[cfg(all(unix, target_os = "linux"))]
use std::collections::HashMap;

/// Minimal, portable token-store contract.
///
/// `load` returns `None` both when no token is stored and when the backend is
/// unusable. Callers that must distinguish those cases on Linux should use
/// `try_load_token`, which reports a fixed error string instead.
pub trait TokenStore {
    /// Returns the stored token, or `None` when it cannot be used.
    fn load(&self) -> Option<String>;
    /// Stores `token`, replacing any previous value. Returns a fixed error text.
    fn save(&self, token: &str) -> Result<(), String>;
    /// Removes the stored token. Must be idempotent.
    fn clear(&self) -> Result<(), String>;
}

/// `true` only where the Secret Service backend is compiled in.
pub const SECRET_SERVICE_AVAILABLE: bool = cfg!(all(unix, target_os = "linux"));

/// Attribute key identifying the owning application.
pub const ATTR_SERVICE_KEY: &str = "service";
/// Fixed value of the owning-application attribute.
pub const ATTR_SERVICE_VALUE: &str = "gpui-task-manager";
/// Attribute key identifying the individual secret.
pub const ATTR_KEY_KEY: &str = "key";
/// Fixed value of the secret-identifier attribute.
pub const ATTR_KEY_VALUE: &str = "agent-token";
/// Fixed label of the stored item (never derived from user input).
pub const ITEM_LABEL: &str = "gpui-task-manager agent token";
/// Fixed MIME type of the stored bytes.
pub const CONTENT_TYPE: &str = "text/plain";

/// No Secret Service provider (or session) is reachable.
pub const ERR_UNAVAILABLE: &str = "Secret Service に接続できませんでした。TM_AGENT_TOKEN または --token-file からトークンを指定してください。";
/// The collection/item is locked and cannot reveal or accept a secret.
pub const ERR_LOCKED: &str = "Secret Service のキーリングがロックされています。ロックを解除するか、TM_AGENT_TOKEN または --token-file からトークンを指定してください。";
/// The provider's authorization prompt was dismissed or disconnected.
pub const ERR_PROMPT: &str = "キーリングの認証プロンプトが完了しませんでした。TM_AGENT_TOKEN または --token-file からトークンを指定してください。";
/// Any other provider/D-Bus failure.
pub const ERR_BACKEND: &str = "Secret Service へのアクセスに失敗しました。TM_AGENT_TOKEN または --token-file からトークンを指定してください。";
/// Stored bytes were not valid UTF-8 and cannot be used as a token.
pub const ERR_INVALID_TOKEN: &str = "保存されたトークンを読み取れませんでした。TM_AGENT_TOKEN または --token-file からトークンを指定してください。";
/// Caller passed an empty token; nothing was written.
pub const ERR_EMPTY_TOKEN: &str = "保存するトークンが空です。";

/// Every error text this module can return (used by tests and callers).
pub const FIXED_ERRORS: &[&str] = &[
    ERR_UNAVAILABLE,
    ERR_LOCKED,
    ERR_PROMPT,
    ERR_BACKEND,
    ERR_INVALID_TOKEN,
    ERR_EMPTY_TOKEN,
];

/// Whether `message` is a backend failure for which the caller should fall
/// back to `TM_AGENT_TOKEN` / `--token-file`. An empty-token rejection is a
/// caller mistake, not a reason to switch token sources.
pub fn should_fallback(message: &str) -> bool {
    message == ERR_UNAVAILABLE
        || message == ERR_LOCKED
        || message == ERR_PROMPT
        || message == ERR_BACKEND
        || message == ERR_INVALID_TOKEN
}

/// The exact attribute set used for search and creation.
#[cfg(all(unix, target_os = "linux"))]
fn fixed_attributes() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        (ATTR_SERVICE_KEY, ATTR_SERVICE_VALUE),
        (ATTR_KEY_KEY, ATTR_KEY_VALUE),
    ])
}

/// Redacts any backend error into one of the fixed messages above.
///
/// Payloads such as `Error::Crypto(&'static str)` and nested zbus errors are
/// intentionally discarded: they can contain provider detail and must never
/// reach a log line or the status bar.
#[cfg(all(unix, target_os = "linux"))]
fn map_service_error(error: &secret_service::Error) -> String {
    use secret_service::Error;

    let fixed = match error {
        Error::Unavailable => ERR_UNAVAILABLE,
        Error::Locked => ERR_LOCKED,
        Error::Prompt | Error::PromptDisconnected => ERR_PROMPT,
        Error::NoResult
        | Error::Crypto(_)
        | Error::Zbus(_)
        | Error::ZbusFdo(_)
        | Error::Zvariant(_) => ERR_BACKEND,
        // `Error` is `#[non_exhaustive]`; future variants must stay redacted.
        _ => ERR_BACKEND,
    };
    fixed.to_owned()
}

/// Loads the token with a distinguishable error channel.
///
/// `Ok(None)` means "no item was stored"; `Err(fixed_text)` means the caller
/// should use [`should_fallback`] and try `TM_AGENT_TOKEN` / `--token-file`.
#[cfg(all(unix, target_os = "linux"))]
pub fn try_load_token() -> Result<Option<String>, String> {
    // A connection failure means the provider is not usable, whatever the
    // underlying zbus variant; reporting it as `Unavailable` keeps the text fixed.
    let service =
        secret_service::blocking::SecretService::connect(secret_service::EncryptionType::Dh)
            .map_err(|_| ERR_UNAVAILABLE.to_owned())?;
    let _collection = service
        .get_default_collection()
        .map_err(|error| map_service_error(&error))?;
    let found = service
        .search_items(fixed_attributes())
        .map_err(|error| map_service_error(&error))?;

    let item = if let Some(item) = found.unlocked.first() {
        item
    } else if let Some(item) = found.locked.first() {
        // Follow the crate's documented pattern: unlocking may wait for the
        // provider prompt, whose failure is still reported as a fixed text.
        item.unlock().map_err(|error| map_service_error(&error))?;
        item
    } else {
        return Ok(None);
    };

    let bytes = item
        .get_secret()
        .map_err(|error| map_service_error(&error))?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| ERR_INVALID_TOKEN.to_owned())
}

/// Stores the token, replacing any item with the same fixed attributes.
#[cfg(all(unix, target_os = "linux"))]
pub fn save_token(token: &str) -> Result<(), String> {
    if token.is_empty() {
        return Err(ERR_EMPTY_TOKEN.to_owned());
    }
    let service =
        secret_service::blocking::SecretService::connect(secret_service::EncryptionType::Dh)
            .map_err(|_| ERR_UNAVAILABLE.to_owned())?;
    let collection = service
        .get_default_collection()
        .map_err(|error| map_service_error(&error))?;
    collection
        .create_item(
            ITEM_LABEL,
            fixed_attributes(),
            token.as_bytes(),
            true,
            CONTENT_TYPE,
        )
        .map_err(|error| map_service_error(&error))?;
    Ok(())
}

/// Deletes every item with the fixed attributes; a missing item is success.
#[cfg(all(unix, target_os = "linux"))]
pub fn clear_token() -> Result<(), String> {
    let service =
        secret_service::blocking::SecretService::connect(secret_service::EncryptionType::Dh)
            .map_err(|_| ERR_UNAVAILABLE.to_owned())?;
    let _collection = service
        .get_default_collection()
        .map_err(|error| map_service_error(&error))?;
    let found = service
        .search_items(fixed_attributes())
        .map_err(|error| map_service_error(&error))?;
    for item in found.unlocked.iter().chain(found.locked.iter()) {
        item.delete().map_err(|error| map_service_error(&error))?;
    }
    Ok(())
}

/// Zero-sized handle implementing [`TokenStore`] through Secret Service.
#[cfg(all(unix, target_os = "linux"))]
#[derive(Debug, Default, Clone, Copy)]
pub struct SecretServiceTokenStore;

#[cfg(all(unix, target_os = "linux"))]
impl TokenStore for SecretServiceTokenStore {
    fn load(&self) -> Option<String> {
        // The trait has no error channel: an unusable store is indistinguishable
        // from an empty one here by design. Use `try_load_token` when the caller
        // needs to trigger the explicit fallback path.
        try_load_token().ok().flatten()
    }

    fn save(&self, token: &str) -> Result<(), String> {
        save_token(token)
    }

    fn clear(&self) -> Result<(), String> {
        clear_token()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    // A fake value used only to prove that error text never echoes a payload.
    const DUMMY_SENTINEL: &str = "DUMMY_TOKEN_VALUE_NOT_A_REAL_SECRET";

    /// Portable mock: proves the trait is usable without any D-Bus backend.
    #[derive(Default)]
    struct MockTokenStore {
        value: RefCell<Option<String>>,
        save_calls: RefCell<usize>,
        clear_calls: RefCell<usize>,
    }

    impl TokenStore for MockTokenStore {
        fn load(&self) -> Option<String> {
            self.value.borrow().clone()
        }

        fn save(&self, token: &str) -> Result<(), String> {
            if token.is_empty() {
                return Err(ERR_EMPTY_TOKEN.to_owned());
            }
            *self.save_calls.borrow_mut() += 1;
            *self.value.borrow_mut() = Some(token.to_owned());
            Ok(())
        }

        fn clear(&self) -> Result<(), String> {
            *self.clear_calls.borrow_mut() += 1;
            *self.value.borrow_mut() = None;
            Ok(())
        }
    }

    #[test]
    fn mock_contract_roundtrip_and_clear() {
        let store = MockTokenStore::default();
        assert_eq!(store.load(), None);
        store.save("first").expect("save must succeed");
        assert_eq!(store.load().as_deref(), Some("first"));
        store.save("second").expect("replacement must succeed");
        assert_eq!(store.load().as_deref(), Some("second"));
        assert_eq!(*store.save_calls.borrow(), 2);
        store.clear().expect("clear must succeed");
        assert_eq!(store.load(), None);
        assert_eq!(*store.clear_calls.borrow(), 1);
        store.clear().expect("clear must be idempotent");
        assert_eq!(store.load(), None);
    }

    #[test]
    fn mock_rejects_empty_token_without_storing() {
        let store = MockTokenStore::default();
        let error = store.save("").unwrap_err();
        assert_eq!(error, ERR_EMPTY_TOKEN);
        assert_eq!(store.load(), None);
        assert_eq!(*store.save_calls.borrow(), 0);
        assert!(!should_fallback(&error));
    }

    #[test]
    fn trait_is_platform_independent() {
        // Compiles (and runs) on wasm and on Linux alike: no cfg gate here.
        fn assert_store<T: TokenStore>() {}
        assert_store::<MockTokenStore>();
    }

    #[test]
    fn fixed_strings_are_redacted_and_stable() {
        for message in FIXED_ERRORS {
            assert!(!message.contains(DUMMY_SENTINEL));
            assert!(!message.contains("D-Bus"));
        }
        assert_eq!(ATTR_SERVICE_KEY, "service");
        assert_eq!(ATTR_SERVICE_VALUE, "gpui-task-manager");
        assert_eq!(ATTR_KEY_KEY, "key");
        assert_eq!(ATTR_KEY_VALUE, "agent-token");
        assert_eq!(ITEM_LABEL, "gpui-task-manager agent token");
        assert_eq!(CONTENT_TYPE, "text/plain");
    }

    #[test]
    fn fallback_detection_is_explicit() {
        for message in [
            ERR_UNAVAILABLE,
            ERR_LOCKED,
            ERR_PROMPT,
            ERR_BACKEND,
            ERR_INVALID_TOKEN,
        ] {
            assert!(should_fallback(message), "backend error must fall back");
        }
        assert!(!should_fallback(ERR_EMPTY_TOKEN));
        assert!(!should_fallback("some unrelated failure"));
    }

    #[cfg(all(unix, target_os = "linux"))]
    #[test]
    fn linux_backend_is_compiled_and_implements_trait() {
        const { assert!(SECRET_SERVICE_AVAILABLE) };
        fn assert_store<T: TokenStore>() {}
        assert_store::<SecretServiceTokenStore>();
    }

    #[cfg(all(unix, target_os = "linux"))]
    #[test]
    fn fixed_attributes_are_exactly_the_two_constants() {
        let attributes = fixed_attributes();
        assert_eq!(attributes.len(), 2);
        assert_eq!(
            attributes.get(ATTR_SERVICE_KEY).copied(),
            Some(ATTR_SERVICE_VALUE)
        );
        assert_eq!(attributes.get(ATTR_KEY_KEY).copied(), Some(ATTR_KEY_VALUE));
    }

    #[cfg(all(unix, target_os = "linux"))]
    #[test]
    fn empty_token_is_rejected_before_any_dbus_access() {
        // No keyring daemon is required for this path: validation happens first.
        let error = save_token("").unwrap_err();
        assert_eq!(error, ERR_EMPTY_TOKEN);
        assert!(!should_fallback(&error));
    }

    #[cfg(all(unix, target_os = "linux"))]
    #[test]
    fn backend_errors_are_redacted_into_fixed_text() {
        use secret_service::Error;

        // The `Crypto` payload is a provider-supplied string; it must be dropped.
        let crypto = map_service_error(&Error::Crypto(DUMMY_SENTINEL));
        assert_eq!(crypto, ERR_BACKEND);
        assert!(!crypto.contains(DUMMY_SENTINEL));

        for (error, expected) in [
            (Error::Unavailable, ERR_UNAVAILABLE),
            (Error::Locked, ERR_LOCKED),
            (Error::Prompt, ERR_PROMPT),
            (Error::PromptDisconnected, ERR_PROMPT),
            (Error::NoResult, ERR_BACKEND),
        ] {
            let message = map_service_error(&error);
            assert_eq!(message, expected);
            assert!(!message.contains(DUMMY_SENTINEL));
        }
    }
}
