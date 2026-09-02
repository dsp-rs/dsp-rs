use std::collections::HashMap;
use std::time::Duration;

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::connector::app_state::AppState;
use crate::store::Store;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct AuthClaims {
    #[serde(flatten)]
    pub(crate) data: HashMap<String, Value>,
}

impl Default for AuthClaims {
    fn default() -> Self {
        let mut data = HashMap::default();
        let iat = Utc::now();
        let exp = iat + Duration::from_hours(8);

        data.insert("jti".into(), uuid::Uuid::new_v4().to_string().into());
        data.insert("iat".into(), iat.timestamp().into());
        data.insert("exp".into(), exp.timestamp().into());

        Self { data }
    }
}

impl AuthClaims {
    pub(crate) fn is_expired(&self) -> bool {
        let threshold = Utc::now().timestamp() + 60;

        self.data
            .get("exp")
            .and_then(|v| v.as_i64())
            .map_or(false, |exp| exp < threshold)
    }

    pub(crate) fn subject(&self) -> anyhow::Result<&str> {
        #[cfg(feature = "tck")]
        return Ok("");

        #[cfg(not(feature = "tck"))]
        Ok(self
            .data
            .get("sub")
            .and_then(|v| v.as_str())
            .ok_or(anyhow::anyhow!("No subject in claims"))?)
    }

    pub(crate) fn issuer(&self) -> anyhow::Result<&str> {
        #[cfg(feature = "tck")]
        return Ok("");

        #[cfg(not(feature = "tck"))]
        Ok(self
            .data
            .get("iss")
            .and_then(|v| v.as_str())
            .ok_or(anyhow::anyhow!("No issuer in claims"))?)
    }
}

impl<T> FromRequestParts<AppState<T>> for AuthClaims
where
    T: Store,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(
        #[cfg_attr(feature = "tck", allow(unused))] parts: &mut Parts,
        #[cfg_attr(feature = "tck", allow(unused))] state: &AppState<T>,
    ) -> Result<Self, Self::Rejection> {
        #[cfg(feature = "tck")]
        {
            let mut claims = AuthClaims::default();

            // NOTE: when running in TCK mode, all calls come from the TCK harness
            claims
                .data
                .insert("sub".into(), Value::String("tck".into()));
            return Ok(claims);
        }

        #[cfg_attr(feature = "tck", allow(unreachable_code))]
        let auth_header = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or((StatusCode::UNAUTHORIZED, "Missing header".into()))?;

        Ok(state
            .authenticator
            .decode(
                auth_header
                    .strip_prefix("Bearer ")
                    .or_else(|| auth_header.strip_prefix("bearer "))
                    .unwrap_or(auth_header),
            )
            .map_err(|err| {
                (
                    StatusCode::UNAUTHORIZED,
                    format!("Failed to decode access token, error: {err}"),
                )
            })?)
    }
}
