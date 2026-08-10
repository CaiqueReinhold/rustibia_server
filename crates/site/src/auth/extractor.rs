use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts},
};

use crate::{db::sessions, error::AppError, state::AppState};

/// Resolved from either the `session` cookie (website) or an
/// `Authorization: Bearer` header (game client). One extractor serves both surfaces.
#[derive(Debug, Clone)]
pub struct CurrentAccount {
    pub account_id: i32,
    /// The token this request authenticated with, cookie or bearer. Handlers that
    /// need to keep the caller's own session alive use this rather than re-deriving
    /// it — re-deriving is how the two paths drifted apart.
    pub session_token: String,
}

impl FromRequestParts<AppState> for CurrentAccount {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts)
            .or_else(|| cookie_token(parts))
            .ok_or(AppError::Unauthenticated)?;

        let account_id = sessions::account_for_token(&state.pool, &token)
            .await?
            .ok_or(AppError::Unauthenticated)?;

        Ok(CurrentAccount { account_id, session_token: token })
    }
}

fn bearer_token(parts: &Parts) -> Option<String> {
    let value = parts.headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ").map(|t| t.trim().to_string())
}

fn cookie_token(parts: &Parts) -> Option<String> {
    let cookies = parts.headers.get(header::COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name.trim() == "session").then(|| value.trim().to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;

    fn parts_with(headers: &[(&str, &str)]) -> Parts {
        let mut builder = Request::builder().uri("/");
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        builder.body(()).unwrap().into_parts().0
    }

    #[test]
    fn reads_a_bearer_token() {
        let parts = parts_with(&[("authorization", "Bearer abc123")]);
        assert_eq!(bearer_token(&parts), Some("abc123".to_string()));
    }

    #[test]
    fn ignores_a_non_bearer_authorization_scheme() {
        let parts = parts_with(&[("authorization", "Basic abc123")]);
        assert_eq!(bearer_token(&parts), None);
    }

    #[test]
    fn reads_the_session_cookie_among_others() {
        let parts = parts_with(&[("cookie", "theme=dark; session=abc123; lang=en")]);
        assert_eq!(cookie_token(&parts), Some("abc123".to_string()));
    }

    #[test]
    fn returns_none_when_no_session_cookie_is_present() {
        let parts = parts_with(&[("cookie", "theme=dark")]);
        assert_eq!(cookie_token(&parts), None);
    }

    #[test]
    fn returns_none_when_there_are_no_headers_at_all() {
        let parts = parts_with(&[]);
        assert_eq!(bearer_token(&parts), None);
        assert_eq!(cookie_token(&parts), None);
    }
}
