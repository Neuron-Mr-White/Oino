//! Provider-level retries (pi-ai `utils/provider-retry.js`, mirroring the `OpenAI`/Anthropic SDK
//! policy). The caller sleeps; this module only decides.

const DEFAULT_MAX_RETRY_DELAY_MS: u64 = 60_000;

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

/// pi `isRetryableProviderError`; `status` is `None` for transport failures.
#[must_use]
pub fn is_retryable(status: Option<u16>, headers: &[(String, String)]) -> bool {
    match header(headers, "x-should-retry") {
        Some("true") => return true,
        Some("false") => return false,
        _ => {}
    }
    match status {
        None => true,
        Some(status) => matches!(status, 408 | 409 | 429) || status >= 500,
    }
}

/// pi `getRetryDelayMs`: server-requested delay (`retry-after-ms`, `retry-after` seconds) or
/// exponential backoff `min(0.5·2^i, 8)s · (1 − jitter·0.25)` with `jitter ∈ [0, 1)`.
///
/// # Errors
/// When the server asks for more than `max_retry_delay_ms` (default 60 s; 0 disables the cap).
pub fn retry_delay_ms(
    headers: &[(String, String)],
    retry_index: u32,
    max_retry_delay_ms: Option<u64>,
    jitter: f64,
    error_message: &str,
) -> Result<u64, String> {
    let validate = |delay: f64| -> Result<u64, String> {
        let max = max_retry_delay_ms.unwrap_or(DEFAULT_MAX_RETRY_DELAY_MS);
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "delays are small non-negative millisecond values"
        )]
        {
            if max > 0 && delay > max as f64 {
                return Err(format!(
                    "Server requested {}s retry delay (max: {}s). {error_message}",
                    (delay / 1000.0).ceil() as u64,
                    max.div_ceil(1000)
                ));
            }
            Ok(delay.max(0.0) as u64)
        }
    };
    if let Some(value) =
        header(headers, "retry-after-ms").and_then(|v| v.trim().parse::<f64>().ok())
        && value.is_finite()
    {
        return validate(value);
    }
    if let Some(value) = header(headers, "retry-after").and_then(|v| v.trim().parse::<f64>().ok())
        && value.is_finite()
    {
        return validate(value * 1000.0);
    }
    let exponential =
        (0.5 * 2f64.powi(i32::try_from(retry_index).unwrap_or(i32::MAX))).min(8.0) * 1000.0;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "bounded positive delay"
    )]
    let delay = (exponential * (1.0 - jitter * 0.25)) as u64;
    Ok(delay)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy() {
        assert!(is_retryable(Some(429), &[]));
        assert!(is_retryable(Some(503), &[]));
        assert!(!is_retryable(Some(400), &[]));
        assert!(is_retryable(None, &[]));
        assert!(!is_retryable(
            Some(500),
            &[("x-should-retry".into(), "false".into())]
        ));
        assert_eq!(retry_delay_ms(&[], 0, None, 0.0, ""), Ok(500));
        assert_eq!(retry_delay_ms(&[], 10, None, 0.0, ""), Ok(8000));
        assert_eq!(
            retry_delay_ms(&[("retry-after".into(), "2".into())], 0, None, 0.0, ""),
            Ok(2000)
        );
        assert!(
            retry_delay_ms(&[("retry-after".into(), "120".into())], 0, None, 0.0, "x").is_err()
        );
    }
}
