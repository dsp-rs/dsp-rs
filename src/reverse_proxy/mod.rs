use anyhow::Context;
use axum::{
    body::Body,
    extract::{FromRequest, Request, State, WebSocketUpgrade, ws::CloseFrame},
    http::{HeaderMap, Response, StatusCode},
};
use futures::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use reqwest::Client;
use reqwest_websocket::Upgrade;
use tracing::error;

use crate::{
    AppError,
    auth::extractor::AuthClaims,
    connector::{Pending, app_state::AppStateReverseProxy},
    store::{RemoteAddress, Store},
    transfer::{Transfer, TransferState},
};

#[cfg(feature = "echo")]
pub(crate) mod echo;

fn unauthorized() -> Response<Body> {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .body(Body::empty())
        .expect("empty body and valid status should not fail")
}

pub(crate) async fn handler<T: Store>(
    claims: AuthClaims,
    State(state): State<AppStateReverseProxy<T>>,
    req: Request,
) -> Result<Response<Body>, AppError> {
    let headers = req.headers().clone();
    let target = match find_data_asset_target(&state, claims).await {
        Ok(Some(target)) => target,
        Ok(None) => {
            error!("Could not determine target dataset");
            return Ok(unauthorized());
        }
        Err(err) => {
            error!("Failed to determine target dataset, error: {err}");
            return Ok(unauthorized());
        }
    };

    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());

    let is_websocket = headers
        .get(axum::http::header::UPGRADE)
        .and_then(|val| val.to_str().ok())
        .map(|val| val.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);

    if is_websocket {
        let ws_url = match &target.url[..] {
            u if u.starts_with("ws://") || u.starts_with("wss://") => {
                format!("{}{}", u, path_and_query)
            }
            u if u.starts_with("http://") => format!("ws://{}{}", &u[7..], path_and_query),
            u if u.starts_with("https://") => format!("wss://{}{}", &u[8..], path_and_query),
            _ => {
                error!(
                    "Failed to determine websocket URL for target URL '{}'",
                    target.url
                );
                return Ok(Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(Body::empty())
                    .expect("empty body and valid status should not fail"));
            }
        };
        match WebSocketUpgrade::from_request(req, &state).await {
            Ok(ws) => {
                return Ok(ws.on_upgrade(move |client_socket| async move {
                    handle_ws_proxy(state.client, ws_url, headers, client_socket).await;
                }));
            }
            Err(rejection) => {
                eprintln!("WebSocket upgrade extraction failed: {:?}", rejection);
                return Ok(axum::response::IntoResponse::into_response(rejection));
            }
        }
    }

    let url = format!("{}{}", target.url, path_and_query);
    let method = req.method().clone();
    let reqwest_body = reqwest::Body::wrap_stream(req.into_data_stream());

    let mut outbound_req = state.client.request(method, &url).body(reqwest_body);

    if target.pass_headers {
        let mut headers = headers.clone();
        headers.remove("Authorization");
        outbound_req = outbound_req.headers(headers);
    }

    Ok(match outbound_req.send().await {
        Ok(upstream_res) => {
            let mut res_builder = Response::builder().status(upstream_res.status());
            if let Some(h) = res_builder.headers_mut() {
                *h = upstream_res.headers().clone();
            }
            res_builder
                .body(Body::from_stream(upstream_res.bytes_stream()))
                .context("cannot create body from stream")?
        }
        Err(_) => Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .body(Body::empty())
            .expect("empty body and valid status should not fail"),
    })
}

async fn handle_ws_proxy(
    client: Client,
    url: String,
    headers: HeaderMap,
    client_socket: axum::extract::ws::WebSocket,
) {
    let mut upstream_req = client.get(&url);
    for (key, value) in headers.iter() {
        if key != "connection"
            && key != "upgrade"
            && key != "sec-websocket-key"
            && key != "sec-websocket-version"
        {
            upstream_req = upstream_req.header(key, value);
        }
    }
    let upstream_req = upstream_req.upgrade();

    let upstream_res = match upstream_req.send().await {
        Ok(res) => res,
        Err(e) => {
            eprintln!("Failed to connect to upstream WebSocket: {:?}", e);
            return;
        }
    };

    let upstream_socket = match upstream_res.into_websocket().await {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("Failed to upgrade upstream connection: {:?}", e);
            return;
        }
    };

    let (mut client_write, mut client_read) = client_socket.split();
    let (mut upstream_write, mut upstream_read) = upstream_socket.split();

    let client_to_upstream = async {
        while let Some(Ok(msg)) = client_read.next().await {
            let reqwest_msg = match msg {
                axum::extract::ws::Message::Text(t) => {
                    reqwest_websocket::Message::Text(t.to_string())
                }
                axum::extract::ws::Message::Binary(b) => reqwest_websocket::Message::Binary(b),
                axum::extract::ws::Message::Ping(p) => reqwest_websocket::Message::Ping(p),
                axum::extract::ws::Message::Pong(p) => reqwest_websocket::Message::Pong(p),
                axum::extract::ws::Message::Close(c) => {
                    let (code, reason) = c
                        .map(|f| (f.code.into(), f.reason.to_string()))
                        .unwrap_or_default();
                    let _ = upstream_write
                        .send(reqwest_websocket::Message::Close { code, reason })
                        .await;
                    break;
                }
            };
            if upstream_write.send(reqwest_msg).await.is_err() {
                break;
            }
        }
    };

    let upstream_to_client = async {
        while let Some(Ok(msg)) = upstream_read.next().await {
            let axum_msg = match msg {
                reqwest_websocket::Message::Text(t) => axum::extract::ws::Message::Text(t.into()),
                reqwest_websocket::Message::Binary(b) => axum::extract::ws::Message::Binary(b),
                reqwest_websocket::Message::Ping(p) => axum::extract::ws::Message::Ping(p),
                reqwest_websocket::Message::Pong(p) => axum::extract::ws::Message::Pong(p),
                reqwest_websocket::Message::Close { code, reason } => {
                    let _ = client_write
                        .send(axum::extract::ws::Message::Close(Some(CloseFrame {
                            code: code.into(),
                            reason: reason.into(),
                        })))
                        .await;
                    break;
                }
            };
            if client_write.send(axum_msg).await.is_err() {
                break;
            }
        }
    };

    tokio::select! {
        _ = client_to_upstream => {},
        _ = upstream_to_client => {},
    }
}

async fn find_data_asset_target<T: Store>(
    state: &AppStateReverseProxy<T>,
    claims: AuthClaims,
) -> anyhow::Result<Option<RemoteAddress>> {
    let Some(serde_json::Value::String(provider_pid)) = claims.data.get("sub") else {
        anyhow::bail!("invalid auth claims")
    };

    let agreement = match state.store.get_transfer(&provider_pid).await? {
        Some(Transfer::Provider {
            process, agreement, ..
        }) => {
            match &process.state {
                TransferState::Started(s) if !s.is_pending() => {}
                s => anyhow::bail!("invalid transfer state {} (pending={})", s, s.is_pending()),
            }
            agreement
        }
        _ => anyhow::bail!("could not find transfer with PID {}", provider_pid),
    };

    state
        .store
        .get_dataset_target(&agreement.target)
        .await
        .context("failed to load dataset")
}
