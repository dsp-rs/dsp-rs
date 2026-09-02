#[cfg(test)]
mod tests {
    #[macro_export]
    macro_rules! assert_json_roundtrip {
        ($path:expr, $ty:ty) => {{
            let msg = std::fs::read($path)
                .unwrap_or_else(|e| panic!("Failed to read file at {}: {}", $path, e));

            // Deserialize into the target type and raw JSON Value
            let req = serde_json::from_slice::<$ty>(&msg)
                .unwrap_or_else(|e| panic!("Failed to deserialize into target type: {}", e));
            let req_json = serde_json::from_slice::<serde_json::Value>(&msg)
                .unwrap_or_else(|e| panic!("Failed to deserialize raw msg into Value: {}", e));

            // Serialize back to pretty string and parse into JSON Value for comparison
            let req_ser = serde_json::to_string_pretty(&req)
                .unwrap_or_else(|e| panic!("Failed to serialize target type: {}", e));
            let req_ser_json =
                serde_json::from_str::<serde_json::Value>(&req_ser).unwrap_or_else(|e| {
                    panic!("Failed to deserialize serialized string into Value: {}", e)
                });

            // Assert they match structurally
            assert_eq!(
                req_json, req_ser_json,
                "Roundtrip structural mismatch for {}",
                $path
            );
        }};
    }
}

pub(crate) mod catalog;
pub(crate) mod common;
pub(crate) mod contract;
pub(crate) mod dataset;
pub(crate) mod metadata;
pub(crate) mod policy;
pub(crate) mod transfer;
