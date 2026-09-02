use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tracing::{debug, error};

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
            ContractAgreementVerification, ContractNegotiationError, ContractNegotiationEventType,
            ContractOffer, ContractRequest, ContractRequestOffer,
        },
        policy::{Agreement, MessageOffer},
    },
    negotiation::{
        ContractNegotiation, FinalizedData, Negotiation, NegotiationEvent, NegotiationEventPayload,
        NegotiationState, NegotiationStateData, TerminatedData, VerifiedData,
    },
    store::Store,
};

#[derive(Serialize, Deserialize)]
pub(crate) struct ProviderView;

#[derive(Serialize, Deserialize)]
pub(crate) struct RequestedData {
    offer: MessageOffer,
    claims: AuthClaims,

    #[cfg(feature = "tck")]
    mark_for_termination: bool,
}
impl Pending for RequestedData {}

#[derive(Serialize, Deserialize)]
pub(crate) struct AgreedDataPayload {
    agreement: Agreement,
}
pub(crate) type AgreedData = PendingStateData<AgreedDataPayload>;

#[derive(Serialize, Deserialize)]
pub(crate) struct OfferedDataPayload {
    offer: MessageOffer,
    assignee: String,
}
pub(crate) type OfferedData = PendingStateData<OfferedDataPayload>;

#[derive(Serialize, Deserialize)]
pub(crate) struct AcceptedData {
    offer: MessageOffer,
    assignee: String,
}
impl Pending for AcceptedData {}

impl NegotiationStateData for ProviderView {
    type RequestedData = RequestedData;
    type OfferedData = OfferedData;
    type AcceptedData = AcceptedData;
    type AgreedData = AgreedData;
}

impl ContractNegotiation<ProviderView> {
    pub(crate) async fn tick<T: Store>(
        #[cfg_attr(not(feature = "tck"), allow(unused_mut))] mut self,
        state: &AppStateNegotiation<T>,
        callback_address: &str,
    ) -> anyhow::Result<Option<Self>> {
        #[cfg(feature = "tck")]
        {
            self = match tck::tick(self).await {
                Ok(tck_result) => return tck_result,
                Err(this) => this,
            };
        }

        let my_did_web = state.participant_info.did_web()?;
        let get_access_token = || async {
            state
                .authenticator
                .get_token(&state.client, callback_address, my_did_web.clone())
                .await
        };

        // NOTE: currently only 2025-1 supported
        // TODO: we should retrieve the metadata and determine the path for version 2025-1
        #[cfg(not(feature = "tck"))]
        let callback_address = format!(
            "{}{}",
            callback_address,
            crate::connector::DSP_API_PATH_2025_1
        );

        match self.state {
            NegotiationState::Requested(s) => {
                #[allow(unused)]
                let Some(dataset) = state.store.get_matching_dataset(&s.offer).await? else {
                    error!("Unknown dataset with id {}", s.offer.target);
                    return Ok(Some(ContractNegotiation {
                        provider_pid: self.provider_pid,
                        consumer_pid: self.consumer_pid,
                        state: NegotiationState::Terminated(TerminatedData {
                            sent: false,
                            payload: TerminatedDataPayload {
                                code: Some("1".into()),
                                reason: Some(vec!["dataset not found".into()]),
                            },
                        }),
                    }));
                };

                #[cfg(feature = "tck")]
                let policy = s.offer.policy_class.policy;

                #[cfg(not(feature = "tck"))]
                let Some(policy) = dataset.dataset.has_policy.into_iter().find_map(|required| {
                    match crate::policy_engine::negotiation::negotiate_policy(
                        &required.policy_class.policy,
                        &s.offer.policy_class.policy,
                    ) {
                        crate::policy_engine::negotiation::NegotiationOutcome::Success(policy) => {
                            Some(policy)
                        }
                        crate::policy_engine::negotiation::NegotiationOutcome::Forbidden(_) => None,
                    }
                }) else {
                    error!(
                        "Could not negotiate policy for dataset {}",
                        s.offer.policy_class.resource.id,
                    );
                    return Ok(Some(ContractNegotiation {
                        provider_pid: self.provider_pid,
                        consumer_pid: self.consumer_pid,
                        state: NegotiationState::Terminated(TerminatedData {
                            sent: false,
                            payload: TerminatedDataPayload {
                                code: Some("2".into()),
                                reason: Some(vec!["policy negotiation failed".into()]),
                            },
                        }),
                    }));
                };

                Ok(Some(ContractNegotiation {
                    provider_pid: self.provider_pid,
                    consumer_pid: self.consumer_pid,
                    state: NegotiationState::Offered(OfferedData {
                        sent: false,
                        payload: OfferedDataPayload {
                            offer: MessageOffer::new(s.offer.target, policy),
                            assignee: s.claims.subject()?.into(),
                        },
                    }),
                }))
            }
            NegotiationState::Offered(s) if s.is_pending() => {
                let offer = ContractOffer::make_secondary(
                    self.provider_pid.clone(),
                    s.payload.offer,
                    self.consumer_pid.clone(),
                );
                post_request(
                    &state.client,
                    &state.validator,
                    &format!(
                        "{}/negotiations/{}/offers",
                        callback_address, self.consumer_pid
                    ),
                    Some(get_access_token().await?),
                    offer,
                    |offer, _: Option<()>| ContractNegotiation {
                        provider_pid: self.provider_pid,
                        consumer_pid: self.consumer_pid,
                        state: NegotiationState::Offered(OfferedData {
                            sent: true,
                            payload: OfferedDataPayload {
                                offer: offer.offer,
                                assignee: s.payload.assignee,
                            },
                        }),
                    },
                )
                .await
                .map(Some)
            }
            NegotiationState::Accepted(s) => {
                let id = format!("urn:uuid:{}", uuid::Uuid::new_v4().to_string());
                let agreement = Agreement::new(id, s.offer, my_did_web, s.assignee);
                Ok(Some(ContractNegotiation {
                    provider_pid: self.provider_pid,
                    consumer_pid: self.consumer_pid,
                    state: NegotiationState::Agreed(AgreedData {
                        sent: false,
                        payload: AgreedDataPayload { agreement },
                    }),
                }))
            }
            NegotiationState::Agreed(s) if s.is_pending() => {
                let agreement = model::contract::ContractNegotiation::agree(
                    self.provider_pid,
                    self.consumer_pid,
                    s.payload.agreement,
                );
                post_request(
                    &state.client,
                    &state.validator,
                    &format!(
                        "{}/negotiations/{}/agreement",
                        callback_address, agreement.consumer_pid
                    ),
                    Some(get_access_token().await?),
                    agreement,
                    |agreement, _: Option<()>| ContractNegotiation {
                        provider_pid: agreement.provider_pid,
                        consumer_pid: agreement.consumer_pid,
                        state: NegotiationState::Agreed(AgreedData {
                            sent: true,
                            payload: AgreedDataPayload {
                                agreement: agreement.agreement,
                            },
                        }),
                    },
                )
                .await
                .map(Some)
            }
            NegotiationState::Verified(s) => {
                let event = model::contract::ContractNegotiation::finalize(
                    self.provider_pid.clone(),
                    self.consumer_pid.clone(),
                );
                post_request(
                    &state.client,
                    &state.validator,
                    &format!(
                        "{}/negotiations/{}/events",
                        callback_address, self.consumer_pid
                    ),
                    Some(get_access_token().await?),
                    event,
                    |_, _: Option<()>| ContractNegotiation {
                        provider_pid: self.provider_pid,
                        consumer_pid: self.consumer_pid,
                        state: NegotiationState::Finalized(FinalizedData {
                            agreement: s.agreement,
                        }),
                    },
                )
                .await
                .map(Some)
            }
            NegotiationState::Terminated(s) if s.is_pending() => {
                let termination = model::contract::ContractNegotiation::terminate(
                    self.provider_pid.clone(),
                    self.consumer_pid.clone(),
                    s.payload.code,
                    s.payload.reason,
                );
                post_request(
                    &state.client,
                    &state.validator,
                    &format!(
                        "{}/negotiations/{}/termination",
                        callback_address, self.consumer_pid
                    ),
                    Some(get_access_token().await?),
                    termination,
                    |termination, _: Option<()>| ContractNegotiation {
                        provider_pid: self.provider_pid,
                        consumer_pid: self.consumer_pid,
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

    pub(crate) async fn handle_event(
        self,
        claims: AuthClaims,
        event: NegotiationEventPayload,
    ) -> Result<Self, AppError> {
        let subject = claims.subject()?;
        match event {
            NegotiationEventPayload::Update(Negotiation::Provider { contract, .. }) => {
                match (&self.state, contract.state) {
                    (NegotiationState::Offered(os), NegotiationState::Requested(s))
                        if os.payload.assignee == subject =>
                    {
                        Ok(Self {
                            provider_pid: self.provider_pid,
                            consumer_pid: self.consumer_pid,
                            state: NegotiationState::Requested(s),
                        })
                    }
                    _ => Err(self.invalid_state_error()),
                }
            }
            NegotiationEventPayload::Event(event) => match (&self.state, event.event_type) {
                (NegotiationState::Offered(os), ContractNegotiationEventType::Accepted)
                    if os.payload.assignee == subject =>
                {
                    if let NegotiationState::Offered(s) = self.state {
                        Ok(Self {
                            provider_pid: self.provider_pid,
                            consumer_pid: self.consumer_pid,
                            state: NegotiationState::Accepted(AcceptedData {
                                offer: s.payload.offer,
                                assignee: s.payload.assignee,
                            }),
                        })
                    } else {
                        unreachable!()
                    }
                }
                _ => Err(self.invalid_state_error()),
            },
            NegotiationEventPayload::Verify(_) => match self.state {
                NegotiationState::Agreed(s) if s.payload.agreement.assignee == subject => {
                    Ok(Self {
                        provider_pid: self.provider_pid,
                        consumer_pid: self.consumer_pid,
                        state: NegotiationState::Verified(VerifiedData {
                            agreement: s.payload.agreement,
                        }),
                    })
                }
                _ => Err(self.invalid_state_error()),
            },
            NegotiationEventPayload::Terminate(termination) => match self.state {
                NegotiationState::Finalized(_) | NegotiationState::Terminated(_) => {
                    Err(self.invalid_state_error())
                }
                // TODO: ideally we should verify the subject similar to the above states
                _ => Ok(Self {
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

pub(crate) async fn contract_request<T: Store>(
    claims: AuthClaims,
    State(state): State<AppStateNegotiation<T>>,
    Json(request): Json<ContractRequest>,
) -> Result<impl IntoResponse, AppError> {
    debug!("Contract request with claims: {:?}", claims);

    let Some(callback_address) = request.callback_address else {
        return Err(AppError::Contract(ContractNegotiationError::new(
            "".into(),
            request.consumer_pid,
            StatusCode::BAD_REQUEST,
            "callback_address missing".into(),
        )));
    };
    let offer = match request.offer {
        ContractRequestOffer::Reference { .. } => unimplemented!(),
        ContractRequestOffer::Concrete(offer) => offer,
    };

    let provider_pid = format!("urn:uuid:{}", uuid::Uuid::new_v4().to_string());
    let contract = ContractNegotiation {
        provider_pid,
        consumer_pid: request.consumer_pid,
        state: NegotiationState::Requested(RequestedData {
            offer,
            claims,
            #[cfg(feature = "tck")]
            mark_for_termination: false,
        }),
    };
    let n = Negotiation::Provider {
        contract,
        callback_address,
    };
    state.store.save_negotiation(&n).await?;

    Ok((
        StatusCode::CREATED,
        Json(model::contract::ContractNegotiation::from(n)),
    ))
}

pub(crate) async fn contract_request_counter<T: Store>(
    claims: AuthClaims,
    State(state): State<AppStateNegotiation<T>>,
    Path(provider_pid): Path<String>,
    Json(request): Json<ContractRequest>,
) -> Result<impl IntoResponse, AppError> {
    debug!("Contract request counter with claims: {:?}", claims);

    let offer = match request.offer {
        ContractRequestOffer::Reference { .. } => unimplemented!(),
        ContractRequestOffer::Concrete(offer) => offer,
    };

    let (tx, rx) = oneshot::channel();

    #[cfg(feature = "tck")]
    let mark_for_termination = offer.target.as_str() == "ACN0102";

    let contract = ContractNegotiation {
        provider_pid: provider_pid.clone(),
        consumer_pid: request.consumer_pid,
        state: NegotiationState::Requested(RequestedData {
            offer,
            claims: claims.clone(),
            #[cfg(feature = "tck")]
            mark_for_termination,
        }),
    };
    let n = Negotiation::Provider {
        contract,
        callback_address: request.callback_address.unwrap_or_default(),
    };

    state
        .tx
        .send(NegotiationEvent {
            pid: provider_pid,
            claims,
            payload: NegotiationEventPayload::Update(n),
            result: tx,
        })
        .await
        .map_err(anyhow::Error::new)?;

    rx.await
        .map_err(anyhow::Error::new)?
        .map(|n| Json(model::contract::ContractNegotiation::from(n)))
}

pub(crate) async fn contract_verification<T: Store>(
    claims: AuthClaims,
    State(state): State<AppStateNegotiation<T>>,
    Path(provider_pid): Path<String>,
    Json(request): Json<ContractAgreementVerification>,
) -> Result<impl IntoResponse, AppError> {
    debug!("Contract verification with claims: {:?}", claims);

    let (tx, rx) = oneshot::channel();
    state
        .tx
        .send(NegotiationEvent {
            pid: provider_pid,
            claims,
            payload: NegotiationEventPayload::Verify(request),
            result: tx,
        })
        .await
        .map_err(anyhow::Error::new)?;

    rx.await
        .map_err(anyhow::Error::new)?
        .map(|_| StatusCode::OK)
}

#[cfg(feature = "tck")]
mod tck {
    use crate::{
        connector::{Pending, TerminatedDataPayload},
        model::policy::Agreement,
        negotiation::{
            ContractNegotiation, NegotiationState, TerminatedData,
            provider::{AgreedData, AgreedDataPayload, ProviderView},
        },
    };

    type ProviderNegotiation = ContractNegotiation<ProviderView>;

    pub(crate) async fn tick(
        negotiation: ProviderNegotiation,
    ) -> Result<
        anyhow::Result<Option<ContractNegotiation<ProviderView>>>,
        ContractNegotiation<ProviderView>,
    > {
        let state = match negotiation.state {
            NegotiationState::Requested(s) => {
                if s.mark_for_termination {
                    return Ok(Ok(Some(ContractNegotiation {
                        provider_pid: negotiation.provider_pid,
                        consumer_pid: negotiation.consumer_pid,
                        state: NegotiationState::Terminated(TerminatedData {
                            sent: false,
                            payload: TerminatedDataPayload {
                                code: Some("3".into()),
                                reason: Some(vec!["marked for termination".into()]),
                            },
                        }),
                    })));
                }
                match s.offer.target.as_str() {
                    "ACN0104" | "ACN0203" | "ACN0207" | "ACN0301" => {
                        let id = format!("urn:uuid:{}", uuid::Uuid::new_v4().to_string());
                        // FIXME: assigner and assignee empty
                        let agreement = Agreement::new(id, s.offer, "".to_owned(), "".to_owned());
                        return Ok(Ok(Some(ContractNegotiation {
                            provider_pid: negotiation.provider_pid,
                            consumer_pid: negotiation.consumer_pid,
                            state: NegotiationState::Agreed(AgreedData {
                                sent: false,
                                payload: AgreedDataPayload { agreement },
                            }),
                        })));
                    }
                    "ACN0201" => {
                        return Ok(Ok(Some(ContractNegotiation {
                            provider_pid: negotiation.provider_pid,
                            consumer_pid: negotiation.consumer_pid,
                            state: NegotiationState::Terminated(TerminatedData {
                                sent: false,
                                payload: TerminatedDataPayload {
                                    code: Some("999".into()),
                                    reason: Some(vec!["TCK ACN0201".into()]),
                                },
                            }),
                        })));
                    }
                    _ => NegotiationState::Requested(s),
                }
            }
            NegotiationState::Offered(s) if !s.is_pending() => {
                match s.payload.offer.target.as_str() {
                    "ACN0205" => {
                        return Ok(Ok(Some(ContractNegotiation {
                            provider_pid: negotiation.provider_pid,
                            consumer_pid: negotiation.consumer_pid,
                            state: NegotiationState::Terminated(TerminatedData {
                                sent: false,
                                payload: TerminatedDataPayload {
                                    code: Some("999".into()),
                                    reason: Some(vec!["TCK ACN0205".into()]),
                                },
                            }),
                        })));
                    }
                    _ => NegotiationState::Offered(s),
                }
            }
            NegotiationState::Accepted(s) => match s.offer.target.as_str() {
                "ACN0206" => {
                    return Ok(Ok(Some(ContractNegotiation {
                        provider_pid: negotiation.provider_pid,
                        consumer_pid: negotiation.consumer_pid,
                        state: NegotiationState::Terminated(TerminatedData {
                            sent: false,
                            payload: TerminatedDataPayload {
                                code: Some("999".into()),
                                reason: Some(vec!["TCK ACN0206".into()]),
                            },
                        }),
                    })));
                }
                _ => NegotiationState::Accepted(s),
            },
            NegotiationState::Verified(s) => match s.agreement.target.as_str() {
                "ACN0207" => {
                    return Ok(Ok(Some(ContractNegotiation {
                        provider_pid: negotiation.provider_pid,
                        consumer_pid: negotiation.consumer_pid,
                        state: NegotiationState::Terminated(TerminatedData {
                            sent: false,
                            payload: TerminatedDataPayload {
                                code: Some("999".into()),
                                reason: Some(vec!["TCK ACN0207".into()]),
                            },
                        }),
                    })));
                }
                _ => NegotiationState::Verified(s),
            },
            s => s,
        };

        Err(ProviderNegotiation {
            provider_pid: negotiation.provider_pid,
            consumer_pid: negotiation.consumer_pid,
            state,
        })
    }
}
