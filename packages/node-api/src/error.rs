//! Standard error response body (spec §4.2).

use serde::{Deserialize, Serialize};

/// The JSON body returned for all error responses.
///
/// ```json
/// { "error": "unit id already exists with different content", "code": "id_conflict" }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorResponse {
    /// Human-readable description of the problem.
    pub error: String,

    /// Machine-readable error code.
    ///
    /// Defined values (see spec §5 for per-endpoint codes):
    ///
    /// | `code` | HTTP status |
    /// |--------|------------|
    /// | `invalid_json` | 400 |
    /// | `invalid_parameter` | 400 |
    /// | `signing_required` | 401 |
    /// | `not_found` | 404 |
    /// | `id_conflict` | 409 |
    /// | `validation_failed` | 422 |
    /// | `pow_required` | 428 |
    /// | `rate_limit_exceeded` | 429 |
    /// | `internal_error` | 500 |
    pub code: String,
}

impl ErrorResponse {
    /// Construct an [`ErrorResponse`] from a static code and message.
    pub fn new(code: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            error: error.into(),
        }
    }
}

/// Well-known error codes defined by the spec.
pub mod codes {
    pub const INVALID_JSON: &str = "invalid_json";
    pub const INVALID_PARAMETER: &str = "invalid_parameter";
    pub const SIGNING_REQUIRED: &str = "signing_required";
    pub const NOT_FOUND: &str = "not_found";
    pub const ID_CONFLICT: &str = "id_conflict";
    pub const VALIDATION_FAILED: &str = "validation_failed";
    pub const POW_REQUIRED: &str = "pow_required";
    pub const RATE_LIMIT_EXCEEDED: &str = "rate_limit_exceeded";
    pub const INTERNAL_ERROR: &str = "internal_error";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let e = ErrorResponse::new(codes::VALIDATION_FAILED, "content must not be empty");
        let json = serde_json::to_string(&e).unwrap();
        let back: ErrorResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }
}
