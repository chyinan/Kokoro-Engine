use reqwest::StatusCode;
use std::time::Duration;

/// retries a request closure with exponential backoff
/// Retries on:
/// - Network errors
/// - 429 Too Many Requests (respects Retry-After header)
/// - 5xx Server Errors
///
/// Returns the last Response (even if error status) or the last Network Error as String.
pub async fn request_with_retry<F, Fut>(
    mut task: F,
    max_retries: u32,
) -> Result<reqwest::Response, String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<reqwest::Response, reqwest::Error>>,
{
    let mut attempt = 0;
    let mut delay = Duration::from_millis(1000);

    loop {
        attempt += 1;
        match task().await {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    return Ok(response);
                }

                // If we exhausted retries, return the error response for the caller to parse
                if attempt > max_retries {
                    return Ok(response);
                }

                // Retry on rate limit or server error
                if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                    // 优先使用 Retry-After 头指定的等待时间
                    let retry_delay = response
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<u64>().ok())
                        .map(Duration::from_secs)
                        .unwrap_or(delay);

                    eprintln!(
                        "[HTTP] Request failed with status {}, retrying in {:?} (attempt {}/{})",
                        status, retry_delay, attempt, max_retries
                    );
                    tokio::time::sleep(retry_delay).await;
                    delay = std::cmp::min(delay * 2, Duration::from_secs(60)); // Cap at 60s
                    continue;
                }

                // Other client errors (400, 401, 404) are likely permanent, return immediately
                return Ok(response);
            }
            Err(e) => {
                if attempt > max_retries {
                    return Err(format!(
                        "Network request failed after {} attempts: {}",
                        max_retries, e
                    ));
                }
                eprintln!(
                    "[HTTP] Network error: {}, retrying in {:?} (attempt {}/{})",
                    e, delay, attempt, max_retries
                );
                tokio::time::sleep(delay).await;
                delay = std::cmp::min(delay * 2, Duration::from_secs(60));
            }
        }
    }
}
