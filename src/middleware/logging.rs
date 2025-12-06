use axum::{
    body::{to_bytes, Body, Bytes},
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::time::Instant;

/// Middleware that logs request and response bodies
pub async fn logging_middleware(request: Request, next: Next) -> Response {
    let request_id = uuid::Uuid::new_v4();
    let method = request.method().clone();
    let uri = request.uri().clone();
    let start = Instant::now();

    // Extract and log request body
    let (parts, body) = request.into_parts();

    // Read the request body (limit to 1MB to prevent memory issues)
    let bytes = match to_bytes(body, 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!(request_id = %request_id, "Failed to read request body: {}", e);
            return (StatusCode::BAD_REQUEST, "Failed to read request body").into_response();
        }
    };

    // Log request with body (truncate if too long)
    let request_body = String::from_utf8_lossy(&bytes);
    let truncated_request = truncate_body(&request_body, 2000);

    tracing::info!(
        request_id = %request_id,
        method = %method,
        uri = %uri,
        body = %truncated_request,
        "→ Request"
    );

    // Reconstruct the request with the body
    let request = Request::from_parts(parts, Body::from(bytes));

    // Call the next middleware/handler
    let response = next.run(request).await;

    // Extract response status before consuming body
    let status = response.status();
    let (parts, body) = response.into_parts();

    // Read the response body (limit to 1MB)
    let bytes = match to_bytes(body, 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!(request_id = %request_id, "Failed to read response body: {}", e);
            Bytes::new()
        }
    };

    // Log response with body (truncate if too long)
    let response_body = String::from_utf8_lossy(&bytes);
    let truncated_response = truncate_body(&response_body, 2000);
    let latency = start.elapsed();

    tracing::info!(
        request_id = %request_id,
        method = %method,
        uri = %uri,
        status = %status.as_u16(),
        latency_ms = %latency.as_millis(),
        body = %truncated_response,
        "← Response"
    );

    // Reconstruct the response with the body
    Response::from_parts(parts, Body::from(bytes))
}

/// Truncate body for logging, adding ellipsis if truncated
fn truncate_body(body: &str, max_len: usize) -> String {
    let body = body.trim();
    if body.len() <= max_len {
        body.to_string()
    } else {
        format!(
            "{}...[truncated, {} bytes total]",
            &body[..max_len],
            body.len()
        )
    }
}
