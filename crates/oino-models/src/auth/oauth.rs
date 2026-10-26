//! pi `auth/resolve.ts` OAuth handling: refresh tokens that expire within five minutes,
//! double-checked under the `auth.json` lock so concurrent callers and processes refresh once.

use std::time::{SystemTime, UNIX_EPOCH};

use futures::future::BoxFuture;
use oino_ai::JsonValue;

use super::credential::{AuthCredential, OAuthCredential};
use super::storage::{AuthStorage, AuthStorageError};

/// Tokens with less validity than this are refreshed before use (pi: five minutes).
pub const DEFAULT_OAUTH_MINIMUM_VALIDITY_MS: i64 = 5 * 60 * 1000;

/// Refreshes OAuth credentials through the provider component's `oauth-refresh` export.
///
/// `credential` is the stored JSON (`type`, `access`, `refresh`, `expires`, extra fields); the
/// result must be a credential of the same shape (`type` may be omitted). The host enforces the
/// call deadline (pi: 15 s).
pub trait OAuthRefresher: Send + Sync {
    /// Exchanges the refresh token of `provider`.
    ///
    /// # Errors
    ///
    /// A message describing why the refresh failed.
    fn refresh<'a>(
        &'a self,
        provider: &'a str,
        credential: JsonValue,
    ) -> BoxFuture<'a, Result<JsonValue, String>>;
}

/// OAuth failures (pi `ModelsError` texts).
#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    /// The refresher failed or returned an invalid credential; `reason` is its message.
    #[error("OAuth refresh failed for {provider}")]
    RefreshFailed {
        /// Provider id.
        provider: String,
        /// The refresher's message (pi surfaces it as the request error).
        reason: String,
    },
    /// An explicit minimum validity could not be met.
    #[error("OAuth refresh returned a token that expires too soon for {provider}")]
    ExpiresTooSoon {
        /// Provider id.
        provider: String,
    },
    /// The credential store failed.
    #[error("Credential store modify failed for {provider}")]
    Store {
        /// Provider id.
        provider: String,
        /// Underlying storage error.
        #[source]
        source: AuthStorageError,
    },
}

impl OAuthError {
    /// The message pi's `getApiKeyAndHeaders` reports (the cause's message).
    #[must_use]
    pub fn cause_message(&self) -> String {
        match self {
            Self::RefreshFailed { reason, .. } => reason.clone(),
            Self::ExpiresTooSoon { .. } => self.to_string(),
            Self::Store { source, .. } => source.to_string(),
        }
    }
}

/// Error type of the locked refresh callback.
#[derive(Debug)]
enum RefreshStep {
    Storage(AuthStorageError),
    Failed(String),
}

impl From<AuthStorageError> for RefreshStep {
    fn from(error: AuthStorageError) -> Self {
        Self::Storage(error)
    }
}

/// Current Unix time in milliseconds.
#[must_use]
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// pi `refreshStoredOAuthCredential`: under the lock, re-reads the credential; refreshes it when
/// still an OAuth credential and `needs_refresh` holds; persists the result.
///
/// Returns the OAuth credential after the operation, or `None` when it was logged out meanwhile.
///
/// # Errors
///
/// [`OAuthError::RefreshFailed`] or [`OAuthError::Store`].
pub async fn refresh_stored_oauth(
    storage: &AuthStorage,
    provider: &str,
    refresher: Option<&dyn OAuthRefresher>,
    needs_refresh: impl Fn(&OAuthCredential) -> bool + Send,
) -> Result<Option<OAuthCredential>, OAuthError> {
    let outcome = storage
        .modify_async(provider, |current| async move {
            let Some(AuthCredential::Oauth(current)) = current else {
                return Ok(None);
            };
            if !needs_refresh(&current) {
                return Ok(None);
            }
            let Some(refresher) = refresher else {
                return Err(RefreshStep::Failed(
                    "no OAuth refresher is registered for this provider".to_owned(),
                ));
            };
            let input = AuthCredential::Oauth(current).to_value();
            let output = refresher
                .refresh(provider, input)
                .await
                .map_err(RefreshStep::Failed)?;
            let mut output = output;
            if let JsonValue::Object(map) = &mut output {
                map.insert("type".to_owned(), JsonValue::String("oauth".to_owned()));
            }
            match AuthCredential::from_value(&output) {
                Some(credential @ AuthCredential::Oauth(_)) => Ok(Some(credential)),
                _ => Err(RefreshStep::Failed(
                    "refresh returned an invalid OAuth credential".to_owned(),
                )),
            }
        })
        .await;
    match outcome {
        Ok(Some(AuthCredential::Oauth(credential))) => Ok(Some(credential)),
        Ok(_) => Ok(None),
        Err(RefreshStep::Failed(reason)) => Err(OAuthError::RefreshFailed {
            provider: provider.to_owned(),
            reason,
        }),
        Err(RefreshStep::Storage(source)) => Err(OAuthError::Store {
            provider: provider.to_owned(),
            source,
        }),
    }
}

/// pi `resolveStoredOAuth`: returns a credential valid for at least five minutes (or
/// `min_validity_ms` when larger), refreshing it first when needed. `None` when the credential
/// was removed meanwhile.
///
/// # Errors
///
/// Refresh or storage failures; [`OAuthError::ExpiresTooSoon`] when an explicit minimum is not
/// met after refreshing.
pub async fn ensure_fresh_oauth(
    storage: &AuthStorage,
    provider: &str,
    refresher: Option<&dyn OAuthRefresher>,
    stored: OAuthCredential,
    min_validity_ms: Option<i64>,
) -> Result<Option<OAuthCredential>, OAuthError> {
    let minimum = DEFAULT_OAUTH_MINIMUM_VALIDITY_MS.max(min_validity_ms.unwrap_or(0));
    let expires_soon = move |credential: &OAuthCredential| now_ms() + minimum >= credential.expires;
    if !expires_soon(&stored) {
        return Ok(Some(stored));
    }
    let Some(credential) = refresh_stored_oauth(storage, provider, refresher, expires_soon).await?
    else {
        return Ok(None);
    };
    if min_validity_ms.is_some() && expires_soon(&credential) {
        return Err(OAuthError::ExpiresTooSoon {
            provider: provider.to_owned(),
        });
    }
    Ok(Some(credential))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use indexmap::IndexMap;
    use serde_json::json;

    use super::*;

    struct Counting(AtomicUsize);

    impl OAuthRefresher for Counting {
        fn refresh<'a>(
            &'a self,
            _provider: &'a str,
            credential: JsonValue,
        ) -> BoxFuture<'a, Result<JsonValue, String>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                assert_eq!(credential["refresh"], "r0");
                Ok(json!({"access": "a1", "refresh": "r1", "expires": now_ms() + 3_600_000}))
            })
        }
    }

    fn storage(expires: i64) -> AuthStorage {
        let credential = AuthCredential::from_value(
            &json!({"type": "oauth", "access": "a0", "refresh": "r0", "expires": expires}),
        )
        .unwrap();
        AuthStorage::in_memory(IndexMap::from([("p".to_owned(), credential)]))
    }

    fn stored(storage: &AuthStorage) -> OAuthCredential {
        match storage.stored("p").unwrap() {
            Some(AuthCredential::Oauth(c)) => c,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn refreshes_expiring_tokens_once() {
        let storage = storage(now_ms() + 60_000);
        let refresher = Counting(AtomicUsize::new(0));
        let current = stored(&storage);
        let fresh = futures::executor::block_on(ensure_fresh_oauth(
            &storage,
            "p",
            Some(&refresher),
            current.clone(),
            None,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(fresh.access, "a1");
        assert_eq!(stored(&storage).refresh, "r1");
        // A stale caller re-checks under the lock and does not refresh again.
        let again = futures::executor::block_on(ensure_fresh_oauth(
            &storage,
            "p",
            Some(&refresher),
            current,
            None,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(again.access, "a1");
        assert_eq!(refresher.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn valid_tokens_are_used_directly() {
        let storage = storage(now_ms() + 3_600_000);
        let current = stored(&storage);
        let fresh =
            futures::executor::block_on(ensure_fresh_oauth(&storage, "p", None, current, None))
                .unwrap()
                .unwrap();
        assert_eq!(fresh.access, "a0");
    }

    #[test]
    fn missing_refresher_fails() {
        let storage = storage(0);
        let current = stored(&storage);
        let error =
            futures::executor::block_on(ensure_fresh_oauth(&storage, "p", None, current, None))
                .unwrap_err();
        assert_eq!(error.to_string(), "OAuth refresh failed for p");
    }
}
