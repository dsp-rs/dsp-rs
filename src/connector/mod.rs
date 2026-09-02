use axum::{
    Json, Router,
    extract::FromRef,
    routing::{any, get},
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::{
    auth,
    catalog::{self, sync::catalog_sync},
    connector::{
        app_state::{AppState, AppStateCatalog, AppStateNegotiation, AppStateTransfer},
        validator::{HasSchemaName, SchemaValidator, ValidatedResponseExt},
    },
    model::metadata::{Auth, ProtocolVersion, VersionResponse},
    negotiation::{self, handle_negotiations},
    reverse_proxy,
    store::{Store, file_store::FileStore},
    transfer::{self, handle_transfers},
};

pub(crate) const DSP_API_PATH_2025_1: &str = "/api/2025/1";

pub(crate) mod app_state;
mod internal_api;
pub(crate) mod utils;
pub(crate) mod validator;

pub(crate) trait Pending {
    fn is_pending(&self) -> bool {
        false
    }
}

impl Pending for () {}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct PendingStateData<T> {
    pub(crate) sent: bool,

    #[serde(flatten)]
    pub(crate) payload: T,
}

impl<T> Pending for PendingStateData<T> {
    fn is_pending(&self) -> bool {
        !self.sent
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct TerminatedDataPayload {
    pub(crate) code: Option<String>,
    pub(crate) reason: Option<Vec<String>>,
}

pub(crate) type TerminatedData = PendingStateData<TerminatedDataPayload>;

pub(crate) trait StateData: Serialize + for<'de> Deserialize<'de> {}
impl<T> StateData for T where T: Serialize + for<'de> Deserialize<'de> {}

pub(crate) async fn start(token: CancellationToken, tracker: &TaskTracker) -> anyhow::Result<()> {
    let store = FileStore::new("./data".into());
    let (state, rx_n, rx_t) = AppState::new(store).await?;

    tracker.spawn(catalog_sync(
        token.clone(),
        AppStateCatalog::from_ref(&state),
    ));

    // negotiations
    tracker.spawn(handle_negotiations(
        token.clone(),
        AppStateNegotiation::from_ref(&state),
        rx_n,
    ));
    // transfers
    tracker.spawn(handle_transfers(
        token.clone(),
        AppStateTransfer::from_ref(&state),
        rx_t,
    ));

    // API server
    tracker.spawn(webserver(token.clone(), state));

    // Echo server
    #[cfg(feature = "echo")]
    tracker.spawn(reverse_proxy::echo::webserver(token.clone()));

    Ok(())
}

async fn webserver<T>(token: CancellationToken, state: AppState<T>)
where
    T: Store,
{
    info!("Started API server");

    let dsp_api_routes = Router::new()
        .nest("/catalog", catalog::router())
        .nest("/negotiations", negotiation::router())
        .nest("/transfers", transfer::router());

    #[cfg_attr(not(feature = "tck"), allow(unused_mut))]
    let mut app = Router::new()
        // did stored inside the wallet
        .route("/.well-known/did.json", get(auth::did))
        // metadata
        .route("/.well-known/dspace-version", get(versions))
        // catalog, negotiation and transfer
        .nest(DSP_API_PATH_2025_1, dsp_api_routes)
        .nest("/auth", auth::router())
        // reverse proxy
        .nest(
            "/pull",
            Router::new().route("/{*path}", any(reverse_proxy::handler)),
        )
        // TCK related routes
        .merge({
            let r = Router::new();
            #[cfg(feature = "tck")]
            let r = r.nest(
                "/tck",
                Router::new()
                    .nest("/negotiations", negotiation::tck::tck_router())
                    .nest("/transfers", transfer::tck::router()),
            );
            r
        })
        .nest("/api-internal", internal_api::router())
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    #[cfg(feature = "tck")]
    {
        app = app.layer(axum::middleware::from_fn(utils::tck::log_req_resp));
    }

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            _ = token.cancelled().await;
        })
        .await
        .unwrap();

    info!("Terminated API server");
}

async fn versions() -> Json<VersionResponse> {
    let versions = VersionResponse {
        protocol_versions: vec![ProtocolVersion {
            version: "2025-1".to_owned(),
            path: DSP_API_PATH_2025_1.to_owned(),
            binding: "HTTPS".to_owned(),
            identifier_type: Some("did:web".into()),
            service_id: None,
            auth: Some(Auth {
                protocol: "DSP-RS-AUTH".into(),
                version: "1.0".into(),
                profile: None,
            }),
        }],
    };
    Json(versions)
}

enum RequestPayload<T> {
    Get,
    Post(T),
}

async fn send_request<'a, Req, Resp, Res, F>(
    client: &Client,
    validator: &SchemaValidator,
    url: &str,
    access_token: Option<String>,
    payload: RequestPayload<Req>,
    mapper: F,
) -> anyhow::Result<Res>
where
    Req: Serialize,
    Resp: for<'de> Deserialize<'de> + HasSchemaName,
    F: FnOnce(Option<Req>, Option<Resp>) -> Res,
{
    let mut request_builder = match &payload {
        RequestPayload::Get => client.get(url),
        RequestPayload::Post(body) => client.post(url).json(body),
    };
    if let Some(access_token) = access_token {
        request_builder = request_builder.bearer_auth(access_token);
    }

    let response = match request_builder.send().await {
        Ok(res) => res,
        Err(err) => {
            return Err(anyhow::anyhow!(
                "Failed to send request to URL {url}, error: {err}"
            ));
        }
    };

    let status = response.status();
    if !status.is_success() {
        return Err(anyhow::anyhow!(
            "Unexpected response status for URL {url}: {status}"
        ));
    }

    let body = match payload {
        RequestPayload::Get => None,
        RequestPayload::Post(b) => Some(b),
    };

    if std::mem::size_of::<Resp>() == 0 {
        Ok(mapper(body, None))
    } else {
        match response.validated_json::<Resp>(validator).await {
            Ok(response) => Ok(mapper(body, Some(response))),
            Err(err) => Err(anyhow::anyhow!(
                "Failed to decode response for URL {url}, error: {err}"
            )),
        }
    }
}

pub(crate) async fn post_request<'a, Req, Resp, Res, F>(
    client: &Client,
    validator: &SchemaValidator,
    url: &str,
    access_token: Option<String>,
    body: Req,
    mapper: F,
) -> anyhow::Result<Res>
where
    Req: Serialize,
    Resp: for<'de> Deserialize<'de> + HasSchemaName,
    F: FnOnce(Req, Option<Resp>) -> Res,
{
    send_request(
        client,
        validator,
        url,
        access_token,
        RequestPayload::Post(body),
        |req, resp| mapper(req.unwrap(), resp),
    )
    .await
}

pub(crate) async fn get_request<Resp>(
    client: &Client,
    validator: &SchemaValidator,
    url: &str,
    access_token: Option<String>,
) -> anyhow::Result<Resp>
where
    Resp: for<'de> Deserialize<'de> + HasSchemaName,
{
    send_request(
        client,
        validator,
        url,
        access_token,
        RequestPayload::Get,
        |_: Option<()>, resp| resp.unwrap(),
    )
    .await
}
