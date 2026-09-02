use anyhow::Context;
use url::Url;

pub(crate) fn derive_did_web(address: &str) -> anyhow::Result<String> {
    let url = Url::parse(address)
        .map_err(|err| anyhow::anyhow!("Failed to parse address, error: {err}"))?;
    let authority = url
        .host_str()
        .map(|h| match url.port() {
            Some(p) => format!("{}%3A{}", h, p),
            None => h.to_string(),
        })
        .ok_or(anyhow::anyhow!(
            "Cannot determine authority from address {}",
            address
        ))?;

    Ok(format!("did:web:{}", authority))
}

pub(crate) fn resolve_did_web(did_web: &str, enforce_https: bool) -> anyhow::Result<Url> {
    let method_specific_id = did_web
        .strip_prefix("did:web:")
        .context("Invalid did:web, missing prefix")?;

    let mut segments = method_specific_id.split(':');

    let domain = segments
        .next()
        .context("Invalid did:web, no domain specified")?
        .replace("%3A", ":")
        .replace("%3a", ":");

    let path = segments.collect::<Vec<_>>().join("/");

    let protocol = if enforce_https { "https" } else { "http" };
    let url = if path.is_empty() {
        format!("{protocol}://{domain}/.well-known/did.json")
    } else {
        format!("{protocol}://{domain}/{path}/did.json")
    };

    Ok(Url::parse(&url)?)
}

#[cfg(feature = "tck")]
pub(super) mod tck {
    use axum::{
        body::Body,
        extract::Request,
        http::{Response, StatusCode},
        middleware::Next,
        response::IntoResponse,
    };
    use futures::StreamExt;
    use http_body_util::BodyExt;
    use tracing::{debug, error};

    pub(crate) async fn log_req_resp(req: Request, next: Next) -> impl IntoResponse {
        let is_websocket = req
            .headers()
            .get("upgrade")
            .map(|v| v.to_str().unwrap_or("") == "websocket")
            .unwrap_or(false);

        if is_websocket {
            return next.run(req).await;
        }

        let (parts, body) = req.into_parts();
        let bytes = match body.collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(e) => {
                error!("Failed to collect request body: {}", e);
                return Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Body::from("could not read request body"))
                    .expect("builder is safe with valid status/body");
            }
        };
        if let Ok(body_str) = std::str::from_utf8(&bytes) {
            debug!("<-- request = {}", body_str);
        } else {
            debug!("<-- request = <non-utf8 {} bytes>", bytes.len());
        }

        let response = next
            .run(Request::from_parts(parts, Body::from(bytes)))
            .await;
        let (parts, body) = response.into_parts();

        let res_stream = body.into_data_stream().map(|chunk| match chunk {
            Ok(bytes) => {
                if let Ok(s) = std::str::from_utf8(&bytes) {
                    debug!("--> response chunk = {}", s);
                } else {
                    debug!("--> response chunk = <non-utf8 {} bytes>", bytes.len());
                }
                Ok(bytes)
            }
            Err(e) => Err(e),
        });

        Response::from_parts(parts, Body::from_stream(res_stream))
    }
}
