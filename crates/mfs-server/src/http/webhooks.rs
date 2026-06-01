use super::*;
use hmac::{Hmac, Mac};
use mfs_metadata::{StoredWebhookWithSecret, WebhookRecord};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Deserialize)]
pub(super) struct CreateWebhookRequest {
    pub event_type: String,
    pub callback_url: String,
    pub secret: String,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ListWebhooksQuery {
    pub limit: Option<usize>,
}

pub(super) async fn create_webhook(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateWebhookRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    validate_create_request(&request)?;
    let id = format!("webhook_{}", uuid::Uuid::new_v4());
    let enabled = request.enabled.unwrap_or(true);
    let record = WebhookRecord {
        id: &id,
        account_id: &state.config.account_id,
        user_id: &state.config.user_id,
        agent_id: Some(&state.config.agent_id),
        event_type: &request.event_type,
        callback_url: &request.callback_url,
        secret: &request.secret,
        enabled,
    };
    state.metadata.upsert_webhook(&record)?;

    append_audit(
        &state,
        "webhook.create",
        Some(&id),
        Some("{\"result\":\"ok\"}"),
    );

    Ok(Json(serde_json::json!({
        "id": id,
        "account_id": state.config.account_id,
        "user_id": state.config.user_id,
        "agent_id": state.config.agent_id,
        "event_type": request.event_type,
        "callback_url": request.callback_url,
        "enabled": enabled,
    })))
}

pub(super) async fn list_webhooks(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListWebhooksQuery>,
) -> HandlerResult<Json<serde_json::Value>> {
    let limit = query.limit.unwrap_or(100);
    let webhooks =
        state
            .metadata
            .list_webhooks(&state.config.account_id, &state.config.user_id, limit)?;
    let total_count = webhooks.len();
    Ok(Json(serde_json::json!({
        "items": webhooks.clone(),
        "webhooks": webhooks,
        "next_cursor": null,
        "total_count": total_count,
        "count": total_count,
        "limit": limit,
    })))
}

pub(super) async fn delete_webhook(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> HandlerResult<Json<serde_json::Value>> {
    let deleted_count =
        state
            .metadata
            .delete_webhook(&state.config.account_id, &state.config.user_id, &id)?;
    if deleted_count == 0 {
        return Err(AppError(MfsError::NotFound {
            resource: format!("webhook:{id}"),
        }));
    }

    append_audit(
        &state,
        "webhook.delete",
        Some(&id),
        Some("{\"result\":\"ok\"}"),
    );

    Ok(Json(serde_json::json!({
        "deleted": true,
        "id": id,
    })))
}

pub(super) async fn test_webhook(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> HandlerResult<Json<serde_json::Value>> {
    let webhook = state
        .metadata
        .get_webhook_with_secret(&state.config.account_id, &state.config.user_id, &id)?
        .ok_or_else(|| {
            AppError(MfsError::NotFound {
                resource: format!("webhook:{id}"),
            })
        })?;
    let payload = webhook_payload(
        &webhook,
        "test.event",
        None,
        Some(serde_json::json!({ "test": true })),
        true,
    );
    deliver_webhook(&webhook, &payload)
        .await
        .map_err(|reason| {
            AppError(MfsError::Unavailable {
                subsystem: "webhook_delivery".into(),
                reason,
            })
        })?;

    Ok(Json(serde_json::json!({
        "ok": true,
        "id": id,
    })))
}

pub(super) async fn trigger_event(
    metadata: Arc<MetadataStore>,
    account_id: String,
    user_id: String,
    event_type: String,
    subject_uri: Option<String>,
    details_json: Option<String>,
) {
    let Ok(webhooks) = metadata.list_enabled_webhooks_for_event(&account_id, &user_id, &event_type)
    else {
        return;
    };
    let details = details_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
    for webhook in webhooks {
        let payload = webhook_payload(
            &webhook,
            &event_type,
            subject_uri.as_deref(),
            details.clone(),
            false,
        );
        if let Err(error) = deliver_webhook(&webhook, &payload).await {
            tracing::warn!(webhook_id = %webhook.id, event_type = %event_type, error = %error, "webhook delivery failed");
        }
    }
}

fn validate_create_request(request: &CreateWebhookRequest) -> HandlerResult<()> {
    if request.event_type.trim().is_empty() {
        return Err(AppError(MfsError::InvalidArgument {
            field: "event_type".into(),
            reason: "must not be empty".into(),
        }));
    }
    if request.secret.is_empty() {
        return Err(AppError(MfsError::InvalidArgument {
            field: "secret".into(),
            reason: "must not be empty".into(),
        }));
    }
    let url = reqwest::Url::parse(&request.callback_url).map_err(|error| {
        AppError(MfsError::InvalidArgument {
            field: "callback_url".into(),
            reason: error.to_string(),
        })
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError(MfsError::InvalidArgument {
            field: "callback_url".into(),
            reason: "must use http or https".into(),
        }));
    }
    Ok(())
}

fn webhook_payload(
    webhook: &StoredWebhookWithSecret,
    event_type: &str,
    subject_uri: Option<&str>,
    details: Option<serde_json::Value>,
    test: bool,
) -> serde_json::Value {
    serde_json::json!({
        "event_type": event_type,
        "webhook_id": webhook.id,
        "account_id": webhook.account_id,
        "user_id": webhook.user_id,
        "agent_id": webhook.agent_id,
        "subject_uri": subject_uri,
        "details": details,
        "test": test,
        "created_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    })
}

async fn deliver_webhook(
    webhook: &StoredWebhookWithSecret,
    payload: &serde_json::Value,
) -> Result<(), String> {
    let body = serde_json::to_vec(payload).map_err(|error| error.to_string())?;
    let signature = sign_body(&webhook.secret, &body)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| error.to_string())?;
    client
        .post(&webhook.callback_url)
        .header("content-type", "application/json")
        .header("X-MemFuse-Signature", signature)
        .body(body)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn sign_body(secret: &str, body: &[u8]) -> Result<String, String> {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).map_err(|error| error.to_string())?;
    mac.update(body);
    Ok(format!(
        "sha256={}",
        hex::encode(mac.finalize().into_bytes())
    ))
}
