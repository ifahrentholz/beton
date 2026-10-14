//! Inbox (UX-001): offene Freigaben und Fragen aller Sessions, direkt beantwortbar – auch für
//! pausierte Sessions ohne Zuschauer –, dazu Hinweise aus `notify`.
//!
//! Die Einträge sind eine Projektion des Event-Logs (`beton_store::InboxRecord`). Entschieden
//! wird über dieselbe Logik wie in der Session (`approval.resolve`); die Inbox nennt nur den
//! Eintrag statt Session und Approval-ID.

use axum::Extension;
use axum::extract::{Path, State};
use beton_store::{InboxQuery, InboxRecord};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
use utoipa::{IntoParams, ToSchema};

use crate::api_sessions::ResolveApprovalRequest;
use crate::app::AppState;
use crate::extract::{ApiJson, ApiQuery};
use crate::problem::{ApiResult, Problem, ProblemCode};
use crate::security::Authenticated;

/// Ein Eintrag der Inbox.
#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct InboxItemView {
    pub id: String,
    pub session_id: String,
    pub session_title: String,
    /// `approval`, `question` oder `notice`.
    pub kind: String,
    /// `open` (wartet auf dich), `done` (entschieden) oder `info` (Hinweis).
    pub status: String,
    #[ts(optional)]
    pub approval_id: Option<String>,
    pub title: String,
    pub summary: String,
    /// Freigaben und Fragen: `subject`, `options`, `expires_at`, `on_timeout`,
    /// `approval_kind`; Hinweise: `rule_id`, `scope`, `set`, `channels`.
    #[schema(value_type = Object)]
    #[ts(type = "unknown")]
    pub detail: Value,
    pub created_seq: u64,
    /// RFC 3339.
    pub created_at: String,
    #[ts(optional)]
    pub resolved_at: Option<String>,
    /// `decision`, `via`, `actor`, `comment?`, `answer?`, `remember?`, `on_timeout_applied?`.
    #[schema(value_type = Option<Object>)]
    #[ts(optional, type = "unknown")]
    pub resolution: Option<Value>,
}

impl From<InboxRecord> for InboxItemView {
    fn from(r: InboxRecord) -> Self {
        Self {
            id: r.id,
            session_id: r.session_id.to_string(),
            session_title: r.session_title,
            kind: r.kind,
            status: r.status,
            approval_id: r.approval_id,
            title: r.title,
            summary: r.summary,
            detail: r.detail,
            created_seq: r.created_seq,
            created_at: r.created_at.to_string(),
            resolved_at: r.resolved_at.map(|t| t.to_string()),
            resolution: r.resolution,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema, TS)]
pub struct InboxPage {
    pub items: Vec<InboxItemView>,
    /// Zähler der Inbox: offene Freigaben und Fragen (UX-001 AC3).
    pub open_count: u64,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct InboxParams {
    /// `open`, `done` oder `info`; ohne Angabe alle.
    pub status: Option<String>,
    /// `approval`, `question` oder `notice`.
    pub kind: Option<String>,
    pub session_id: Option<String>,
    /// 1 bis 500 (Default 100).
    pub limit: Option<String>,
}

fn invalid(detail: String) -> Problem {
    Problem::new(ProblemCode::ValidationFailed).detail(detail)
}

/// Inbox, neueste Einträge zuerst (UX-001).
#[utoipa::path(get, path = "/v1/inbox", tag = "inbox",
    params(InboxParams),
    responses((status = 200, description = "Einträge und Zähler", body = InboxPage)))]
pub async fn list_inbox(
    State(state): State<AppState>,
    ApiQuery(p): ApiQuery<InboxParams>,
) -> ApiResult<axum::Json<InboxPage>> {
    if let Some(s) = &p.status
        && !matches!(s.as_str(), "open" | "done" | "info")
    {
        return Err(invalid(format!(
            "status `{s}` ist ungültig (open, done, info)"
        )));
    }
    if let Some(k) = &p.kind
        && !matches!(k.as_str(), "approval" | "question" | "notice")
    {
        return Err(invalid(format!(
            "kind `{k}` ist ungültig (approval, question, notice)"
        )));
    }
    let session = match &p.session_id {
        Some(s) => Some(
            s.parse()
                .map_err(|_| invalid(format!("session_id `{s}` ist ungültig")))?,
        ),
        None => None,
    };
    let limit = match &p.limit {
        None => 100,
        Some(raw) => raw
            .parse::<u32>()
            .ok()
            .filter(|l| (1..=500).contains(l))
            .ok_or_else(|| {
                Problem::new(ProblemCode::InvalidLimit).detail("limit muss 1 bis 500 sein")
            })?,
    };
    let org = state.local.org;
    let items = state
        .store
        .inbox(
            org,
            &InboxQuery {
                status: p.status,
                kind: p.kind,
                session,
                limit,
            },
        )
        .await?
        .into_iter()
        .map(InboxItemView::from)
        .collect();
    Ok(axum::Json(InboxPage {
        items,
        open_count: state.store.inbox_open_count(org).await?,
    }))
}

/// Ein Eintrag der Inbox.
#[utoipa::path(get, path = "/v1/inbox/{item_id}", tag = "inbox",
    params(("item_id" = String, Path)),
    responses((status = 200, description = "Eintrag", body = InboxItemView)))]
pub async fn get_inbox_item(
    State(state): State<AppState>,
    Path(item_id): Path<String>,
) -> ApiResult<axum::Json<InboxItemView>> {
    let item = state.store.inbox_item(state.local.org, &item_id).await?;
    Ok(axum::Json(item.into()))
}

/// Freigabe oder Frage aus der Inbox entscheiden (UX-001 AC2): setzt eine pausierte Session
/// fort, ohne dass sie geöffnet wurde.
#[utoipa::path(post, path = "/v1/inbox/{item_id}/resolve", tag = "inbox",
    params(("item_id" = String, Path)),
    request_body = ResolveApprovalRequest,
    responses((status = 200, description = "Entscheidung zugestellt", body = Object)))]
pub async fn resolve_inbox_item(
    State(state): State<AppState>,
    Extension(_auth): Extension<Authenticated>,
    Path(item_id): Path<String>,
    ApiJson(req): ApiJson<ResolveApprovalRequest>,
) -> ApiResult<axum::Json<Value>> {
    let item = state.store.inbox_item(state.local.org, &item_id).await?;
    let approval = match (item.status.as_str(), &item.approval_id) {
        ("open", Some(a)) => a.clone(),
        ("done", Some(_)) => {
            return Err(Problem::new(ProblemCode::Conflict)
                .detail("Dieser Eintrag ist schon entschieden".to_owned()));
        }
        _ => {
            return Err(Problem::new(ProblemCode::Conflict)
                .detail("Hinweise kann man nicht entscheiden".to_owned()));
        }
    };
    let args = serde_json::to_value(&req).map_err(|e| Problem::internal(&e))?;
    state
        .sessions()
        .resolve_approval(
            item.session_id,
            &approval,
            args,
            beton_core::id::PrincipalId::User(beton_core::id::UserId::LOCAL),
        )
        .await?;
    Ok(axum::Json(serde_json::json!({})))
}
