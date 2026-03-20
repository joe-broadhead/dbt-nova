use serde_json::Value as JsonValue;

use crate::manifest::search::{ManifestSearchHandle, ManifestStatus};

pub(crate) struct ManifestHealthPayload {
    pub ready_for_traffic: bool,
    pub payload: JsonValue,
}

pub(crate) async fn build_manifest_health_payload(
    searcher: &ManifestSearchHandle,
) -> ManifestHealthPayload {
    let status = searcher.status().await;
    let ready_for_traffic = matches!(
        status,
        ManifestStatus::Ready { .. } | ManifestStatus::Refreshing { .. }
    );
    let mut payload = match &status {
        ManifestStatus::Loading { elapsed_ms } => serde_json::json!({
            "status": "loading",
            "elapsed_ms": elapsed_ms,
        }),
        ManifestStatus::Ready { entity_count } => serde_json::json!({
            "status": "ready",
            "entity_count": entity_count,
        }),
        ManifestStatus::Refreshing {
            elapsed_ms,
            entity_count,
        } => serde_json::json!({
            "status": "refreshing",
            "elapsed_ms": elapsed_ms,
            "entity_count": entity_count,
        }),
        ManifestStatus::Failed { error } => serde_json::json!({
            "status": "failed",
            "error": error,
        }),
    };

    if ready_for_traffic && let Ok(active_searcher) = searcher.get().await {
        merge_object_fields(&mut payload, &active_searcher.health_snapshot().await);
    }

    merge_object_fields(&mut payload, &searcher.refresh_stats_snapshot().await);

    ManifestHealthPayload {
        ready_for_traffic,
        payload,
    }
}

fn merge_object_fields(target: &mut JsonValue, source: &JsonValue) {
    if let (Some(target), Some(source)) = (target.as_object_mut(), source.as_object()) {
        for (key, value) in source {
            target.insert(key.clone(), value.clone());
        }
    }
}
