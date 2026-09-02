use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    connector::validator::HasSchemaName,
    model::common::{JsonLDContext, JsonLDType},
    transfer::{self, TransferStateData},
};

use axum::http::StatusCode;

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TransferProcess {
    #[serde(flatten)]
    context: JsonLDContext,

    #[serde(flatten)]
    r#type: JsonLDType,

    pub(crate) provider_pid: String,
    pub(crate) consumer_pid: String,

    pub(crate) state: String,
}

impl HasSchemaName for TransferProcess {
    const NAME: &'static str =
        "https://w3id.org/dspace/2025/1/transfer/transfer-process-schema.json";
}

impl TransferProcess {
    pub(crate) fn start(
        provider_pid: String,
        consumer_pid: String,
        data_address: Option<DataAddress>,
    ) -> TransferStart {
        TransferStart {
            context: Default::default(),
            r#type: "TransferStartMessage".into(),
            provider_pid,
            consumer_pid,
            data_address,
        }
    }

    pub(crate) fn terminate(
        provider_pid: String,
        consumer_pid: String,
        code: Option<String>,
        reason: Option<Vec<String>>,
    ) -> AbstractTransferCode {
        AbstractTransferCode::new(
            "TransferTerminationMessage",
            provider_pid,
            consumer_pid,
            code,
            reason,
        )
    }

    pub(crate) fn suspend(
        provider_pid: String,
        consumer_pid: String,
        code: Option<String>,
        reason: Option<Vec<String>>,
    ) -> AbstractTransferCode {
        AbstractTransferCode::new(
            "TransferSuspensionMessage",
            provider_pid,
            consumer_pid,
            code,
            reason,
        )
    }

    pub(crate) fn complete(provider_pid: String, consumer_pid: String) -> TransferCompletion {
        TransferCompletion::new(provider_pid, consumer_pid)
    }
}

impl<V: TransferStateData> From<transfer::TransferProcess<V>> for TransferProcess {
    fn from(value: transfer::TransferProcess<V>) -> Self {
        Self {
            context: Default::default(),
            r#type: "TransferProcess".into(),
            provider_pid: value.provider_pid.clone(),
            consumer_pid: value.consumer_pid.clone(),
            state: value.state.to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DataAddress {
    #[serde(flatten)]
    r#type: JsonLDType,

    pub(crate) endpoint_type: String,
    pub(crate) endpoint: Option<String>,
    pub(crate) endpoint_properties: Option<Vec<EndpointProperty>>,
}

impl DataAddress {
    pub(crate) fn new_http_with_token(endpoint: String, token: String) -> Self {
        Self {
            r#type: "DataAddress".into(),
            endpoint_type: "https://w3id.org/idsa/v4.1/HTTP".into(),
            endpoint: Some(endpoint),
            endpoint_properties: Some(vec![
                EndpointProperty::new("authorization".into(), token),
                EndpointProperty::new("authType".into(), "bearer".into()),
            ]),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EndpointProperty {
    #[serde(flatten)]
    r#type: JsonLDType,

    pub(crate) name: String,
    pub(crate) value: String,
}

impl EndpointProperty {
    fn new(name: String, value: String) -> Self {
        Self {
            r#type: "EndpointProperty".into(),
            name,
            value,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TransferRequest {
    #[serde(flatten)]
    context: JsonLDContext,

    #[serde(flatten)]
    r#type: JsonLDType,

    pub(crate) agreement_id: String,
    pub(crate) format: String,
    pub(crate) callback_address: String,
    pub(crate) consumer_pid: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) data_address: Option<DataAddress>,
}

impl HasSchemaName for TransferRequest {
    const NAME: &'static str =
        "https://w3id.org/dspace/2025/1/transfer/transfer-request-message-schema.json";
}

impl TransferRequest {
    pub(crate) fn new(
        agreement_id: String,
        format: String,
        callback_address: String,
        consumer_pid: String,
        data_address: Option<DataAddress>,
    ) -> Self {
        Self {
            context: Default::default(),
            r#type: "TransferRequestMessage".into(),
            agreement_id,
            format,
            callback_address,
            consumer_pid,
            data_address,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TransferStart {
    #[serde(flatten)]
    context: JsonLDContext,

    #[serde(flatten)]
    r#type: JsonLDType,

    pub(crate) provider_pid: String,
    pub(crate) consumer_pid: String,

    pub(crate) data_address: Option<DataAddress>,
}

impl HasSchemaName for TransferStart {
    const NAME: &'static str =
        "https://w3id.org/dspace/2025/1/transfer/transfer-start-message-schema.json";
}

impl From<TransferStart> for (String, String) {
    fn from(value: TransferStart) -> Self {
        (value.provider_pid, value.consumer_pid)
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AbstractTransferCode {
    #[serde(flatten)]
    context: JsonLDContext,

    #[serde(flatten)]
    r#type: JsonLDType,

    pub(crate) provider_pid: String,
    pub(crate) consumer_pid: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<Vec<String>>,
}

impl HasSchemaName for AbstractTransferCode {
    const NAME: &'static str = "https://w3id.org/dspace/2025/1/transfer/transfer-schema.json";
}

impl From<AbstractTransferCode> for (String, String) {
    fn from(value: AbstractTransferCode) -> Self {
        (value.provider_pid, value.consumer_pid)
    }
}

#[derive(Serialize, Deserialize, Debug, Error)]
#[serde(rename_all = "camelCase")]
#[error("Transfer error, code: {code:?}, reason: {reason:?}", code = inner.code, reason = inner.reason)]
pub(crate) struct TransferError {
    #[serde(flatten)]
    pub(crate) inner: AbstractTransferCode,
}

impl TransferError {
    fn new(provider_pid: String, consumer_pid: String, code: StatusCode, reason: String) -> Self {
        Self {
            inner: AbstractTransferCode::new(
                "TransferError",
                provider_pid,
                consumer_pid,
                Some(format!("{code}", code = code.as_u16())),
                Some(vec![reason]),
            ),
        }
    }

    pub(crate) fn agreement_not_found(provider_pid: String, consumer_pid: String) -> TransferError {
        Self::new(
            provider_pid,
            consumer_pid,
            StatusCode::BAD_REQUEST,
            "No matching agreement".to_owned(),
        )
    }

    pub(crate) fn forbidden(provider_pid: String, consumer_pid: String) -> TransferError {
        Self::new(
            provider_pid,
            consumer_pid,
            StatusCode::FORBIDDEN,
            "Access denied".to_owned(),
        )
    }

    pub(crate) fn transfer_not_found(provider_pid: String, consumer_pid: String) -> TransferError {
        Self::new(
            provider_pid,
            consumer_pid,
            StatusCode::NOT_FOUND,
            "Transfer not found".to_owned(),
        )
    }

    pub(crate) fn invalid_state(provider_pid: String, consumer_pid: String) -> Self {
        Self::new(
            provider_pid,
            consumer_pid,
            StatusCode::BAD_REQUEST,
            "Invalid state".to_owned(),
        )
    }
}

impl AbstractTransferCode {
    fn new(
        r#type: &str,
        provider_pid: String,
        consumer_pid: String,
        code: Option<String>,
        reason: Option<Vec<String>>,
    ) -> Self {
        Self {
            context: Default::default(),
            r#type: r#type.into(),
            provider_pid,
            consumer_pid,
            code,
            reason,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TransferCompletion {
    #[serde(flatten)]
    context: JsonLDContext,

    #[serde(flatten)]
    r#type: JsonLDType,

    pub(crate) provider_pid: String,
    pub(crate) consumer_pid: String,
}

impl HasSchemaName for TransferCompletion {
    const NAME: &'static str =
        "https://w3id.org/dspace/2025/1/transfer/transfer-completion-message-schema.json";
}

impl From<TransferCompletion> for (String, String) {
    fn from(value: TransferCompletion) -> Self {
        (value.provider_pid, value.consumer_pid)
    }
}

impl TransferCompletion {
    fn new(provider_pid: String, consumer_pid: String) -> Self {
        Self {
            context: Default::default(),
            r#type: "TransferCompletionMessage".into(),
            provider_pid,
            consumer_pid,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::assert_json_roundtrip;

    use super::*;

    #[test]
    fn test_transfer_process() {
        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/transfer/example/transfer-process.json",
            TransferProcess
        );

        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/transfer/example/transfer-request-message.json",
            TransferRequest
        );

        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/transfer/example/transfer-start-message.json",
            TransferStart
        );

        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/transfer/example/transfer-suspension-message.json",
            AbstractTransferCode
        );

        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/transfer/example/transfer-termination-message.json",
            AbstractTransferCode
        );

        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/transfer/example/transfer-completion-message.json",
            TransferCompletion
        );

        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/transfer/example/transfer-error.json",
            AbstractTransferCode
        );
    }
}
