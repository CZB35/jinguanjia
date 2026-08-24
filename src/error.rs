use axum::{
    Json,
    extract::multipart::MultipartError,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use thiserror::Error;

use crate::response::ApiResponse;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    PayloadTooLarge(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    pub fn from_multipart_error(error: MultipartError) -> Self {
        Self::from_multipart_error_with_limit_message(error, "上传文件大小不能超过10MB")
    }

    pub fn from_multipart_error_with_limit_message(
        error: MultipartError,
        limit_message: &'static str,
    ) -> Self {
        match error.status() {
            StatusCode::PAYLOAD_TOO_LARGE => Self::PayloadTooLarge(limit_message.to_string()),
            StatusCode::BAD_REQUEST => Self::BadRequest(error.body_text()),
            _ => Self::Internal(anyhow::anyhow!(error.body_text())),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, 1001, message),
            Self::Unauthorized(message) => (StatusCode::UNAUTHORIZED, 1006, message),
            Self::Forbidden(message) => (StatusCode::FORBIDDEN, 1007, message),
            Self::NotFound(message) => (StatusCode::NOT_FOUND, 2001, message),
            Self::Conflict(message) => (StatusCode::CONFLICT, 2002, message),
            Self::PayloadTooLarge(message) => (StatusCode::PAYLOAD_TOO_LARGE, 1008, message),
            Self::Internal(error) => {
                tracing::error!(error = %error, "unhandled internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    9000,
                    "系统开小差了，请稍后重试".to_string(),
                )
            }
        };

        (status, Json(ApiResponse::<()>::error(code, message))).into_response()
    }
}
