use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tower_http::cors::{Any, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::core::{Core, HOST, PORT, WS_PATH};
use crate::system;
use crate::types::{JobSource, PrintPayload};

type Shared = Arc<Core>;

pub fn router(core: Shared) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/printers", get(printers))
        .route("/jobs", get(jobs))
        .route("/print", post(print))
        .route(WS_PATH, get(ws_upgrade))
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
        // Chrome requires this for public pages calling a loopback service.
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("access-control-allow-private-network"),
            HeaderValue::from_static("true"),
        ))
        .with_state(core)
}

pub async fn serve(core: Shared) -> std::io::Result<()> {
    let addr: SocketAddr = format!("{HOST}:{PORT}").parse().expect("valid listen address");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(core)).await
}

async fn health(State(core): State<Shared>) -> Json<Value> {
    let mut info = core.server_info();
    info["success"] = json!(true);
    Json(info)
}

async fn printers() -> Json<Value> {
    Json(json!({ "success": true, "printers": system::list_printers().await }))
}

async fn jobs(State(core): State<Shared>) -> Json<Value> {
    Json(json!({ "success": true, "jobs": core.jobs() }))
}

async fn print(State(core): State<Shared>, body: axum::body::Bytes) -> impl IntoResponse {
    let payload: Result<PrintPayload, _> = if body.is_empty() {
        Ok(PrintPayload::default())
    } else {
        serde_json::from_slice(&body)
    };
    match payload {
        Ok(payload) => (StatusCode::OK, Json(json!(core.print_html(payload, JobSource::Http).await))),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "success": false, "message": e.to_string() }))),
    }
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(core): State<Shared>) -> impl IntoResponse {
    ws.max_message_size(64 * 1024 * 1024).on_upgrade(move |socket| ws_session(socket, core))
}

async fn ws_session(mut socket: WebSocket, core: Shared) {
    while let Some(Ok(message)) = socket.recv().await {
        let Message::Text(text) = message else {
            if matches!(message, Message::Close(_)) {
                break;
            }
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&text) {
            Err(e) => Some(json!({ "cmd": "error", "success": false, "message": e.to_string() })),
            Ok(msg) => handle_ws_message(&core, msg).await,
        };
        if let Some(reply) = reply {
            if socket.send(Message::Text(reply.to_string().into())).await.is_err() {
                break;
            }
        }
    }
}

async fn handle_ws_message(core: &Shared, msg: Value) -> Option<Value> {
    let request_id = msg.get("requestId").cloned().unwrap_or(Value::Null);
    match msg.get("cmd").and_then(Value::as_str)? {
        "printReport" => {
            let payload = match serde_json::from_value::<PrintPayload>(msg.get("payload").cloned().unwrap_or_else(|| json!({}))) {
                Ok(p) => p,
                Err(e) => return Some(json!({ "cmd": "error", "success": false, "message": e.to_string() })),
            };
            let mut reply = json!(core.print_html(payload, JobSource::Ws).await);
            reply["cmd"] = json!("printReportResult");
            reply["requestId"] = request_id;
            Some(reply)
        }
        "health" => {
            let mut reply = core.server_info();
            reply["cmd"] = json!("healthResult");
            reply["requestId"] = request_id;
            reply["success"] = json!(true);
            Some(reply)
        }
        _ => None,
    }
}
