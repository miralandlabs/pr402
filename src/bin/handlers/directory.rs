use super::*;

pub const DIRECTORY_STATS: &str = "/api/v1/facilitator/directory/stats";

/// Public aggregate counts for the seller + resource directories (same visibility as list endpoints).
pub async fn handle_directory_stats() -> Response<Body> {
    let Some(db) = pr402_db() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "Public directory requires DATABASE_URL to be configured.",
        );
    };

    match db.fetch_public_directory_stats().await {
        Ok(stats) => {
            let body = serde_json::json!({
                "network": solana_network_from_chain(),
                "providers": { "total": stats.provider_total },
                "resources": {
                    "total": stats.resource_total,
                    "byScheme": stats.resources_by_scheme,
                },
                "asOf": pr402::db::format_system_time_rfc3339(std::time::SystemTime::now()),
            });
            facilitator_response!()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Body::Text(body.to_string()))
                .unwrap()
        }
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("directory stats failed: {}", e),
        ),
    }
}
