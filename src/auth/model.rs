use std::collections::HashMap;

use jsonwebtoken::{DecodingKey, jwk::Jwk};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct VerifyMeRequest {
    pub(crate) did_web: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Session {
    #[serde(rename = "sessionId")]
    pub(crate) session_id: String,

    #[serde(rename = "bootstrapAuthorizationRequestUrl")]
    pub(crate) openid4vp_url: String,
}

#[derive(Debug, Serialize)]
pub(crate) enum VerificationState {
    Pending,
    Successful {
        credentials: HashMap<String, CredentialData>,
        did_web: String,
    },
    Failed,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum Status {
    Pending,
    Success { access_token: String },
    Failed,
}

#[derive(Debug, Serialize)]
pub(crate) struct VerificationResult {
    pub(crate) session_id: String,
    pub(crate) state: VerificationState,
}

pub(crate) struct CredentialType {
    pub(crate) format: String,
    pub(crate) vct: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct CredentialData {
    #[serde(rename = "type")]
    pub(crate) r#type: String,

    pub(crate) format: String,

    #[serde(rename = "credentialData")]
    pub(crate) credential_data: Value,

    pub(crate) issuer: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct Credential {
    pub(crate) id: String,
    pub(crate) format: String,
    pub(crate) meta: CredentialMeta,
}

#[derive(Debug, Serialize)]
pub(crate) struct CredentialMeta {
    pub(crate) vct_values: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct DcqlQuery {
    pub(crate) credentials: Vec<Credential>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CoreFlow {
    pub(crate) dcql_query: DcqlQuery,
    pub(crate) policies: Value,
}

#[derive(Debug, Serialize)]
pub(crate) struct VerificationRequest {
    pub(crate) flow_type: String,
    pub(crate) core_flow: CoreFlow,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StatusResponse {
    pub(crate) status: String,
    pub(crate) presented_credentials: Option<HashMap<String, Vec<CredentialData>>>,
}

#[derive(Debug, Serialize)]
pub(crate) struct PresentationRequest {
    #[serde(rename = "requestUrl")]
    pub(crate) request_url: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Did {
    pub(crate) did: String,
    pub(crate) document: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VerificationMethod {
    id: String,
    controller: String,
    public_key_jwk: Jwk,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DidDocument {
    pub(crate) id: String,
    authentication: Vec<String>,
    verification_method: Vec<VerificationMethod>,
}

impl DidDocument {
    pub(crate) fn decoding_key(&self) -> anyhow::Result<DecodingKey> {
        // NOTE: This is a simplification, ideally we should use the `kid` to find the correct public key
        let Some(method) = self.authentication.iter().find_map(|auth| {
            self.verification_method
                .iter()
                .find(|method| &method.id == auth && method.controller == self.id)
        }) else {
            anyhow::bail!("Could not find verification method for authentication");
        };

        DecodingKey::from_jwk(&method.public_key_jwk)
            .map_err(|err| anyhow::anyhow!("Failed to decode public key, error: {err}"))
    }
}
