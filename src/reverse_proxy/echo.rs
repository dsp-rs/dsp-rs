use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Request, WebSocketUpgrade, ws::Message},
    response::IntoResponse,
    routing::{any, get},
};
use futures::{SinkExt, StreamExt};
use serde::Serialize;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::trace::TraceLayer;
use tracing::{debug, error, info};
use url::form_urlencoded;

use base64::prelude::*;

use crate::AppError;

pub(crate) async fn webserver(token: CancellationToken) {
    info!("Started Echo server");

    let app = Router::new()
        // metadata
        .route("/api/{*path}", any(echo))
        .route("/ws/{*path}", get(echo_ws))
        .layer(TraceLayer::new_for_http());

    let listener = TcpListener::bind("0.0.0.0:4000").await.unwrap();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            _ = token.cancelled().await;
        })
        .await
        .unwrap();

    info!("Terminated Echo server");
}

#[derive(Serialize)]
struct EchoResponse {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: String, // Base64 encoded
}

async fn echo(req: Request) -> Result<impl IntoResponse, AppError> {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();

    let query: HashMap<String, String> = req
        .uri()
        .query()
        .map(|v| {
            form_urlencoded::parse(v.as_bytes())
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect()
        })
        .unwrap_or_default();

    let mut headers = HashMap::new();
    for (name, value) in req.headers().iter() {
        headers.insert(
            name.as_str().to_string(),
            value.to_str().unwrap_or("<non-utf8>").to_string(),
        );
    }

    let (_, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .unwrap_or_default();

    let body = BASE64_STANDARD.encode(bytes);

    Ok(Json(EchoResponse {
        method,
        path,
        query,
        headers,
        body,
    }))
}

async fn echo_ws(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|socket| async move {
        debug!("Client connected");

        let (mut sender, mut receiver) = socket.split();

        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Text(text) => {
                    debug!("Received text: {}", text);
                    if let Err(err) = sender
                        .send(Message::Text(format!("Echo: {}", text).into()))
                        .await
                    {
                        error!("Error sending text message: {err}");
                        break;
                    }
                }
                Message::Binary(bin) => {
                    debug!("Received binary: {} bytes", bin.len());
                    if let Err(err) = sender.send(Message::Binary(bin)).await {
                        error!("Error sending binary message: {err}");
                        break;
                    }
                }
                Message::Close(_) => {
                    println!("Client disconnected");
                    break;
                }
                _ => {} // all other message types are ignored
            }
        }
    })
}
