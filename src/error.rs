use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

pub struct AppError(StatusCode, anyhow::Error);

impl AppError {
    pub fn bad_request(msg: impl std::fmt::Display) -> Self {
        Self(StatusCode::BAD_REQUEST, anyhow::anyhow!("{}", msg))
    }

    pub fn not_found(msg: impl std::fmt::Display) -> Self {
        Self(StatusCode::NOT_FOUND, anyhow::anyhow!("{}", msg))
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = if self.0 == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!("{:#}", self.1);
            (self.0, "Internal server error".to_string())
        } else {
            (self.0, self.1.to_string())
        };
        (status, axum::Json(json!({"error": message}))).into_response()
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(StatusCode::INTERNAL_SERVER_ERROR, err.into())
    }
}
