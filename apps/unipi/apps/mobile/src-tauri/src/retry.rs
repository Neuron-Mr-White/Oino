use std::future::Future;
use unipi_proto::{AppErrorCode, AppErrorDto};

pub async fn retry_once<T, F, Fut>(mut attempt: F) -> Result<T, AppErrorDto>
where
    F: FnMut(bool) -> Fut,
    Fut: Future<Output = Result<T, AppErrorDto>>,
{
    match attempt(false).await {
        Err(error) if error.code == AppErrorCode::E_TRANSPORT => attempt(true).await,
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn replaces_failed_transport_once() {
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let result = retry_once(|fresh| {
            attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move {
                if fresh {
                    Ok(42)
                } else {
                    Err(AppErrorDto::new(AppErrorCode::E_TRANSPORT, "closed", true))
                }
            }
        })
        .await;
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
    #[tokio::test]
    async fn does_not_retry_application_errors() {
        let mut attempts = 0;
        let _: Result<(), _> = retry_once(|_| {
            attempts += 1;
            async {
                Err(AppErrorDto::new(
                    AppErrorCode::E_UNAUTHORIZED,
                    "denied",
                    false,
                ))
            }
        })
        .await;
        assert_eq!(attempts, 1);
    }
}
