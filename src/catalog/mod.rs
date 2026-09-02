use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use tracing::debug;

use crate::{
    AppError,
    auth::extractor::AuthClaims,
    connector::app_state::{AppState, AppStateCatalog},
    model::{
        catalog::{CatalogError, CatalogRequest, RootCatalog},
        dataset::{AccessService, RootDataset},
    },
    store::Store,
};

pub(crate) mod sync;

pub(crate) fn router<T: Store>() -> Router<AppState<T>> {
    Router::new()
        .route("/request", post(catalog_request))
        .route("/datasets/{id}", get(dataset_request))
}

pub(crate) async fn catalog_request<T: Store>(
    claims: AuthClaims,
    State(state): State<AppStateCatalog<T>>,
    Json(request): Json<CatalogRequest>,
) -> Result<impl IntoResponse, AppError> {
    debug!("Retrieving catalog with claims: {:?}", claims);

    let datasets = state.store.get_datasets(request.filter).await?;

    // NOTE: probably datasets should be stored in a simpler schema. The distribution will be
    // generated automatically. We could also define a single access service, which will be shared
    // by all datasets served by this connector.

    let datasets: Vec<RootDataset> = datasets
        .into_iter()
        .map(|mut dataset| {
            dataset.dataset.distribution.iter_mut().for_each(|dist| {
                match &mut dist.access_service {
                    AccessService::Reference(_) => {
                        unimplemented!(
                            "Referencing access services in distribution is not supported"
                        )
                    }
                    AccessService::Concrete(data_service) => {
                        data_service.endpoint_url = state.participant_info.external_address.clone();
                    }
                }
            });
            dataset
        })
        .collect();

    let datasets = (!datasets.is_empty()).then_some(datasets);

    // FIXME: this id should be stable
    let id = format!("urn:uuid:{}", uuid::Uuid::new_v4());
    let catalog = RootCatalog::new(id, state.participant_info.id.clone(), datasets);
    Ok(Json(catalog))
}

pub(crate) async fn dataset_request<T: Store>(
    claims: AuthClaims,
    State(state): State<AppStateCatalog<T>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    debug!("Retrieving dataset {id} with claims: {:?}", claims);

    if let Some(dataset) = state.store.get_dataset(&id).await? {
        return Ok(Json(dataset));
    }

    Err(AppError::Catalog(CatalogError::new(
        StatusCode::NOT_FOUND,
        "dataset not found".to_owned(),
    )))
}
