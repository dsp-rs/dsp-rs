use std::collections::HashMap;

use serde::{Deserialize, Serialize};

const CONTEXT: &str = "https://w3id.org/dspace/2025/1/context.jsonld";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub(crate) enum ContextEntry {
    Iri(String),
    Map(HashMap<String, ContextEntry>),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub(crate) enum Context {
    Single(ContextEntry),
    Array(Vec<ContextEntry>),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub(crate) struct JsonLDContext {
    #[serde(rename = "@context")]
    context: Context,
}

impl Default for JsonLDContext {
    fn default() -> Self {
        Self {
            context: Context::Array(vec![ContextEntry::Iri(CONTEXT.to_owned())]),
        }
    }
}

impl JsonLDContext {
    pub(crate) fn union(self, others: Vec<Self>) -> Self {
        let mut all_entries: Vec<ContextEntry> = match self.context {
            Context::Single(e) => vec![e],
            Context::Array(v) => v,
        };

        for other in others {
            let other_entries = match other.context {
                Context::Single(e) => vec![e],
                Context::Array(v) => v,
            };
            all_entries.extend(other_entries);
        }

        let mut unique_entries: Vec<ContextEntry> = Vec::new();
        for entry in all_entries {
            if !unique_entries.contains(&entry) {
                unique_entries.push(entry);
            }
        }

        JsonLDContext {
            context: Context::Array(unique_entries),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub(crate) struct JsonLDType {
    #[serde(rename = "@type")]
    r#type: String,
}

impl From<String> for JsonLDType {
    fn from(value: String) -> Self {
        Self { r#type: value }
    }
}

impl From<&str> for JsonLDType {
    fn from(value: &str) -> Self {
        value.to_string().into()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Resource {
    #[serde(rename = "@id")]
    pub(crate) id: String,
}

impl From<String> for Resource {
    fn from(value: String) -> Self {
        Self { id: value }
    }
}

impl From<&str> for Resource {
    fn from(value: &str) -> Self {
        value.to_string().into()
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;

    use crate::model::common::{Context, ContextEntry, JsonLDContext};

    #[test]
    fn test_context_union() {
        let context = JsonLDContext::default();
        let context = context.union(vec![JsonLDContext::default(), JsonLDContext::default()]);

        if let Context::Array(items) = context.context {
            assert_eq!(
                items.len(),
                1,
                "The number of context items should be 1 after deduplication"
            );
        } else {
            panic!("Expected Context::Array, but got a different variant");
        }
    }

    #[test]
    fn test_deeply_nested_deduplication() {
        // Context 1: { "a": { "b": "c" } }
        let mut inner_map = HashMap::new();
        inner_map.insert("b".to_string(), ContextEntry::Iri("c".to_string()));

        let mut outer_map = HashMap::new();
        outer_map.insert("a".to_string(), ContextEntry::Map(inner_map));

        let ctx1 = JsonLDContext {
            context: Context::Single(ContextEntry::Map(outer_map)),
        };

        // Context 2: Exact same nested structure
        let ctx2 = ctx1.clone();

        let unioned = ctx1.union(vec![ctx2]);

        if let Context::Array(items) = unioned.context {
            assert_eq!(
                items.len(),
                1,
                "Should have deduplicated identical nested maps"
            );
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_key_collision_different_values() {
        // Context 1: { "name": "schema.org/name" }
        let ctx1 = JsonLDContext {
            context: Context::Single(ContextEntry::Map(
                [(
                    "name".to_string(),
                    ContextEntry::Iri("schema.org/name".to_string()),
                )]
                .into_iter()
                .collect(),
            )),
        };

        // Context 2: { "name": "custom.org/name" }
        let ctx2 = JsonLDContext {
            context: Context::Single(ContextEntry::Map(
                [(
                    "name".to_string(),
                    ContextEntry::Iri("custom.org/name".to_string()),
                )]
                .into_iter()
                .collect(),
            )),
        };

        let unioned = ctx1.union(vec![ctx2]);

        if let Context::Array(items) = unioned.context {
            assert_eq!(
                items.len(),
                2,
                "Should have kept both because values differ"
            );
        } else {
            panic!("Expected Array");
        }
    }
}
