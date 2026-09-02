use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tracing::debug;

use crate::{
    AppError,
    auth::extractor::AuthClaims,
    connector::{
        Pending, PendingStateData, TerminatedDataPayload, app_state::AppStateNegotiation,
        post_request,
    },
    model::{
        self,
        contract::{
            ContractAgreement, ContractNegotiationEventType, ContractOffer, ContractRequest,
            ContractRequestOffer,
        },
        policy::{Agreement, MessageOffer},
    },
    negotiation::{
        Connector, ContractNegotiation, FinalizedData, NegotiationEvent, NegotiationEventPayload,
        NegotiationState, NegotiationStateData, TerminatedData, VerifiedData,
    },
    store::Store,
};

#[derive(Serialize, Deserialize)]
pub(crate) struct ConsumerView;

#[derive(Serialize, Deserialize)]
pub(crate) struct RequestedDataPayload {
    pub(crate) offer: MessageOffer,
    pub(crate) provider_pid: Option<String>,
}
pub(crate) type RequestedData = PendingStateData<RequestedDataPayload>;

#[derive(Serialize, Deserialize)]
pub(crate) struct OfferedData {
    offer: MessageOffer,
}
impl Pending for OfferedData {}

#[derive(Serialize, Deserialize)]
pub(crate) struct AcceptedDataPayload {
    offer: MessageOffer,
}
pub(crate) type AcceptedData = PendingStateData<AcceptedDataPayload>;

#[derive(Serialize, Deserialize)]
pub(crate) struct AgreedData {
    agreement: Agreement,
}
impl Pending for AgreedData {}

impl NegotiationStateData for ConsumerView {
    type RequestedData = RequestedData;
    type OfferedData = OfferedData;
    type AcceptedData = AcceptedData;
    type AgreedData = AgreedData;
}

impl ContractNegotiation<ConsumerView> {
    pub(crate) async fn tick<T: Store>(
        #[cfg_attr(not(feature = "tck"), allow(unused_mut))] mut self,
        state: &AppStateNegotiation<T>,
        connector: &Connector,
    ) -> anyhow::Result<Option<Self>> {
        #[cfg(feature = "tck")]
        {
            self = match tck::tick(self).await {
                Ok(tck_result) => return tck_result,
                Err(this) => this,
            };
        }

        let iss_did_web = state.participant_info.did_web()?;
        let get_access_token = || async {
            state
                .authenticator
                .get_token(&state.client, &connector.address, iss_did_web)
                .await
        };

        let connector_address = connector.api_address();
        let callback_address = state.participant_info.callback_address();

        match self.state {
            NegotiationState::Requested(s) if s.is_pending() => {
                let (url, request) = match &s.payload.provider_pid {
                    None => (
                        format!("{}/negotiations/request", connector_address),
                        ContractRequest::make_initial(
                            self.consumer_pid.clone(),
                            ContractRequestOffer::Concrete(s.payload.offer),
                            callback_address,
                        ),
                    ),
                    Some(pid) => (
                        format!("{}/negotiations/{}/request", connector_address, pid),
                        ContractRequest::make_secondary(
                            self.consumer_pid.clone(),
                            ContractRequestOffer::Concrete(s.payload.offer),
                            pid.clone(),
                        ),
                    ),
                };
                post_request(
                    &state.client,
                    &state.validator,
                    &url,
                    Some(get_access_token().await?),
                    request,
                    |request, response: Option<model::contract::ContractNegotiation>| match (
                        response,
                        request.offer,
                    ) {
                        (None, _) => unreachable!(),
                        (Some(response), ContractRequestOffer::Concrete(offer)) => {
                            ContractNegotiation {
                                provider_pid: response.provider_pid.clone(),
                                consumer_pid: self.consumer_pid,
                                state: NegotiationState::Requested(RequestedData {
                                    sent: true,
                                    payload: RequestedDataPayload {
                                        offer: offer,
                                        provider_pid: Some(response.provider_pid),
                                    },
                                }),
                            }
                        }
                        (Some(_), ContractRequestOffer::Reference(_)) => unimplemented!(),
                    },
                )
                .await
                .map(Some)
            }
            // TODO: should we automatically accept offers?
            NegotiationState::Offered(s) => Ok(Some(ContractNegotiation {
                provider_pid: self.provider_pid,
                consumer_pid: self.consumer_pid,
                state: NegotiationState::Accepted(AcceptedData {
                    sent: false,
                    payload: AcceptedDataPayload { offer: s.offer },
                }),
            })),
            NegotiationState::Accepted(s) if s.is_pending() => {
                let event = model::contract::ContractNegotiation::accept(
                    self.provider_pid,
                    self.consumer_pid,
                );
                post_request(
                    &state.client,
                    &state.validator,
                    &format!(
                        "{}/negotiations/{}/events",
                        connector_address, event.provider_pid,
                    ),
                    Some(get_access_token().await?),
                    event,
                    |event, _: Option<()>| ContractNegotiation {
                        provider_pid: event.provider_pid,
                        consumer_pid: event.consumer_pid,
                        state: NegotiationState::Accepted(AcceptedData {
                            sent: true,
                            payload: AcceptedDataPayload {
                                offer: s.payload.offer,
                            },
                        }),
                    },
                )
                .await
                .map(Some)
            }
            NegotiationState::Agreed(s) => {
                let verification = model::contract::ContractNegotiation::verify(
                    self.provider_pid,
                    self.consumer_pid,
                );
                post_request(
                    &state.client,
                    &state.validator,
                    &format!(
                        "{}/negotiations/{}/agreement/verification",
                        connector_address, verification.provider_pid
                    ),
                    Some(get_access_token().await?),
                    verification,
                    |verification, _: Option<()>| ContractNegotiation {
                        provider_pid: verification.provider_pid,
                        consumer_pid: verification.consumer_pid,
                        state: NegotiationState::Verified(VerifiedData {
                            agreement: s.agreement,
                        }),
                    },
                )
                .await
                .map(Some)
            }
            NegotiationState::Terminated(s) if s.is_pending() => {
                let termination = model::contract::ContractNegotiation::terminate(
                    self.provider_pid,
                    self.consumer_pid,
                    s.payload.code,
                    s.payload.reason,
                );
                post_request(
                    &state.client,
                    &state.validator,
                    &format!(
                        "{}/negotiations/{}/termination",
                        connector_address, termination.provider_pid
                    ),
                    Some(get_access_token().await?),
                    termination,
                    |termination, _: Option<()>| ContractNegotiation {
                        provider_pid: termination.provider_pid,
                        consumer_pid: termination.consumer_pid,
                        state: NegotiationState::Terminated(TerminatedData {
                            sent: true,
                            payload: TerminatedDataPayload {
                                code: termination.code,
                                reason: termination.reason,
                            },
                        }),
                    },
                )
                .await
                .map(Some)
            }
            _ => Ok(None),
        }
    }

    #[allow(unused)]
    fn is_assigner(&self, agreement: &ContractAgreement, subject: &str) -> bool {
        #[cfg(feature = "tck")]
        {
            true
        }
        #[cfg(not(feature = "tck"))]
        {
            agreement.agreement.assigner == subject
        }
    }

    pub(crate) async fn handle_event(
        self,
        claims: AuthClaims,
        event: NegotiationEventPayload,
    ) -> Result<Self, AppError> {
        let subject = claims.subject()?;

        match event {
            NegotiationEventPayload::Offer(offer) => match self.state {
                NegotiationState::Requested(s) if !s.is_pending() => Ok(ContractNegotiation {
                    provider_pid: self.provider_pid,
                    consumer_pid: self.consumer_pid,
                    state: NegotiationState::Offered(OfferedData { offer: offer.offer }),
                }),
                _ => Err(self.invalid_state_error()),
            },
            NegotiationEventPayload::Agree(agreement) if self.is_assigner(&agreement, subject) => {
                match self.state {
                    NegotiationState::Accepted(_) | NegotiationState::Requested(_) => {
                        // TODO: check agreement?
                        Ok(ContractNegotiation {
                            provider_pid: self.provider_pid,
                            consumer_pid: self.consumer_pid,
                            state: NegotiationState::Agreed(AgreedData {
                                agreement: agreement.agreement,
                            }),
                        })
                    }
                    _ => Err(self.invalid_state_error()),
                }
            }
            NegotiationEventPayload::Event(event) => match (&self.state, event.event_type) {
                (NegotiationState::Verified(_), ContractNegotiationEventType::Finalized) => {
                    if let NegotiationState::Verified(s) = self.state {
                        Ok(ContractNegotiation {
                            provider_pid: self.provider_pid,
                            consumer_pid: self.consumer_pid,
                            state: NegotiationState::Finalized(FinalizedData {
                                agreement: s.agreement,
                            }),
                        })
                    } else {
                        unreachable!()
                    }
                }
                _ => Err(self.invalid_state_error()),
            },
            NegotiationEventPayload::Terminate(termination) => match self.state {
                NegotiationState::Finalized(_) | NegotiationState::Terminated(_) => {
                    Err(self.invalid_state_error())
                }
                _ => Ok(ContractNegotiation {
                    provider_pid: self.provider_pid,
                    consumer_pid: self.consumer_pid,
                    state: NegotiationState::Terminated(TerminatedData {
                        sent: true,
                        payload: TerminatedDataPayload {
                            code: termination.code,
                            reason: termination.reason,
                        },
                    }),
                }),
            },
            _ => Err(self.invalid_state_error()),
        }
    }
}

pub(crate) async fn contract_offer<T: Store>(
    claims: AuthClaims,
    State(state): State<AppStateNegotiation<T>>,
    Path(consumer_pid): Path<String>,
    Json(offer): Json<ContractOffer>,
) -> Result<impl IntoResponse, AppError> {
    debug!("Contract offer with claims: {:?}", claims);

    let (tx, rx) = oneshot::channel();
    state
        .tx
        .send(NegotiationEvent {
            pid: consumer_pid,
            claims,
            payload: NegotiationEventPayload::Offer(offer),
            result: tx,
        })
        .await
        .map_err(anyhow::Error::new)?;

    rx.await
        .map_err(anyhow::Error::new)?
        .map(|_| StatusCode::OK)
}

pub(crate) async fn contract_agreement<T: Store>(
    claims: AuthClaims,
    State(state): State<AppStateNegotiation<T>>,
    Path(consumer_pid): Path<String>,
    Json(agreement): Json<ContractAgreement>,
) -> Result<impl IntoResponse, AppError> {
    debug!("Contract agreement with claims: {:?}", claims);

    let (tx, rx) = oneshot::channel();
    state
        .tx
        .send(NegotiationEvent {
            pid: consumer_pid,
            claims,
            payload: NegotiationEventPayload::Agree(agreement),
            result: tx,
        })
        .await
        .map_err(anyhow::Error::new)?;

    rx.await
        .map_err(anyhow::Error::new)?
        .map(|_| StatusCode::OK)
}

#[cfg(feature = "tck")]
pub(crate) mod tck {
    use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
    use serde::Deserialize;

    use crate::{
        AppError,
        connector::{
            Pending, TerminatedData, TerminatedDataPayload, app_state::AppStateNegotiation,
        },
        model::policy::MessageOffer,
        negotiation::{
            ContractNegotiation, NegotiationState,
            consumer::{
                AcceptedData, AcceptedDataPayload, Connector, ConsumerView, RequestedData,
                RequestedDataPayload,
            },
            negotiate,
        },
        store::Store,
    };

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct TckNegotiationsRequest {
        #[serde(flatten)]
        connector: Connector,
        offer_id: String,
        dataset_id: String,
    }

    pub(crate) async fn negotiation_request<T: Store>(
        State(state): State<AppStateNegotiation<T>>,
        Json(request): Json<TckNegotiationsRequest>,
    ) -> Result<impl IntoResponse, AppError> {
        let offer = MessageOffer::new_tck(request.dataset_id, request.offer_id);

        negotiate(state.store.as_ref(), offer, request.connector).await?;

        Ok(StatusCode::OK)
    }

    type ConsumerNegotiation = ContractNegotiation<ConsumerView>;

    pub(crate) async fn tick(
        negotiation: ConsumerNegotiation,
    ) -> Result<anyhow::Result<Option<ConsumerNegotiation>>, ConsumerNegotiation> {
        let state = match negotiation.state {
            NegotiationState::Requested(s) if !s.is_pending() => {
                match s.payload.offer.target.as_str() {
                    "ACNC0202" => {
                        return Ok(Ok(Some(ContractNegotiation {
                            provider_pid: negotiation.provider_pid,
                            consumer_pid: negotiation.consumer_pid,
                            state: NegotiationState::Terminated(TerminatedData {
                                sent: false,
                                payload: TerminatedDataPayload {
                                    code: Some("999".into()),
                                    reason: Some(vec!["TCK ACNC0202".into()]),
                                },
                            }),
                        })));
                    }
                    _ => NegotiationState::Requested(s),
                }
            }
            NegotiationState::Offered(s) => match s.offer.target.as_str() {
                "ACNC0101" | "ACNC0205" | "ACNC0304" | "ACNC0305" | "ACNC0306" => {
                    return Ok(Ok(Some(ContractNegotiation {
                        provider_pid: negotiation.provider_pid,
                        consumer_pid: negotiation.consumer_pid,
                        state: NegotiationState::Accepted(AcceptedData {
                            sent: false,
                            payload: AcceptedDataPayload { offer: s.offer },
                        }),
                    })));
                }
                "ACNC0102" => {
                    return Ok(Ok(Some(ContractNegotiation {
                        provider_pid: negotiation.provider_pid.clone(),
                        consumer_pid: negotiation.consumer_pid,
                        state: NegotiationState::Requested(RequestedData {
                            sent: false,
                            payload: RequestedDataPayload {
                                offer: s.offer,
                                provider_pid: Some(negotiation.provider_pid),
                            },
                        }),
                    })));
                }
                "ACNC0103" | "ACNC0204" => {
                    return Ok(Ok(Some(ContractNegotiation {
                        provider_pid: negotiation.provider_pid,
                        consumer_pid: negotiation.consumer_pid,
                        state: NegotiationState::Terminated(TerminatedData {
                            sent: false,
                            payload: TerminatedDataPayload {
                                code: Some("999".into()),
                                reason: Some(vec!["TCK ACNC0103|ACNC0204".into()]),
                            },
                        }),
                    })));
                }
                _ => NegotiationState::Offered(s),
            },
            NegotiationState::Agreed(s) => match s.agreement.target.as_str() {
                "ACNC0203" => {
                    return Ok(Ok(Some(ContractNegotiation {
                        provider_pid: negotiation.provider_pid,
                        consumer_pid: negotiation.consumer_pid,
                        state: NegotiationState::Terminated(TerminatedData {
                            sent: false,
                            payload: TerminatedDataPayload {
                                code: Some("999".into()),
                                reason: Some(vec!["TCK ACNC0203".into()]),
                            },
                        }),
                    })));
                }
                _ => NegotiationState::Agreed(s),
            },
            s => s,
        };

        Err(ConsumerNegotiation {
            provider_pid: negotiation.provider_pid,
            consumer_pid: negotiation.consumer_pid,
            state,
        })
    }
}
