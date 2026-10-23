use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use thiserror::Error;

pub type AppResult<T> = Result<T, AppError>;

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

    #[error("Error interno de base de datos")]
    Database(#[from] sqlx::Error),

    #[error("Error interno del servidor: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg.clone()),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            AppError::Database(sqlx::Error::RowNotFound) => (
                StatusCode::NOT_FOUND,
                "El recurso solicitado no existe".to_string(),
            ),
            AppError::Database(sqlx::Error::Database(db_err)) => {
                // Código 23505 en PostgreSQL / YugabyteDB = unique_violation
                if db_err.code().as_deref() == Some("23505") {
                    (
                        StatusCode::CONFLICT,
                        "Ya existe un registro con esos datos únicos".to_string(),
                    )
                } else {
                    tracing::error!(error = ?db_err, "Database error");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Error interno en la base de datos".to_string(),
                    )
                }
            }
            AppError::Database(err) => {
                tracing::error!(error = ?err, "Unexpected SQLx error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Error interno en la base de datos".to_string(),
                )
            }
            AppError::Internal(msg) => {
                tracing::error!(error = %msg, "Internal server error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Error interno del servidor".to_string(),
                )
            }
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}

