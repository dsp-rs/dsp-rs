use std::time::Duration;

use tokio::{
    select,
    time::{MissedTickBehavior, interval},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::{
    connector::{
        app_state::{AppStateCatalog, RemoteConnector},
        get_request, post_request,
    },
    model::{
        catalog::{CatalogRequest, RootCatalog},
        metadata::{ProtocolVersion, VersionResponse},
    },
    store::Store,
};

pub(crate) async fn catalog_sync<T: Store>(token: CancellationToken, state: AppStateCatalog<T>) {
    for (name, connector) in state.federation.as_ref() {
        let token = token.clone();
        tokio::spawn(perform_sync(
            token,
            state.clone(),
            name.clone(),
            connector.clone(),
        ));
    }
}

async fn perform_sync<T: Store>(
    token: CancellationToken,
    state: AppStateCatalog<T>,
    name: String,
    connector: RemoteConnector,
) {
    let sync_interval_normal: Duration = Duration::from_secs(connector.sync_interval_secs);
    let sync_interval_escalated: Duration = Duration::from_secs(30);

    let mut ticker = interval(sync_interval_normal);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let mut version: Option<ProtocolVersion> = None;
    loop {
        select! {
            _ = token.cancelled() => {
                return
            }
            _ = ticker.tick() => {
                if do_sync(&state, &name, &connector, &mut version).await {
                    ticker.reset_after(sync_interval_normal);
                } else {
                    ticker.reset_after(sync_interval_escalated);
                }
            }
        }
    }
}

async fn do_sync<T: Store>(
    state: &AppStateCatalog<T>,
    name: &str,
    connector: &RemoteConnector,
    version: &mut Option<ProtocolVersion>,
) -> bool {
    if version.is_none() {
        version.replace(
            match select_version(&state, &connector.remote_address).await {
                Ok(Some(v)) => {
                    info!("Using version {} for connector {name}", v.version);
                    v
                }
                Ok(_) => {
                    warn!("No supported version for connector {name}");
                    return false;
                }
                Err(err) => {
                    error!("Failed to select version for connector {name}, error: {err}");
                    return false;
                }
            },
        );
    }
    let Some(version) = version.as_ref() else {
        return false;
    };

    let my_did_web = match state.participant_info.did_web() {
        Ok(did) => did,
        Err(err) => {
            error!("Failed to determine own did web, error: {err}");
            return false;
        }
    };
    let access_token = match state
        .authenticator
        .get_token(&state.client, &connector.remote_address, my_did_web)
        .await
    {
        Ok(t) => {
            debug!("Successfully retrieved access token for connector {name}");
            t
        }
        Err(err) => {
            error!("Failed to retrieve an access token for connector {name}, error: {err}");
            return false;
        }
    };

    match fetch_and_sync_catalog(
        &state,
        &name,
        &connector.remote_address,
        access_token.clone(),
        version,
    )
    .await
    {
        Ok(_) => true,
        Err(err) => {
            error!("Failed to perform catalog sync for connector {name}, error: {err}");
            false
        }
    }
}

async fn select_version<T: Store>(
    state: &AppStateCatalog<T>,
    remote_address: &str,
) -> anyhow::Result<Option<ProtocolVersion>> {
    let versions: VersionResponse = get_request(
        &state.client,
        &state.validator,
        &format!("{remote_address}/.well-known/dspace-version"),
        None,
    )
    .await?;

    // TODO: define our own authentication protocol and check whether the version supports it

    Ok(versions
        .protocol_versions
        .into_iter()
        .filter(|v| v.version == "2025-1" && v.binding == "HTTPS")
        .next())
}

async fn fetch_and_sync_catalog<T: Store>(
    state: &AppStateCatalog<T>,
    name: &str,
    remote_address: &str,
    access_token: String,
    version: &ProtocolVersion,
) -> anyhow::Result<()> {
    let request = CatalogRequest::new(None);
    let root_catalog = post_request(
        &state.client,
        &state.validator,
        &format!(
            "{remote_address}{path}/catalog/request",
            path = version.path
        ),
        Some(access_token),
        &request,
        |_, response: Option<RootCatalog>| response.unwrap(),
    )
    .await?;

    sync_catalog(state, name, root_catalog).await?;

    Ok(())
}

async fn sync_catalog<T: Store>(
    state: &AppStateCatalog<T>,
    name: &str,
    catalog: RootCatalog,
) -> anyhow::Result<()> {
    let datasets = catalog.extract_datasets();

    for dataset in &datasets {
        match state.store.save_federated_dataset(name, dataset).await {
            Ok(_) => info!(
                "Sucessfully stored federated dataset {} for connector {name}",
                dataset.dataset.resource.id
            ),
            Err(err) => error!(
                "Failed to store federated dataset {} for connector {name}, error: {err}",
                dataset.dataset.resource.id
            ),
        }
    }

    Ok(())
}
