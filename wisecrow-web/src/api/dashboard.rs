use dioxus::prelude::*;
use wisecrow_dto::DashboardDto;

/// Rolls up the signed-in learner's progress in every language they have
/// touched.
///
/// # Errors
///
/// Returns authentication or sanitized storage errors.
#[post("/api/dashboard")]
pub async fn dashboard() -> Result<DashboardDto, ServerFnError> {
    let user = crate::server::auth::current_user().await?;
    let languages = wisecrow::progress::ProgressQuery::dashboard(crate::server::pool()?, user.id)
        .await
        .map_err(|error| crate::server::internal_error("progress dashboard", &error))?;
    Ok(wisecrow::dto_convert::dashboard(&languages))
}
