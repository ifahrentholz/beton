//! Einstellungen des angemeldeten Users (UX-006): Benachrichtigungen.
//!
//! Gespeichert je User auf dem Server, damit sie auf allen Geräten gelten (UX-006 AC3). Das
//! Dokument ist [`NotificationSettings`] (Schema `notification-settings.schema.json`); fehlende
//! Felder bekommen ihren Default, unbekannte lehnt der Server ab.

use axum::extract::State;
use beton_core::notifications::NotificationSettings;

use crate::app::AppState;
use crate::extract::ApiJson;
use crate::problem::{ApiResult, Problem, ProblemCode};

/// Schlüssel der Benachrichtigungs-Einstellungen in `user_settings`.
pub const NOTIFICATIONS_KEY: &str = "notifications";

fn invalid(detail: String) -> Problem {
    Problem::new(ProblemCode::ValidationFailed).detail(detail)
}

/// Gespeicherte Einstellungen; ohne gespeicherten Stand bzw. bei unlesbarem Dokument (etwa nach
/// einem Downgrade) die Defaults.
pub async fn load_notifications(state: &AppState) -> ApiResult<NotificationSettings> {
    let doc = state
        .store
        .user_setting(state.local.org, state.local.user, NOTIFICATIONS_KEY)
        .await?;
    Ok(doc
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

/// Benachrichtigungs-Einstellungen des Users (UX-006).
#[utoipa::path(get, path = "/v1/settings/notifications", tag = "settings",
    responses((status = 200, description = "Matrix Ereignis × Kanal, Ruhezeiten, Vorschau",
        body = Object, content_type = "application/json")))]
pub async fn get_notifications(
    State(state): State<AppState>,
) -> ApiResult<axum::Json<NotificationSettings>> {
    Ok(axum::Json(load_notifications(&state).await?))
}

/// Benachrichtigungs-Einstellungen ersetzen (UX-006). Fehlende Felder bekommen ihren Default.
#[utoipa::path(put, path = "/v1/settings/notifications", tag = "settings",
    request_body(content = Object, description = "NotificationSettings"),
    responses((status = 200, description = "Gespeicherter Stand", body = Object,
        content_type = "application/json")))]
pub async fn put_notifications(
    State(state): State<AppState>,
    ApiJson(settings): ApiJson<NotificationSettings>,
) -> ApiResult<axum::Json<NotificationSettings>> {
    settings.validate().map_err(invalid)?;
    let doc = serde_json::to_value(&settings).map_err(|e| Problem::internal(&e))?;
    state
        .store
        .set_user_setting(state.local.org, state.local.user, NOTIFICATIONS_KEY, &doc)
        .await?;
    Ok(axum::Json(settings))
}
