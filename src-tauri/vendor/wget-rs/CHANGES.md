# Fixes for 403 Download Issue

## Problem Description
The original issue was that downloading certain files would result in getting HTML pages with "403 Forbidden" content instead of the actual file. This was caused by servers rejecting requests that didn't have proper User-Agent headers.

## Changes Made

### 1. Added Proper User-Agent Header
- Created `create_client()` function that builds an HTTP client with a proper User-Agent
- User-Agent format: `Wget/{version} ({os})` (e.g., "Wget/0.1.0 (linux)")
- This mimics the behavior of the original wget tool

### 2. Enable Automatic Redirect Following  
- Configured client to follow up to 10 HTTP redirects automatically
- Many servers use redirects that need to be followed to get the actual file

### 3. Improved Error Detection
- Added `validate_response()` function to detect HTML error pages
- Checks if the server returns HTML content when a binary file is expected
- Provides clear error messages when this occurs

### 4. Better Error Messages
- Enhanced HTTP error reporting with status codes and descriptions
- More informative error messages for debugging

## Technical Details

### User-Agent Configuration
```rust
fn create_client() -> Result<Client, Box<dyn std::error::Error>> {
    let user_agent = format!("Wget/{} ({})", env!("CARGO_PKG_VERSION"), std::env::consts::OS);
    Client::builder()
        .user_agent(user_agent)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| e.into())
}
```

### Response Validation
```rust
fn validate_response(response: &reqwest::blocking::Response, expected_filename: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Check HTTP status
    if !response.status().is_success() {
        return Err(format!("HTTP error: {} - {}", response.status().as_u16(), response.status().canonical_reason().unwrap_or("Unknown")).into());
    }
    
    // Check for HTML error pages when expecting binary files
    if let Some(content_type) = response.headers().get(CONTENT_TYPE) {
        if let Ok(content_type_str) = content_type.to_str() {
            if content_type_str.starts_with("text/html") && !expected_filename.ends_with(".html") && !expected_filename.ends_with(".htm") {
                return Err("服务器返回了 HTML 页面而不是预期的文件，可能是 403 或其他错误页面".into());
            }
        }
    }
    
    Ok(())
}
```

## Testing
- All existing tests continue to pass
- Added tests for User-Agent functionality
- Added integration tests to verify version and OS constants
- Added test for content type validation

## Expected Impact
These changes should resolve the 403 Forbidden issue by:
1. Sending proper identification headers that servers expect
2. Following redirects that might be required
3. Detecting and reporting when servers return error pages instead of files
4. Providing better error messages for troubleshooting

The implementation maintains compatibility with existing functionality while adding the necessary HTTP client improvements.