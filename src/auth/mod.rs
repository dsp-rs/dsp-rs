use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};

use jsonwebtoken::{Algorithm, Validation, dangerous::insecure_decode, decode};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, error, warn};

use crate::{
    AppError,
    auth::{
        extractor::AuthClaims,
        model::{
            CoreFlow, Credential, CredentialData, CredentialMeta, CredentialType, DcqlQuery, Did,
            DidDocument, PresentationRequest, Session, Status, StatusResponse, VerificationRequest,
            VerificationResult, VerificationState, VerifyMeRequest,
        },
    },
    connector::{
        app_state::{AppState, AppStateAuthentication, KeyPair},
        utils::resolve_did_web,
    },
    store::Store,
};

pub(crate) mod extractor;
mod model;

type TokenCache = RwLock<HashMap<String, Arc<Mutex<TokenState>>>>;
type Sessions = Mutex<HashMap<String, String>>;

pub(crate) struct Authenticator {
    issuer_url: String,
    verifier_url: String,
    wallet_url: String,
    wallet_id: String,
    allowed_issuers: Vec<String>,

    key_pair: KeyPair,

    cache: TokenCache,
    sessions: Sessions,
}

impl Authenticator {
    pub(crate) fn new(
        issuer_url: String,
        verifier_url: String,
        wallet_url: String,
        wallet_id: String,
        allowed_issuers: Vec<String>,
        key_pair: KeyPair,
    ) -> Self {
        Self {
            issuer_url,
            verifier_url,
            wallet_url,
            wallet_id,
            allowed_issuers,
            key_pair,
            cache: Default::default(),
            sessions: Default::default(),
        }
    }

    async fn did(&self, client: &Client, did: &str) -> anyhow::Result<Value> {
        let result: Did = client
            .get(format!(
                "{}/wallet/{}/dids/{}",
                self.wallet_url,
                self.wallet_id,
                utf8_percent_encode(did, NON_ALPHANUMERIC),
            ))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        anyhow::ensure!(
            result.did == did,
            "DID mismatch: expected {}, got {}",
            did,
            result.did
        );

        Ok(result.document)
    }

    // TODO: provide offer ID in order to match correct credentials on the provider side
    pub(crate) async fn get_token(
        &self,
        #[cfg_attr(feature = "tck", allow(unused))] client: &Client,
        #[cfg_attr(feature = "tck", allow(unused))] remote_address: &str,
        #[cfg_attr(feature = "tck", allow(unused))] did_web: String,
    ) -> anyhow::Result<String> {
        #[cfg(feature = "tck")]
        return Ok("".to_string());

        #[cfg_attr(feature = "tck", allow(unreachable_code))]
        let token_mutex = {
            let mut cache_guard = self.cache.write().await;
            cache_guard
                .entry(remote_address.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(TokenState::default())))
                .clone()
        };

        let mut token_guard = token_mutex.lock().await;
        match &mut *token_guard {
            TokenState::Fetched((token, claims)) => {
                if !claims.is_expired() {
                    return Ok(token.clone());
                }

                // reset state
                *token_guard = TokenState::default();
            }
            TokenState::Fetching(backoff) => {
                if !backoff.should_retry() {
                    anyhow::bail!("Too many requests");
                }
            }
        }

        debug!("Trying to get token for remote address '{remote_address}' ...");
        let new_token = self.do_get_token(client, remote_address, did_web).await?;
        let claims = insecure_decode::<AuthClaims>(&new_token)?.claims;
        let iss_did_web = claims
            .issuer()
            .context("Token does not contain an issuer claim")?;

        // TODO: determine `kid` from header and pass it to `get_public_key_from_did`
        let decoding_key = self
            .load_verify_did(client, iss_did_web)
            .await
            .context(format!("Failed to load DID for {iss_did_web}"))?
            .decoding_key()
            .context(format!(
                "Failed to determine decoding key for {iss_did_web}"
            ))?;

        let validation = Validation::new(Algorithm::ES256);
        let claims = decode::<AuthClaims>(&new_token, &decoding_key, &validation)?.claims;

        *token_guard = TokenState::Fetched((new_token.clone(), claims));

        Ok(new_token)
    }

    async fn load_verify_did(&self, client: &Client, did_web: &str) -> anyhow::Result<DidDocument> {
        let url = resolve_did_web(did_web, false)?; // TODO: enforce should be configurable
        let did = client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json::<DidDocument>()
            .await?;

        if did.id != did_web {
            anyhow::bail!("Retrieved DID document does not match provided did:web");
        }

        Ok(did)
    }

    async fn do_get_token(
        &self,
        client: &Client,
        remote_address: &str,
        did_web: String,
    ) -> anyhow::Result<String> {
        let session = client
            .post(format!("{}/auth/verify_me", remote_address))
            .json(&VerifyMeRequest { did_web })
            .send()
            .await?
            .error_for_status()?
            .json::<Session>()
            .await?;

        let request = PresentationRequest {
            request_url: session.openid4vp_url,
        };

        // TODO: do we really need to parse the response?
        client
            .post(format!(
                "{}/wallet/{}/credentials/present",
                self.wallet_url, self.wallet_id
            ))
            .json(&request)
            .send()
            .await?
            .error_for_status()?;

        let start = Instant::now();

        const POLL_INTERVAL: Duration = Duration::from_secs(3);
        const MAX_DURATION: Duration = Duration::from_secs(10);

        let check_status = async || -> anyhow::Result<Status> {
            Ok(client
                .get(format!(
                    "{}/auth/status/{}",
                    remote_address, session.session_id
                ))
                .send()
                .await?
                .error_for_status()?
                .json::<Status>()
                .await?)
        };

        loop {
            if start.elapsed() >= MAX_DURATION {
                anyhow::bail!("did not receive a valid status within time limit");
            }

            match check_status().await {
                Ok(Status::Success { access_token }) => return Ok(access_token),
                Ok(Status::Failed) => anyhow::bail!("authentication failed"),
                Ok(Status::Pending) => {}
                Err(err) => warn!("failed to retrieve status, will retry, error: {err}"),
            }

            let elapsed = start.elapsed();
            let remaining = MAX_DURATION.saturating_sub(elapsed);
            tokio::time::sleep(POLL_INTERVAL.min(remaining)).await;
        }
    }

    async fn verify_me(
        &self,
        client: Client,
        credential_types: HashMap<String, CredentialType>,
        did_web: String,
    ) -> anyhow::Result<Session> {
        if credential_types.is_empty() {
            anyhow::bail!("no credential types specified");
        }

        // s. https://docs.walt.id/community-stack/verifier2/policies/configuration
        let request = VerificationRequest {
            flow_type: "cross_device".to_string(),
            core_flow: CoreFlow {
                dcql_query: DcqlQuery {
                    credentials: credential_types
                        .into_iter()
                        .map(|(id, ct)| Credential {
                            id: id,
                            format: ct.format,
                            meta: CredentialMeta {
                                vct_values: vec![ct.vct],
                            },
                        })
                        .collect(),
                },
                policies: json!({
                    "vc_policies": [
                      "signature",
                      "expiration",
                      "not-before",
                      {
                        "policy": "allowed-issuer",
                        "allowed_issuer": &self.allowed_issuers,
                      }
                    ]
                }),
            },
        };

        let session = client
            .post(format!("{}/verification-session/create", self.verifier_url))
            .json(&request)
            .send()
            .await?
            .error_for_status()?
            .json::<Session>()
            .await?;

        {
            let mut guard = self.sessions.lock().await;
            guard.insert(session.session_id.clone(), did_web);
        }

        Ok(session)
    }

    async fn status(
        &self,
        client: Client,
        session_id: String,
    ) -> anyhow::Result<VerificationResult> {
        let did_web = self
            .sessions
            .lock()
            .await
            .get(&session_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

        let response = client
            .get(format!(
                "{}/verification-session/{}/info",
                self.verifier_url, session_id
            ))
            .send()
            .await?
            .error_for_status()?
            .json::<StatusResponse>()
            .await?;

        let state = match response.status.as_str() {
            "SUCCESSFUL" if response.presented_credentials.is_some() => {
                self.sessions.lock().await.remove(&session_id);

                VerificationState::Successful {
                    credentials: response
                        .presented_credentials
                        .unwrap()
                        .into_iter()
                        .filter_map(|(id, cd)| {
                            let Some(cd) = cd.into_iter().next() else {
                                return None;
                            };
                            Some((id, cd))
                        })
                        .collect(),
                    did_web,
                }
            }
            "ACTIVE" | "UNUSED" | "IN_USE" | "VALIDATING_RECEIVED_REQUEST" | "PROCESSING_FLOW" => {
                VerificationState::Pending
            }
            _ => VerificationState::Failed,
        };

        Ok(VerificationResult { session_id, state })
    }

    fn derive_access_token(
        &self,
        credentials: HashMap<String, CredentialData>,
        iss_did_web: String,
        sub_did_web: String,
    ) -> anyhow::Result<Option<String>> {
        let mut claims = AuthClaims::default();

        debug!("Presented credentials:\n{:?}", &credentials);
        let mut found = false;
        for (id, cd) in credentials {
            match id.as_str() {
                "identity" => {
                    if let Ok(data) =
                        serde_json::from_value::<IdentityCredentialData>(cd.credential_data)
                    {
                        claims.data.insert("email".into(), data.email);
                        claims.data.insert("country".into(), data.address.country);
                        found = true;
                    }
                }
                _ => {}
            }
        }
        if !found {
            return Ok(None);
        }

        claims.data.insert("iss".into(), Value::String(iss_did_web));
        claims.data.insert("sub".into(), Value::String(sub_did_web));

        self.key_pair.encode(&claims).map(Some)
    }

    pub(crate) fn new_transfer_token(
        &self,
        pid: String,
        iss_did_web: String,
    ) -> anyhow::Result<String> {
        let mut claims = AuthClaims::default();
        claims.data.insert("sub".to_string(), Value::String(pid));
        claims
            .data
            .insert("iss".to_string(), Value::String(iss_did_web));

        self.key_pair.encode(&claims)
    }

    pub(crate) fn decode(&self, token: &str) -> anyhow::Result<AuthClaims> {
        self.key_pair.decode(token)
    }
}

struct ExpBackoff {
    current_attempt: u32,
    initial_delay: Duration,
    max_delay: Duration,
    next_allowed_attempt_at: Instant,
}

impl ExpBackoff {
    pub fn new(initial_delay: Duration, max_delay: Duration) -> Self {
        Self {
            current_attempt: 0,
            initial_delay,
            max_delay,
            next_allowed_attempt_at: Instant::now(),
        }
    }

    pub fn should_retry(&mut self) -> bool {
        let now = Instant::now();

        if now < self.next_allowed_attempt_at {
            return false;
        }

        self.current_attempt += 1;
        self.calculate_next_window(now);

        true
    }

    fn calculate_next_window(&mut self, last_attempt_time: Instant) {
        let exponent = self.current_attempt.saturating_sub(1);
        let multiplier = 2u32.pow(exponent);
        let delay = (self.initial_delay * multiplier).min(self.max_delay);

        self.next_allowed_attempt_at = last_attempt_time + delay;
    }
}

enum TokenState {
    Fetched((String, AuthClaims)),
    Fetching(ExpBackoff),
}

impl Default for TokenState {
    fn default() -> Self {
        TokenState::Fetching(ExpBackoff::new(
            Duration::from_secs(1),
            Duration::from_secs(60),
        ))
    }
}

#[derive(Debug, Deserialize)]
struct Address {
    country: Value,
}

#[derive(Debug, Deserialize)]
struct IdentityCredentialData {
    email: Value,
    address: Address,
}

pub(crate) fn router<T: Store>() -> Router<AppState<T>> {
    Router::new()
        .route("/verify_me", post(verify_me))
        .route("/status/{session_id}", get(status))
        .merge({
            let r = Router::new();
            #[cfg(feature = "tck")]
            let r = r.route("/tck/get_token", post(tck::get_token));
            r
        })
}

pub(crate) async fn did(
    State(state): State<AppStateAuthentication>,
) -> Result<impl IntoResponse, AppError> {
    let did = state.participant_info.did_web()?;
    let did_document = state.authenticator.did(&state.client, &did).await?;
    Ok(Json(did_document))
}

// TODO: maybe provide as payload the offer ID such that the necessary credentials can be
// determined and requested.
async fn verify_me(
    State(state): State<AppStateAuthentication>,
    Json(request): Json<VerifyMeRequest>,
) -> Result<impl IntoResponse, AppError> {
    let did_web = request.did_web;
    _ = state
        .authenticator
        .load_verify_did(&state.client, &did_web)
        .await
        .context(format!("Failed to load DID for {did_web}"))?;

    // TODO: the base (not offer specific) credential types should be configurable
    const IDENTITY_VCT: &str = "identity_credential";
    const IDENTITY_FORMAT: &str = "dc+sd-jwt";

    let credential_types = HashMap::from([(
        "identity".to_string(),
        CredentialType {
            format: IDENTITY_FORMAT.to_string(),
            vct: format!(
                "{}/openid4vci/{}",
                state.authenticator.issuer_url, IDENTITY_VCT
            ),
        },
    )]);
    let session = state
        .authenticator
        .verify_me(state.client, credential_types, did_web)
        .await?;

    Ok((StatusCode::CREATED, Json(session)))
}

async fn status(
    State(state): State<AppStateAuthentication>,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let status = match state
        .authenticator
        .status(state.client, session_id)
        .await?
        .state
    {
        VerificationState::Pending => Status::Pending,
        VerificationState::Successful {
            credentials,
            did_web,
        } => {
            match state.authenticator.derive_access_token(
                credentials,
                state.participant_info.did_web()?, // issuer
                did_web,                           // subject
            ) {
                Ok(Some(access_token)) => Status::Success { access_token },
                Ok(None) => {
                    error!("Credentials to claims mapping failed");
                    Status::Failed
                }
                Err(err) => {
                    error!("Failed to derive claims from credentials, error: {err}");
                    Status::Failed
                }
            }
        }
        VerificationState::Failed => Status::Failed,
    };

    Ok((StatusCode::OK, Json(status)))
}

#[cfg(feature = "tck")]
mod tck {
    use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
    use serde::Deserialize;

    use crate::{AppError, connector::app_state::AppStateAuthentication};

    #[derive(Debug, Deserialize)]
    pub(super) struct GetTokenRequest {
        remote_address: String,
    }

    pub(super) async fn get_token(
        State(state): State<AppStateAuthentication>,
        Json(request): Json<GetTokenRequest>,
    ) -> Result<impl IntoResponse, AppError> {
        let access_token = state
            .authenticator
            .get_token(
                &state.client,
                &request.remote_address,
                state.participant_info.did_web()?,
            )
            .await?;
        Ok((StatusCode::OK, access_token))
    }
}
