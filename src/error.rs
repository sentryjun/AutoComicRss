// SPDX-License-Identifier: GPL-2.0-only
use actix_web::{http::StatusCode, HttpResponse, ResponseError};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("resource not found")]
    NotFound,
    #[error("a scan or download synchronization is already running")]
    Busy,
    #[error("too many Mikan requests; retry shortly")]
    SourceBusy,
    #[error("{0}")]
    Upstream(String),
    #[error("database operation failed")]
    Database(#[from] sqlx::Error),
    #[error("stored data could not be decoded")]
    Json(#[from] serde_json::Error),
    #[error("database migration failed")]
    Migration(#[from] sqlx::migrate::MigrateError),
}

impl From<reqwest::Error> for Error {
    fn from(value: reqwest::Error) -> Self {
        Self::Upstream(value.without_url().to_string())
    }
}

impl ResponseError for Error {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::Invalid(_) => StatusCode::BAD_REQUEST,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Busy => StatusCode::CONFLICT,
            Self::SourceBusy => StatusCode::TOO_MANY_REQUESTS,
            Self::Upstream(_) => StatusCode::BAD_GATEWAY,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status_code()).json(serde_json::json!({"error": self.to_string()}))
    }
}
