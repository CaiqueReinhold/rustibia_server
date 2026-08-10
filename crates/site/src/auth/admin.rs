use axum::{extract::FromRequestParts, http::request::Parts};

use crate::{auth::extractor::CurrentAccount, db::accounts, error::AppError, state::AppState};

/// A `CurrentAccount` that additionally holds `is_admin`.
///
/// Admin status is read from the database on every request rather than carried in the
/// session, so revoking it takes effect immediately instead of at the attacker's next
/// login. There is no handler anywhere that sets `is_admin` — see README.
#[derive(Debug, Clone)]
pub struct AdminAccount {
    pub account_id: i32,
}

impl FromRequestParts<AppState> for AdminAccount {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let current = CurrentAccount::from_request_parts(parts, state).await?;

        let account = accounts::find_by_id(&state.pool, current.account_id)
            .await?
            .ok_or(AppError::Unauthenticated)?;

        if !account.is_admin {
            // NotFound rather than Forbidden: a non-admin should not learn that
            // /admin/news exists at all.
            return Err(AppError::NotFound);
        }

        Ok(AdminAccount { account_id: current.account_id })
    }
}
