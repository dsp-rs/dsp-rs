use std::{collections::HashMap, path::PathBuf, sync::Arc};

use anyhow::Context;
use axum::Json;
use axum::{
    extract::{FromRequest, Request},
    http::StatusCode,
    response::IntoResponse,
};
use jsonschema::{Retrieve, Uri, Validator};
use reqwest::Response;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::fs;
use tracing::debug;

use crate::{connector::AppState, store::Store};

struct LocalRetriever(Arc<HashMap<String, Value>>);

impl Retrieve for LocalRetriever {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        self.0
            .get(uri.as_str())
            .cloned()
            .ok_or_else(|| format!("Schema not found: {uri}").into())
    }
}

pub(crate) struct SchemaValidator {
    validators: HashMap<String, Validator>,
}

impl SchemaValidator {
    pub(crate) async fn new() -> anyhow::Result<Self> {
        let schemas = Arc::new(
            load_schemas("third_party/dsp-spec/artifacts/src/main/resources".into())
                .await
                .context("Failed to load schemas")?,
        );

        let mut validators = HashMap::new();
        for (uri, schema) in schemas.iter() {
            debug!("Inserting schema '{uri}' ...");
            let validator = jsonschema::options()
                .with_retriever(LocalRetriever(schemas.clone()))
                .build(schema)
                .map_err(|err| {
                    anyhow::anyhow!("Failed to create validator for schema {uri}, error: {err}")
                })?;
            validators.insert(uri.clone(), validator);
        }

        Ok(Self { validators })
    }

    pub(crate) fn validate<T: HasSchemaName>(&self, data: &Value) -> anyhow::Result<()> {
        let schema = T::NAME;
        if let Some(validator) = self.validators.get(schema) {
            validator
                .validate(data)
                .map_err(|err| anyhow::anyhow!("Validation failed, error: {err}"))
        } else {
            anyhow::bail!("Schema for URI '{schema}' not found")
        }
    }
}

async fn load_schemas(root: PathBuf) -> anyhow::Result<HashMap<String, Value>> {
    let mut schemas = HashMap::new();

    let mut stack = vec![root];
    while let Some(path) = stack.pop() {
        let mut entries = fs::read_dir(path).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            match path.file_name().and_then(|n| n.to_str()) {
                Some(name) if name == "example" => continue,
                Some(name) => {
                    if path.is_dir() {
                        stack.push(path);
                    } else if name.ends_with("-schema.json") {
                        let data = fs::read(path).await?;
                        let schema: Value = serde_json::from_slice(&data)?;
                        let id = schema.get("$id").cloned();
                        if let Some(Value::String(id)) = id {
                            schemas.insert(id, schema);
                        }
                    }
                }
                _ => continue,
            }
        }
    }
    Ok(schemas)
}

pub trait HasSchemaName {
    const NAME: &'static str;
}

impl HasSchemaName for () {
    const NAME: &'static str = "";
}

pub struct ValidatedJson<T>(pub T);

impl<T, S> FromRequest<AppState<S>> for ValidatedJson<T>
where
    T: DeserializeOwned + HasSchemaName + Send,
    S: Store,
{
    type Rejection = axum::response::Response;

    async fn from_request(req: Request, state: &AppState<S>) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
            .await
            .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()).into_response())?;

        let json_value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()).into_response())?;

        let schema_name = T::NAME;

        state.validator.validate::<T>(&json_value).map_err(|err| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "error": format!("Schema '{}' validation failed", schema_name),
                    "details": err.to_string()
                })),
            )
                .into_response()
        })?;

        let target_struct = serde_json::from_value::<T>(json_value)
            .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()).into_response())?;

        Ok(ValidatedJson(target_struct))
    }
}

#[async_trait::async_trait]
pub trait ValidatedResponseExt {
    async fn validated_json<T>(self, validator: &SchemaValidator) -> Result<T, anyhow::Error>
    where
        T: DeserializeOwned + HasSchemaName;
}

#[async_trait::async_trait]
impl ValidatedResponseExt for Response {
    async fn validated_json<T>(mut self, validator: &SchemaValidator) -> Result<T, anyhow::Error>
    where
        T: DeserializeOwned + HasSchemaName,
    {
        let json_value: Value = self.json().await.map_err(|e| anyhow::anyhow!(e))?;

        validator
            .validate::<T>(&json_value)
            .map_err(|e| anyhow::anyhow!("Validation failed for {}: {}", T::NAME, e))?;

        let target = serde_json::from_value(json_value).map_err(|e| anyhow::anyhow!(e))?;
        Ok(target)
    }
}
