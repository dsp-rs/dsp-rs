use std::{collections::HashMap, path::PathBuf, time::SystemTime};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::{
    model::{dataset::RootDataset, policy::MessageOffer},
    negotiation::Negotiation,
    store::{RemoteAddress, Store},
    transfer::Transfer,
};

#[cfg(not(feature = "tck"))]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct LocalDataset {
    pub(crate) dataset: RootDataset,
    pub(crate) remote_address: RemoteAddress,
}

pub(crate) struct FileStore {
    base_path: PathBuf,
}

impl FileStore {
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }

    fn datasets_path(&self) -> PathBuf {
        self.base_path.join("datasets")
    }

    fn federated_datasets_path(&self, name: &str) -> PathBuf {
        self.datasets_path().join(format!("federated/{name}"))
    }

    fn negotiations_path(&self) -> PathBuf {
        self.base_path.join("negotiations")
    }

    fn transfers_path(&self) -> PathBuf {
        self.base_path.join("transfers")
    }
}

#[async_trait]
impl Store for FileStore {
    #[cfg(feature = "tck")]
    async fn get_datasets(&self, _filter: Option<Vec<String>>) -> anyhow::Result<Vec<RootDataset>> {
        Ok(vec![
            RootDataset::new_tck("CAT0101", "CD123:CAT101:456"),
            RootDataset::new_tck("CAT0102", "CD123:CAT102:456"),
        ])
    }

    #[cfg(not(feature = "tck"))]
    async fn get_datasets(&self, _filter: Option<Vec<String>>) -> anyhow::Result<Vec<RootDataset>> {
        let path = self.datasets_path();
        if !path.exists() {
            return Ok(vec![]);
        }

        let mut datasets = vec![];
        let mut dir = fs::read_dir(path).await?;
        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path();
            let filename = entry.file_name();
            let filename_str = filename.to_string_lossy();
            if path.is_file() && filename_str.ends_with(".json") {
                let data = fs::read(entry.path()).await?;
                let dataset = serde_json::from_slice::<LocalDataset>(&data)?;
                datasets.push(dataset.dataset);
            }
        }
        Ok(datasets)
    }

    #[cfg(feature = "tck")]
    async fn get_dataset(&self, id: &str) -> anyhow::Result<Option<RootDataset>> {
        Ok(self
            .get_datasets(None)
            .await?
            .into_iter()
            .find(|d| d.dataset.resource.id == id))
    }

    #[cfg(not(feature = "tck"))]
    async fn get_dataset(&self, id: &str) -> anyhow::Result<Option<RootDataset>> {
        Ok(self.get_local_dataset(id).await?.map(|d| d.dataset))
    }

    #[cfg(feature = "tck")]
    async fn get_matching_dataset(
        &self,
        offer: &MessageOffer,
    ) -> anyhow::Result<Option<RootDataset>> {
        Ok(Some(RootDataset::new_tck(
            &offer.target,
            &offer.policy_class.resource.id,
        )))
    }

    #[cfg(not(feature = "tck"))]
    async fn get_matching_dataset(
        &self,
        offer: &MessageOffer,
    ) -> anyhow::Result<Option<RootDataset>> {
        self.get_dataset(&offer.target).await
    }

    #[cfg(feature = "tck")]
    async fn get_dataset_target(&self, _id: &str) -> anyhow::Result<Option<RemoteAddress>> {
        Ok(Some(RemoteAddress {
            url: "http://localhost:4000".into(),
            pass_headers: false,
        }))
    }

    #[cfg(not(feature = "tck"))]
    async fn get_dataset_target(&self, id: &str) -> anyhow::Result<Option<RemoteAddress>> {
        Ok(self.get_local_dataset(id).await?.map(|d| d.remote_address))
    }

    async fn get_federated_dataset(
        &self,
        name: &str,
        dataset_id: &str,
    ) -> anyhow::Result<Option<RootDataset>> {
        let path = self
            .federated_datasets_path(name)
            .join(format!("{}.json", dataset_id.replace(":", "-")));

        self.load(path).await
    }

    async fn save_federated_dataset(
        &self,
        name: &str,
        dataset: &RootDataset,
    ) -> anyhow::Result<()> {
        let path = self.federated_datasets_path(name);
        self.save(path, &dataset.dataset.resource.id, None, dataset)
            .await
    }

    async fn get_negotiation(&self, pid: &str) -> anyhow::Result<Option<Negotiation>> {
        self.get(self.negotiations_path(), pid).await
    }

    #[cfg(feature = "tck")]
    async fn get_finalized_negotiation(
        &self,
        agreement_id: &str,
    ) -> anyhow::Result<Option<Negotiation>> {
        use crate::negotiation;
        use crate::{model::policy::Agreement, negotiation::ContractNegotiation};

        let offer = MessageOffer::new_tck("same-dataset-id".into(), "some-offer-id".into());
        // FIXME: assigner and assignee empty
        let agreement = Agreement::new(agreement_id.into(), offer, "".into(), "".into());
        Ok(Some(Negotiation::Provider {
            contract: ContractNegotiation {
                provider_pid: "some-provider-pid".into(),
                consumer_pid: "some-consumer-pid".into(),
                state: negotiation::NegotiationState::Finalized(negotiation::FinalizedData {
                    agreement,
                }),
            },
            callback_address: "some-callback-address".into(),
        }))
    }

    #[cfg(not(feature = "tck"))]
    async fn get_finalized_negotiation(
        &self,
        agreement_id: &str,
    ) -> anyhow::Result<Option<Negotiation>> {
        Ok(self
            .get_with_state(self.negotiations_path(), 1, |state| {
                state == format!("finalized-{}", agreement_id)
            })
            .await?
            .into_iter()
            .next())
    }

    async fn save_negotiation(&self, negotiation: &Negotiation) -> anyhow::Result<()> {
        self.save(
            self.negotiations_path(),
            negotiation.pid(),
            Some(&negotiation.state_description()),
            negotiation,
        )
        .await
    }

    async fn get_pending_negotiations(&self, limit: usize) -> anyhow::Result<Vec<Negotiation>> {
        self.get_with_state(self.negotiations_path(), limit, |state| {
            // TODO: terminated negotiations get picked up!
            !state.starts_with("finalized") || state.ends_with("pending")
        })
        .await
    }

    async fn get_transfer(&self, pid: &str) -> anyhow::Result<Option<Transfer>> {
        self.get(self.transfers_path(), pid).await
    }

    async fn save_transfer(&self, transfer: &Transfer) -> anyhow::Result<()> {
        self.save(
            self.transfers_path(),
            transfer.pid(),
            Some(&transfer.state()),
            transfer,
        )
        .await
    }

    async fn get_pending_transfers(&self, limit: usize) -> anyhow::Result<Vec<Transfer>> {
        self.get_with_state(self.transfers_path(), limit, |state| {
            !state.starts_with("completed")
                || !state.starts_with("terminated")
                || state.ends_with("pending")
        })
        .await
    }
}

impl FileStore {
    #[cfg(not(feature = "tck"))]
    async fn get_local_dataset(&self, id: &str) -> anyhow::Result<Option<LocalDataset>> {
        let path = self
            .datasets_path()
            .join(format!("{}.json", id.replace(":", "-")));
        self.load(path).await
    }

    async fn load<T>(&self, path: PathBuf) -> anyhow::Result<Option<T>>
    where
        T: for<'de> Deserialize<'de>,
    {
        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&path).await?;
        let data: T = serde_json::from_str(&content)?;

        Ok(Some(data))
    }

    async fn get<T>(&self, path: PathBuf, id: &str) -> anyhow::Result<Option<T>>
    where
        T: for<'de> Deserialize<'de>,
    {
        if !path.exists() {
            return Ok(None);
        }

        let pid = id.replace(":", "-");

        let prefix = format!("{}_", pid);
        let mut entries = fs::read_dir(path).await?;

        let mut matched_files: Vec<(PathBuf, SystemTime)> = Vec::new();

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let filename = entry.file_name();
            let filename_str = filename.to_string_lossy();

            if filename_str.starts_with(&prefix) && filename_str.ends_with(".json") {
                let metadata = entry.metadata().await?;
                let modified = metadata.modified()?;
                matched_files.push((path, modified));
            }
        }

        if matched_files.is_empty() {
            return Ok(None);
        }

        matched_files.sort_by_key(|&(_, modified)| modified);

        let (newest_path, _) = matched_files.pop().expect("matched_files is not empty");

        let content = fs::read_to_string(&newest_path).await?;
        let data: T = serde_json::from_str(&content)?;

        for (stale_path, _) in matched_files {
            let _ = fs::remove_file(stale_path).await;
        }

        Ok(Some(data))
    }

    async fn save<T: Serialize>(
        &self,
        path: PathBuf,
        id: &str,
        state: Option<&str>,
        data: &T,
    ) -> anyhow::Result<()> {
        if !path.exists() {
            fs::create_dir_all(&path).await?;
        }

        let id = id.replace(":", "-");

        let target_filename = if let Some(state) = state {
            format!("{}_{}.json", id, state)
        } else {
            format!("{}.json", id)
        };
        let target_path = path.join(&target_filename);
        let temp_path = path.join(format!("{}.tmp", id));

        let content = serde_json::to_string_pretty(data)?;
        fs::write(&temp_path, content).await?;

        if let Err(err) = fs::rename(&temp_path, &target_path).await {
            let _ = fs::remove_file(&temp_path).await;
            return Err(anyhow::anyhow!("Atomic save failed, error: {err}"));
        }

        if state.is_some() {
            let prefix = format!("{}_", id);
            let mut entries = fs::read_dir(path).await?;
            while let Some(entry) = entries.next_entry().await? {
                let filename = entry.file_name();
                let filename_str = filename.to_string_lossy();

                if filename_str.starts_with(&prefix)
                    && filename_str.ends_with(".json")
                    && filename_str != target_filename
                {
                    let _ = fs::remove_file(entry.path()).await;
                    break;
                }
            }
        }

        Ok(())
    }

    async fn get_with_state<T, F>(
        &self,
        path: PathBuf,
        limit: usize,
        state_filter: F,
    ) -> anyhow::Result<Vec<T>>
    where
        T: for<'de> Deserialize<'de>,
        F: Fn(&str) -> bool,
    {
        let mut pending = Vec::new();

        if !path.exists() {
            return Ok(pending);
        }

        let mut latest_active_files: HashMap<String, (PathBuf, SystemTime)> = HashMap::new();
        let mut entries = fs::read_dir(path).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let filename = entry.file_name();
            let filename_str = filename.to_string_lossy();

            if path.is_file() && filename_str.ends_with(".json") {
                let parts: Vec<&str> = filename_str.split('_').collect();

                if parts.len() == 2 {
                    let pid = parts[0].to_string();
                    let state = parts[1].replace(".json", "");

                    if state_filter(&state) {
                        let metadata = entry.metadata().await?;
                        let modified = metadata.modified()?;

                        if let Some((_, existing_time)) = latest_active_files.get_mut(&pid) {
                            if modified > *existing_time {
                                *existing_time = modified;
                            }
                        } else {
                            latest_active_files.insert(pid, (path, modified));
                        }
                    }
                }
            }
        }

        for (_pid, (path, _)) in latest_active_files {
            let content = fs::read_to_string(&path).await?;
            let data: T = serde_json::from_str(&content)?;
            pending.push(data);

            if pending.len() >= limit {
                break;
            }
        }

        Ok(pending)
    }
}
