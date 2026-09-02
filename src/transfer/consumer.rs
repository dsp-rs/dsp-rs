use serde::{Deserialize, Serialize};

use crate::{
    connector::{Pending, TerminatedDataPayload, app_state::AppStateTransfer, post_request},
    model::{
        self,
        policy::Agreement,
        transfer::{DataAddress, TransferRequest},
    },
    negotiation::Connector,
    store::Store,
    transfer::{
        CompletedData, PendingStateData, StartedData, StartedDataPayload, SuspendedData,
        SuspendedDataPayload, TerminatedData, TransferProcess, TransferState, TransferStateData,
    },
};

#[derive(Serialize, Deserialize)]
pub(crate) struct ConsumerView;

pub(crate) type RequestedData = PendingStateData<()>;

impl TransferStateData for ConsumerView {
    type RequestedData = RequestedData;
}

impl TransferProcess<ConsumerView> {
    pub(crate) async fn tick<T: Store>(
        #[cfg_attr(not(feature = "tck"), allow(unused_mut))] mut self,
        state: &AppStateTransfer<T>,
        agreement: &Agreement,
        format: &String,
        connector: &Connector,
        data_address: &Option<DataAddress>,
    ) -> anyhow::Result<Option<Self>> {
        #[cfg(feature = "tck")]
        {
            self = match tck::tick(self, agreement).await {
                Ok(tck_result) => return tck_result,
                Err(this) => this,
            };
        }

        let my_did_web = state.participant_info.did_web()?;
        let did = my_did_web.clone();
        let get_access_token = || async {
            state
                .authenticator
                .get_token(&state.client, &connector.address, did)
                .await
        };

        let connector_address = connector.api_address();
        let callback_address = state.participant_info.callback_address();

        // TODO: the push endpoint should be set with the transfer start request
        let push_endpoint = format!("{}/push", state.participant_info.external_address);

        match self.state {
            TransferState::Requested(s) if s.is_pending() => {
                let request = TransferRequest::new(
                    agreement.policy_class.resource.id.clone(),
                    format.clone(),
                    callback_address,
                    self.consumer_pid,
                    data_address.clone(),
                );
                post_request(
                    &state.client,
                    &state.validator,
                    &format!("{}/transfers/request", connector_address),
                    Some(get_access_token().await?),
                    request,
                    |request, response: Option<model::transfer::TransferProcess>| TransferProcess {
                        provider_pid: response.expect("should have a response").provider_pid,
                        consumer_pid: request.consumer_pid,
                        state: TransferState::Requested(RequestedData {
                            sent: true,
                            payload: (),
                        }),
                    },
                )
                .await
                .map(Some)
            }
            TransferState::Started(s) if s.is_pending() => {
                // TODO: code duplication with provider
                let token = state
                    .authenticator
                    .new_transfer_token(self.consumer_pid.clone(), my_did_web)?;
                let start = model::transfer::TransferProcess::start(
                    self.provider_pid,
                    self.consumer_pid,
                    Some(DataAddress::new_http_with_token(push_endpoint, token)), // FIXME: data address only if PUSH?
                );
                post_request(
                    &state.client,
                    &state.validator,
                    &format!(
                        "{}/transfers/{}/start",
                        connector_address, &start.provider_pid
                    ),
                    Some(get_access_token().await?),
                    start,
                    |start, _: Option<()>| TransferProcess {
                        provider_pid: start.provider_pid,
                        consumer_pid: start.consumer_pid,
                        state: TransferState::Started(StartedData {
                            sent: true,
                            payload: StartedDataPayload {
                                #[cfg(feature = "tck")]
                                count: s.payload.count,
                            },
                        }),
                    },
                )
                .await
                .map(Some)
            }
            TransferState::Suspended(s) if s.is_pending() => {
                let suspend = model::transfer::TransferProcess::suspend(
                    self.provider_pid,
                    self.consumer_pid,
                    s.payload.code,
                    s.payload.reason,
                );
                post_request(
                    &state.client,
                    &state.validator,
                    &format!(
                        "{}/transfers/{}/suspension",
                        connector_address, &suspend.provider_pid
                    ),
                    Some(get_access_token().await?),
                    suspend,
                    |suspend, _: Option<()>| TransferProcess {
                        provider_pid: suspend.provider_pid,
                        consumer_pid: suspend.consumer_pid,
                        state: TransferState::Suspended(SuspendedData {
                            sent: true,
                            payload: SuspendedDataPayload {
                                #[cfg(feature = "tck")]
                                count: s.payload.count,
                                code: suspend.code,
                                reason: suspend.reason,
                            },
                        }),
                    },
                )
                .await
                .map(Some)
            }
            TransferState::Completed(s) if s.is_pending() => {
                let complete = model::transfer::TransferProcess::complete(
                    self.provider_pid,
                    self.consumer_pid,
                );
                post_request(
                    &state.client,
                    &state.validator,
                    &format!(
                        "{}/transfers/{}/completion",
                        connector_address, &complete.provider_pid
                    ),
                    Some(get_access_token().await?),
                    complete,
                    |complete, _: Option<()>| TransferProcess {
                        provider_pid: complete.provider_pid,
                        consumer_pid: complete.consumer_pid,
                        state: TransferState::Completed(CompletedData {
                            sent: true,
                            payload: (),
                        }),
                    },
                )
                .await
                .map(Some)
            }
            TransferState::Terminated(s) if s.is_pending() => {
                let terminate = model::transfer::TransferProcess::terminate(
                    self.provider_pid,
                    self.consumer_pid,
                    s.payload.code,
                    s.payload.reason,
                );
                post_request(
                    &state.client,
                    &state.validator,
                    &format!(
                        "{}/transfers/{}/termination",
                        connector_address, &terminate.provider_pid
                    ),
                    Some(get_access_token().await?),
                    terminate,
                    |terminate, _: Option<()>| TransferProcess {
                        provider_pid: terminate.provider_pid,
                        consumer_pid: terminate.consumer_pid,
                        state: TransferState::Terminated(TerminatedData {
                            sent: true,
                            payload: TerminatedDataPayload {
                                code: terminate.code,
                                reason: terminate.reason,
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
}

#[cfg(feature = "tck")]
pub(crate) mod tck {
    use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
    use serde::Deserialize;

    use crate::{
        AppError,
        connector::{Pending, TerminatedDataPayload, app_state::AppStateTransfer},
        model::policy::Agreement,
        store::Store,
        transfer::{
            CompletedData, StartedData, StartedDataPayload, SuspendedData, SuspendedDataPayload,
            TerminatedData, TransferProcess, TransferState,
            consumer::{Connector, ConsumerView},
            transfer,
        },
    };

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct TckTransferRequest {
        #[serde(flatten)]
        connector: Connector,
        agreement_id: String,
        format: String,
    }

    pub(crate) async fn transfer_request<T: Store>(
        State(state): State<AppStateTransfer<T>>,
        Json(request): Json<TckTransferRequest>,
    ) -> Result<impl IntoResponse, AppError> {
        // FIXME: assigner and assignee empty
        let agreement = Agreement::new_tck(request.agreement_id, "".to_owned(), "".to_owned());

        transfer(
            state.store.as_ref(),
            agreement,
            request.format,
            request.connector,
        )
        .await?;

        Ok(StatusCode::OK)
    }

    type Process = TransferProcess<ConsumerView>;

    pub(crate) async fn tick(
        process: Process,
        agreement: &Agreement,
    ) -> Result<anyhow::Result<Option<Process>>, Process> {
        let state = match process.state {
            TransferState::Requested(s) if !s.is_pending() => {
                match agreement.policy_class.resource.id.as_str() {
                    "ATPC0205" => {
                        return Ok(Ok(Some(TransferProcess {
                            provider_pid: process.provider_pid,
                            consumer_pid: process.consumer_pid,
                            state: TransferState::Terminated(TerminatedData {
                                sent: false,
                                payload: TerminatedDataPayload {
                                    code: Some("999".into()),
                                    reason: Some(vec!["TCK ATPC0205".into()]),
                                },
                            }),
                        })));
                    }
                    _ => TransferState::Requested(s),
                }
            }
            TransferState::Started(s) if !s.is_pending() => {
                match agreement.policy_class.resource.id.as_str() {
                    "ATPC0201" => {
                        return Ok(Ok(Some(TransferProcess {
                            provider_pid: process.provider_pid,
                            consumer_pid: process.consumer_pid,
                            state: TransferState::Terminated(TerminatedData {
                                sent: false,
                                payload: TerminatedDataPayload {
                                    code: Some("999".into()),
                                    reason: Some(vec!["TCK ATPC0201".into()]),
                                },
                            }),
                        })));
                    }
                    "ATPC0202" => {
                        return Ok(Ok(Some(TransferProcess {
                            provider_pid: process.provider_pid,
                            consumer_pid: process.consumer_pid,
                            state: TransferState::Completed(Default::default()),
                        })));
                    }
                    "ATPC0203" => {
                        return Ok(Ok(Some(TransferProcess {
                            provider_pid: process.provider_pid,
                            consumer_pid: process.consumer_pid,
                            state: TransferState::Suspended(SuspendedData {
                                sent: false,
                                payload: SuspendedDataPayload {
                                    count: s.payload.count,
                                    code: Some("999".into()),
                                    reason: Some(vec!["TCK ATPC0203".into()]),
                                },
                            }),
                        })));
                    }
                    "ATPC0204" if s.payload.count == 1 => {
                        return Ok(Ok(Some(TransferProcess {
                            provider_pid: process.provider_pid,
                            consumer_pid: process.consumer_pid,
                            state: TransferState::Suspended(SuspendedData {
                                sent: false,
                                payload: SuspendedDataPayload {
                                    count: s.payload.count,
                                    code: Some("999".into()),
                                    reason: Some(vec!["TCK ATPC0204".into()]),
                                },
                            }),
                        })));
                    }
                    "ATPC0204" if s.payload.count > 1 => {
                        return Ok(Ok(Some(TransferProcess {
                            provider_pid: process.provider_pid,
                            consumer_pid: process.consumer_pid,
                            state: TransferState::Completed(CompletedData {
                                sent: false,
                                payload: (),
                            }),
                        })));
                    }
                    _ => TransferState::Started(s),
                }
            }
            TransferState::Suspended(s) if !s.is_pending() => {
                match agreement.policy_class.resource.id.as_str() {
                    "ATPC0203" => {
                        return Ok(Ok(Some(TransferProcess {
                            provider_pid: process.provider_pid,
                            consumer_pid: process.consumer_pid,
                            state: TransferState::Terminated(TerminatedData {
                                sent: false,
                                payload: TerminatedDataPayload {
                                    code: Some("999".into()),
                                    reason: Some(vec!["TCK ATPC0203".into()]),
                                },
                            }),
                        })));
                    }
                    "ATPC0204" => {
                        return Ok(Ok(Some(TransferProcess {
                            provider_pid: process.provider_pid,
                            consumer_pid: process.consumer_pid,
                            state: TransferState::Started(StartedData {
                                sent: false,
                                payload: StartedDataPayload {
                                    count: s.payload.count + 1,
                                },
                            }),
                        })));
                    }
                    _ => TransferState::Suspended(s),
                }
            }
            s => s,
        };
        Err(Process {
            provider_pid: process.provider_pid,
            consumer_pid: process.consumer_pid,
            state,
        })
    }
}
