use async_trait::async_trait;
use serde::Deserialize;

use crate::{
    model::{dataset::RootDataset, policy::MessageOffer},
    negotiation::Negotiation,
    transfer::Transfer,
};

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteAddress {
    pub(crate) url: String,
    pub(crate) pass_headers: bool,
}

#[async_trait]
pub(crate) trait Store: Send + Sync + 'static {
    async fn get_datasets(&self, filter: Option<Vec<String>>) -> anyhow::Result<Vec<RootDataset>>;

    async fn get_dataset(&self, id: &str) -> anyhow::Result<Option<RootDataset>>;
    async fn get_matching_dataset(
        &self,
        offer: &MessageOffer,
    ) -> anyhow::Result<Option<RootDataset>>;
    async fn get_dataset_target(&self, id: &str) -> anyhow::Result<Option<RemoteAddress>>;

    async fn get_federated_dataset(
        &self,
        name: &str,
        dataset_id: &str,
    ) -> anyhow::Result<Option<RootDataset>>;
    async fn save_federated_dataset(&self, name: &str, dataset: &RootDataset)
    -> anyhow::Result<()>;

    async fn get_negotiation(&self, pid: &str) -> anyhow::Result<Option<Negotiation>>;
    async fn get_finalized_negotiation(
        &self,
        agreement_id: &str,
    ) -> anyhow::Result<Option<Negotiation>>;
    async fn save_negotiation(&self, negotiation: &Negotiation) -> anyhow::Result<()>;
    async fn get_pending_negotiations(&self, limit: usize) -> anyhow::Result<Vec<Negotiation>>;

    async fn get_transfer(&self, pid: &str) -> anyhow::Result<Option<Transfer>>;
    async fn save_transfer(&self, transfer: &Transfer) -> anyhow::Result<()>;
    async fn get_pending_transfers(&self, limit: usize) -> anyhow::Result<Vec<Transfer>>;
}

pub(crate) mod file_store;
