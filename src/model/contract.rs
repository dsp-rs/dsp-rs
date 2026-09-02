use std::fmt::Debug;

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    connector::validator::HasSchemaName,
    model::{
        common::{JsonLDContext, JsonLDType, Resource},
        policy::{Agreement, MessageOffer},
    },
    negotiation::{self, NegotiationStateData},
};

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContractRequest {
    #[serde(flatten)]
    context: JsonLDContext,

    #[serde(flatten)]
    r#type: JsonLDType,

    pub(crate) consumer_pid: String,

    pub(crate) offer: ContractRequestOffer,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) provider_pid: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) callback_address: Option<String>,
}

impl From<ContractRequest> for (String, String) {
    fn from(value: ContractRequest) -> Self {
        (
            value.provider_pid.unwrap_or("".to_owned()),
            value.consumer_pid,
        )
    }
}

impl ContractRequest {
    pub(crate) fn make_initial(
        consumer_pid: String,
        offer: ContractRequestOffer,
        callback_address: String,
    ) -> Self {
        Self {
            context: Default::default(),
            r#type: "ContractRequestMessage".into(),
            consumer_pid,
            offer,
            provider_pid: None,
            callback_address: Some(callback_address),
        }
    }

    pub(crate) fn make_secondary(
        consumer_pid: String,
        offer: ContractRequestOffer,
        provider_pid: String,
    ) -> Self {
        Self {
            context: Default::default(),
            r#type: "ContractRequestMessage".into(),
            consumer_pid,
            offer,
            provider_pid: Some(provider_pid),
            callback_address: None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub(crate) enum ContractRequestOffer {
    // NOTE: order is important for deserialization since MessageOffer also
    // contains a `Resource`!
    Concrete(MessageOffer),
    Reference(Resource),
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContractOffer {
    #[serde(flatten)]
    context: JsonLDContext,

    #[serde(flatten)]
    r#type: JsonLDType,

    pub(crate) provider_pid: String,
    pub(crate) offer: MessageOffer,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) callback_address: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) consumer_pid: Option<String>,
}

impl From<ContractOffer> for (String, String) {
    fn from(value: ContractOffer) -> Self {
        (
            value.provider_pid,
            value.consumer_pid.unwrap_or("".to_owned()),
        )
    }
}

impl ContractOffer {
    pub(crate) fn _make_initial(
        provider_pid: String,
        offer: MessageOffer,
        callback_address: String,
    ) -> Self {
        Self {
            context: Default::default(),
            r#type: "ContractOfferMessage".into(),
            provider_pid,
            offer,
            callback_address: Some(callback_address),
            consumer_pid: None,
        }
    }

    pub(crate) fn make_secondary(
        provider_pid: String,
        offer: MessageOffer,
        consumer_pid: String,
    ) -> Self {
        Self {
            context: Default::default(),
            r#type: "ContractOfferMessage".into(),
            provider_pid,
            offer,
            callback_address: None,
            consumer_pid: Some(consumer_pid),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContractNegotiation {
    #[serde(flatten)]
    context: JsonLDContext,

    #[serde(flatten)]
    r#type: JsonLDType,

    pub(crate) provider_pid: String,
    pub(crate) consumer_pid: String,
    pub(crate) state: String,
}

impl HasSchemaName for ContractNegotiation {
    const NAME: &'static str =
        "https://w3id.org/dspace/2025/1/negotiation/contract-negotiation-schema.json";
}

impl<V: NegotiationStateData> From<negotiation::ContractNegotiation<V>> for ContractNegotiation {
    fn from(value: negotiation::ContractNegotiation<V>) -> Self {
        Self {
            context: Default::default(),
            r#type: "ContractNegotiation".into(),
            provider_pid: value.provider_pid.clone(),
            consumer_pid: value.consumer_pid.clone(),
            state: value.state.to_string(),
        }
    }
}

impl ContractNegotiation {
    pub(crate) fn accept(provider_pid: String, consumer_pid: String) -> ContractNegotiationEvent {
        ContractNegotiationEvent {
            context: Default::default(),
            r#type: "ContractNegotiationEventMessage".into(),
            provider_pid,
            consumer_pid,
            event_type: ContractNegotiationEventType::Accepted,
        }
    }

    pub(crate) fn agree(
        provider_pid: String,
        consumer_pid: String,
        agreement: Agreement,
    ) -> ContractAgreement {
        ContractAgreement {
            context: Default::default(),
            r#type: "ContractAgreementMessage".into(),
            provider_pid,
            consumer_pid,
            agreement,
        }
    }

    pub(crate) fn verify(
        provider_pid: String,
        consumer_pid: String,
    ) -> ContractAgreementVerification {
        ContractAgreementVerification {
            context: Default::default(),
            r#type: "ContractAgreementVerificationMessage".into(),
            provider_pid,
            consumer_pid,
        }
    }

    pub(crate) fn finalize(provider_pid: String, consumer_pid: String) -> ContractNegotiationEvent {
        ContractNegotiationEvent {
            context: Default::default(),
            r#type: "ContractNegotiationEventMessage".into(),
            provider_pid,
            consumer_pid,
            event_type: ContractNegotiationEventType::Finalized,
        }
    }

    pub(crate) fn terminate(
        provider_pid: String,
        consumer_pid: String,
        code: Option<String>,
        reason: Option<Vec<String>>,
    ) -> ContractNegotiationTermination {
        ContractNegotiationTermination {
            context: Default::default(),
            r#type: "ContractNegotiationTerminationMessage".into(),
            provider_pid,
            consumer_pid,
            code,
            reason,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub(crate) enum ContractNegotiationEventType {
    Accepted,
    Finalized,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContractNegotiationEvent {
    #[serde(flatten)]
    context: JsonLDContext,

    #[serde(flatten)]
    r#type: JsonLDType,

    pub(crate) provider_pid: String,
    pub(crate) consumer_pid: String,
    pub(crate) event_type: ContractNegotiationEventType,
}

impl From<ContractNegotiationEvent> for (String, String) {
    fn from(value: ContractNegotiationEvent) -> Self {
        (value.provider_pid, value.consumer_pid)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContractNegotiationTermination {
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

impl From<ContractNegotiationTermination> for (String, String) {
    fn from(value: ContractNegotiationTermination) -> Self {
        (value.provider_pid, value.consumer_pid)
    }
}

#[derive(Serialize, Deserialize, Debug, Error)]
#[serde(rename_all = "camelCase")]
#[error("Negotiation error, code: {code:?}, reason: {reason:?}")]
pub(crate) struct ContractNegotiationError {
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

impl ContractNegotiationError {
    pub(crate) fn new(
        provider_pid: String,
        consumer_pid: String,
        code: StatusCode,
        reason: String,
    ) -> Self {
        Self {
            context: Default::default(),
            r#type: "ContractNegotiationError".into(),
            provider_pid,
            consumer_pid,
            code: Some(format!("{code}", code = code.as_u16())),
            reason: Some(vec![reason]),
        }
    }

    pub(crate) fn contract_not_found(provider_pid: String, consumer_pid: String) -> Self {
        Self::new(
            provider_pid,
            consumer_pid,
            StatusCode::NOT_FOUND,
            "Contract not found".to_owned(),
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

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContractAgreement {
    #[serde(flatten)]
    context: JsonLDContext,

    #[serde(flatten)]
    r#type: JsonLDType,

    pub(crate) provider_pid: String,
    pub(crate) consumer_pid: String,
    pub(crate) agreement: Agreement,
}

impl From<ContractAgreement> for (String, String) {
    fn from(value: ContractAgreement) -> Self {
        (value.provider_pid, value.consumer_pid)
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContractAgreementVerification {
    #[serde(flatten)]
    context: JsonLDContext,

    #[serde(flatten)]
    r#type: JsonLDType,

    pub(crate) provider_pid: String,
    pub(crate) consumer_pid: String,
}

impl From<ContractAgreementVerification> for (String, String) {
    fn from(value: ContractAgreementVerification) -> Self {
        (value.provider_pid, value.consumer_pid)
    }
}

#[cfg(test)]
mod tests {
    use crate::assert_json_roundtrip;

    use super::*;

    #[test]
    fn test_contract_request() {
        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/negotiation/example/contract-request-message_initial.json",
            ContractRequest
        );

        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/negotiation/example/contract-request-message.json",
            ContractRequest
        );
    }

    #[test]
    fn test_contract_offer() {
        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/negotiation/example/contract-offer-message_initial.json",
            ContractOffer
        );

        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/negotiation/example/contract-offer-message.json",
            ContractOffer
        );
    }

    #[test]
    fn test_contract_negotiation() {
        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/negotiation/example/contract-negotiation.json",
            ContractNegotiation
        );

        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/negotiation/example/contract-negotiation-event-message.json",
            ContractNegotiationEvent
        );

        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/negotiation/example/contract-negotiation-termination-message.json",
            ContractNegotiationTermination
        );

        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/negotiation/example/contract-negotiation-error.json",
            ContractNegotiationError
        );
    }

    #[test]
    fn test_contract_agreement() {
        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/negotiation/example/contract-agreement-message.json",
            ContractAgreement
        );

        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/negotiation/example/contract-agreement-message-full.json",
            ContractAgreement
        );

        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/negotiation/example/contract-agreement-verification-message.json",
            ContractAgreementVerification
        );
    }
}
