// SPDX-License-Identifier: GPL-2.0-only
use crate::error::Error;
use actix_web::{http::StatusCode, HttpResponse, ResponseError};

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
