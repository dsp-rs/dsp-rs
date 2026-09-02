use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    connector::validator::HasSchemaName,
    model::{
        common::{JsonLDContext, JsonLDType, Resource},
        dataset::{DataService, Dataset, Distribution, RootDataset},
    },
};

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RootCatalog {
    #[serde(flatten)]
    context: JsonLDContext,

    #[serde(flatten)]
    pub(crate) catalog: Catalog,

    pub(crate) participant_id: String,
}

impl HasSchemaName for RootCatalog {
    const NAME: &'static str = "https://w3id.org/dspace/2025/1/catalog/catalog-schema.json";
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Catalog {
    #[serde(flatten)]
    r#type: JsonLDType,

    #[serde(flatten)]
    pub(crate) resource: Resource,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) distribution: Option<Vec<Distribution>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) catalog: Option<Vec<Catalog>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) dataset: Option<Vec<Dataset>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) service: Option<Vec<DataService>>,
}

impl RootCatalog {
    pub(crate) fn new(
        id: String,
        participant_id: String,
        datasets: Option<Vec<RootDataset>>,
    ) -> Self {
        let mut contexts = vec![];
        let datasets = datasets.map(|ds| {
            ds.into_iter()
                .map(|d| {
                    contexts.push(d.context);
                    d.dataset
                })
                .collect()
        });
        let context = JsonLDContext::default();
        let context = context.union(contexts);

        Self {
            context,
            participant_id,
            catalog: Catalog {
                r#type: "Catalog".into(),
                resource: id.into(),
                catalog: None,
                dataset: datasets,
                distribution: None,
                service: None,
            },
        }
    }

    pub(crate) fn extract_datasets(self) -> Vec<RootDataset> {
        RootCatalog::extract_datasets_from_cat(self.context, self.catalog)
    }

    fn extract_datasets_from_cat(context: JsonLDContext, catalog: Catalog) -> Vec<RootDataset> {
        // TODO: resolve distribition, access service, etc.
        let mut datasets: Vec<RootDataset> = catalog
            .dataset
            .into_iter()
            .flatten()
            .map(|d| RootDataset {
                context: context.clone(),
                dataset: d,
            })
            .collect();

        catalog.catalog.into_iter().flatten().for_each(|cat| {
            datasets.extend(RootCatalog::extract_datasets_from_cat(context.clone(), cat));
        });

        datasets
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct CatalogRequest {
    #[serde(flatten)]
    context: JsonLDContext,

    #[serde(flatten)]
    r#type: JsonLDType,

    pub(crate) filter: Option<Vec<String>>,
}

impl CatalogRequest {
    pub(crate) fn new(filter: Option<Vec<String>>) -> Self {
        Self {
            context: Default::default(),
            r#type: "CatalogRequest".into(),
            filter,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Error)]
#[error("Catalog error, code: {code:?}, reason: {reason:?}")]
pub(crate) struct CatalogError {
    #[serde(flatten)]
    context: JsonLDContext,

    #[serde(flatten)]
    r#type: JsonLDType,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<Vec<String>>,
}

impl CatalogError {
    pub(crate) fn new(code: StatusCode, reason: String) -> Self {
        Self {
            context: Default::default(),
            r#type: "CatalogError".into(),
            code: Some(format!("{code}", code = code.as_u16())),
            reason: Some(vec![reason]),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::assert_json_roundtrip;

    use super::*;

    #[test]
    fn test_catalog_request() {
        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/catalog/example/catalog-request-message.json",
            CatalogRequest
        );
    }

    #[test]
    fn test_catalog_response() {
        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/catalog/example/catalog.json",
            RootCatalog
        );

        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/catalog/example/nested-catalog.json",
            RootCatalog
        );
    }
}
