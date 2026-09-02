use serde::{Deserialize, Serialize};

use crate::model::{
    common::{JsonLDContext, JsonLDType, Resource},
    policy::Offer,
};

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RootDataset {
    #[serde(flatten)]
    pub(crate) context: JsonLDContext,

    #[serde(flatten)]
    pub(crate) dataset: Dataset,
}

#[cfg(feature = "tck")]
impl RootDataset {
    pub(crate) fn new_tck(id: &str, offer_id: &str) -> Self {
        Self {
            context: Default::default(),
            dataset: Dataset {
                r#type: "Dataset".into(),
                resource: id.into(),
                distribution: vec![Distribution::new_tck()],
                has_policy: vec![Offer::new_tck(offer_id.into())],
            },
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Dataset {
    #[serde(flatten)]
    r#type: JsonLDType,

    #[serde(flatten)]
    pub(crate) resource: Resource,

    pub(crate) distribution: Vec<Distribution>,
    pub(crate) has_policy: Vec<Offer>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Distribution {
    #[serde(flatten)]
    r#type: JsonLDType,

    pub(crate) access_service: AccessService,
    format: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    has_policy: Option<Vec<Offer>>,
}

#[cfg(feature = "tck")]
impl Distribution {
    pub(crate) fn new_tck() -> Self {
        Self {
            r#type: "Distribution".into(),
            access_service: AccessService::Concrete(DataService {
                r#type: "DataService".into(),
                resource: format!("urn:uuid:{}", uuid::Uuid::new_v4().to_string()).into(),
                endpoint_url: "https://provider-a.com/connector".into(),
                serves_dataset: None,
            }),
            format: "HttpData-PULL".into(),
            has_policy: None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub(crate) enum AccessService {
    Reference(String),
    Concrete(DataService),
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DataService {
    #[serde(flatten)]
    r#type: JsonLDType,

    #[serde(flatten)]
    resource: Resource,

    #[serde(rename = "endpointURL")]
    pub(crate) endpoint_url: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    serves_dataset: Option<Vec<Dataset>>,
}

#[cfg(test)]
mod tests {
    use crate::assert_json_roundtrip;

    use super::*;

    #[test]
    fn test_dataset() {
        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/catalog/example/dataset.json",
            RootDataset
        );
    }
}
