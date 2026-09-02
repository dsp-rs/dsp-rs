use std::{collections::HashMap, env, sync::Arc};

use anyhow::Context;
use axum::extract::FromRef;
use jsonwebtoken::{
    Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode, jwk::Jwk,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::{
    fs,
    sync::mpsc::{Receiver, Sender, channel},
};

use crate::{
    auth::Authenticator,
    connector::{utils::derive_did_web, validator::SchemaValidator},
    negotiation::NegotiationEvent,
    store::Store,
    transfer::TransferEvent,
};

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct RemoteConnector {
    pub(crate) remote_address: String,

    #[serde(default = "default_sync_interval")]
    pub(crate) sync_interval_secs: u64,
}

fn default_sync_interval() -> u64 {
    600
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct ParticipantInfo {
    pub(crate) id: String,
    pub(crate) external_address: String,
}

impl ParticipantInfo {
    pub(crate) fn did_web(&self) -> anyhow::Result<String> {
        derive_did_web(&self.external_address)
    }

    pub(crate) fn callback_address(&self) -> String {
        #[cfg(feature = "tck")]
        return format!("{}{}", self.external_address, super::DSP_API_PATH_2025_1);

        // NOTE: we don't include the path for 2025-1
        #[cfg(not(feature = "tck"))]
        format!("{}", self.external_address)
    }
}

#[derive(Debug, Deserialize)]
struct Configuration {
    participant_info: ParticipantInfo,

    // Used for signing access tokens. This is the same private key as used inside the wallet. The
    // corresponding public key is made available as DID.
    private_key_pem: String,

    issuer_url: String,
    verifier_url: String,
    wallet_url: String,
    wallet_id: String,
    allowed_issuers: Vec<String>,

    federation: HashMap<String, RemoteConnector>,
}

pub(crate) struct AppState<T: Store> {
    participant_info: ParticipantInfo,
    federation: Arc<HashMap<String, RemoteConnector>>,
    pub(crate) validator: Arc<SchemaValidator>,
    pub(crate) authenticator: Arc<Authenticator>,
    client: Client,
    store: Arc<T>,
    negotiation_events: Sender<NegotiationEvent>,
    transfer_events: Sender<TransferEvent>,
}

impl<T: Store> Clone for AppState<T> {
    fn clone(&self) -> Self {
        Self {
            participant_info: self.participant_info.clone(),
            federation: self.federation.clone(),
            validator: self.validator.clone(),
            authenticator: self.authenticator.clone(),
            store: self.store.clone(),
            client: self.client.clone(),
            negotiation_events: self.negotiation_events.clone(),
            transfer_events: self.transfer_events.clone(),
        }
    }
}

impl<T: Store> AppState<T> {
    pub(crate) async fn new(
        store: T,
    ) -> anyhow::Result<(Self, Receiver<NegotiationEvent>, Receiver<TransferEvent>)> {
        let config_path = env::var("CONFIG_PATH").unwrap_or("config.json".into());

        let data = fs::read(config_path)
            .await
            .context("Failed to load configuration file")?;
        let config: Configuration =
            serde_json::from_slice(&data).context("Failed to deserialize config")?;

        let encoding_key = EncodingKey::from_ec_pem(&config.private_key_pem.as_bytes())
            .context("Failed to decode private key format")?;
        let jwk = Jwk::from_encoding_key(&encoding_key, Algorithm::ES256)
            .context("Failed to convert encoding key to jwk")?;
        let decoding_key = DecodingKey::from_jwk(&jwk).context("Failed to create decoding key")?;

        let validator = SchemaValidator::new().await?;

        let (tx_n, rx_n) = channel::<NegotiationEvent>(10);
        let (tx_t, rx_t) = channel::<TransferEvent>(10);

        Ok((
            Self {
                participant_info: config.participant_info,
                federation: Arc::new(config.federation),
                validator: Arc::new(validator),
                authenticator: Arc::new(Authenticator::new(
                    config.issuer_url,
                    config.verifier_url,
                    config.wallet_url,
                    config.wallet_id,
                    config.allowed_issuers,
                    KeyPair {
                        encoding_key,
                        decoding_key,
                    },
                )),
                store: Arc::new(store),
                client: Client::new(),
                negotiation_events: tx_n,
                transfer_events: tx_t,
            },
            rx_n,
            rx_t,
        ))
    }
}

pub(crate) struct AppStateAPI<T: Store> {
    pub(crate) store: Arc<T>,
}

impl<T: Store> FromRef<AppState<T>> for AppStateAPI<T> {
    fn from_ref(app_state: &AppState<T>) -> Self {
        Self {
            store: app_state.store.clone(),
        }
    }
}

pub(crate) struct AppStateAuthentication {
    pub(crate) client: Client,
    pub(crate) participant_info: ParticipantInfo,
    pub(crate) authenticator: Arc<Authenticator>,
}

impl<T: Store> FromRef<AppState<T>> for AppStateAuthentication {
    fn from_ref(app_state: &AppState<T>) -> Self {
        Self {
            client: app_state.client.clone(),
            participant_info: app_state.participant_info.clone(),
            authenticator: app_state.authenticator.clone(),
        }
    }
}

pub(crate) struct AppStateNegotiation<T: Store> {
    pub(crate) participant_info: ParticipantInfo,
    pub(crate) store: Arc<T>,
    pub(crate) authenticator: Arc<Authenticator>,
    pub(crate) client: Client,
    pub(crate) tx: Sender<NegotiationEvent>,
    pub(crate) validator: Arc<SchemaValidator>,
}

impl<T: Store> FromRef<AppState<T>> for AppStateNegotiation<T> {
    fn from_ref(app_state: &AppState<T>) -> Self {
        Self {
            participant_info: app_state.participant_info.clone(),
            store: app_state.store.clone(),
            authenticator: app_state.authenticator.clone(),
            client: app_state.client.clone(),
            tx: app_state.negotiation_events.clone(),
            validator: app_state.validator.clone(),
        }
    }
}

pub(crate) struct AppStateTransfer<T: Store> {
    pub(crate) participant_info: ParticipantInfo,
    pub(crate) store: Arc<T>,
    pub(crate) authenticator: Arc<Authenticator>,
    pub(crate) client: Client,
    pub(crate) tx: Sender<TransferEvent>,
    pub(crate) validator: Arc<SchemaValidator>,
}

impl<T: Store> FromRef<AppState<T>> for AppStateTransfer<T> {
    fn from_ref(app_state: &AppState<T>) -> Self {
        Self {
            participant_info: app_state.participant_info.clone(),
            store: app_state.store.clone(),
            authenticator: app_state.authenticator.clone(),
            client: app_state.client.clone(),
            tx: app_state.transfer_events.clone(),
            validator: app_state.validator.clone(),
        }
    }
}

pub(crate) struct AppStateCatalog<T: Store> {
    pub(crate) store: Arc<T>,
    pub(crate) authenticator: Arc<Authenticator>,
    pub(crate) client: Client,
    pub(crate) participant_info: ParticipantInfo,
    pub(crate) federation: Arc<HashMap<String, RemoteConnector>>,
    pub(crate) validator: Arc<SchemaValidator>,
}

impl<T: Store> Clone for AppStateCatalog<T> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            authenticator: self.authenticator.clone(),
            client: self.client.clone(),
            participant_info: self.participant_info.clone(),
            federation: self.federation.clone(),
            validator: self.validator.clone(),
        }
    }
}

impl<T: Store> FromRef<AppState<T>> for AppStateCatalog<T> {
    fn from_ref(app_state: &AppState<T>) -> Self {
        Self {
            store: app_state.store.clone(),
            authenticator: app_state.authenticator.clone(),
            client: app_state.client.clone(),
            participant_info: app_state.participant_info.clone(),
            federation: app_state.federation.clone(),
            validator: app_state.validator.clone(),
        }
    }
}

pub(crate) struct AppStateReverseProxy<T: Store> {
    pub(crate) store: Arc<T>,
    pub(crate) client: Client,
}

impl<T: Store> FromRef<AppState<T>> for AppStateReverseProxy<T> {
    fn from_ref(app_state: &AppState<T>) -> Self {
        Self {
            store: app_state.store.clone(),
            client: app_state.client.clone(),
        }
    }
}

pub(crate) struct KeyPair {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
}

impl KeyPair {
    pub(crate) fn encode<T: Serialize>(&self, claims: T) -> anyhow::Result<String> {
        let header = Header::new(Algorithm::ES256);
        encode(&header, &claims, &self.encoding_key).map_err(anyhow::Error::msg)
    }

    pub(crate) fn decode<T>(&self, token: &str) -> anyhow::Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let token_data = {
            let validation = Validation::new(Algorithm::ES256);
            decode::<T>(&token, &self.decoding_key, &validation)?
        };
        Ok(token_data.claims)
    }
}
