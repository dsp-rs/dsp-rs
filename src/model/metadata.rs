use serde::{Deserialize, Serialize};

use crate::connector::validator::HasSchemaName;

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VersionResponse {
    pub(crate) protocol_versions: Vec<ProtocolVersion>,
}

impl HasSchemaName for VersionResponse {
    const NAME: &'static str = "https://w3id.org/dspace/2025/1/common/protocol-version-schema.json";
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProtocolVersion {
    pub(crate) version: String,
    pub(crate) path: String,
    pub(crate) binding: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) identifier_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) service_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) auth: Option<Auth>,
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct Auth {
    pub(crate) protocol: String,
    pub(crate) version: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) profile: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use crate::assert_json_roundtrip;

    use super::*;

    #[test]
    fn test_versions_response() {
        assert_json_roundtrip!(
            "third_party/dsp-spec/artifacts/src/main/resources/common/example/protocol-version.json",
            VersionResponse
        );
    }
}
