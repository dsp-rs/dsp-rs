use std::collections::HashMap;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::common::{JsonLDType, Resource};

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Policy {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) profile: Option<Profile>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) permission: Option<Vec<Rule>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) prohibition: Option<Vec<Rule>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) obligation: Option<Vec<Rule>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PolicyClass {
    #[serde(flatten)]
    r#type: JsonLDType,

    #[serde(flatten)]
    pub(crate) resource: Resource,

    #[serde(flatten)]
    pub(crate) policy: Policy,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Offer {
    #[serde(flatten)]
    pub(crate) policy_class: PolicyClass,
}

#[cfg(feature = "tck")]
impl Offer {
    pub(crate) fn new_tck(id: String) -> Self {
        Self {
            policy_class: PolicyClass {
                r#type: "Offer".into(),
                resource: id.into(),
                policy: Policy {
                    profile: None,
                    permission: Some(vec![Rule {
                        action: "use".into(),
                        constraint: Some(vec![Constraint::Atomic(AtomicConstraint {
                            left_operand: "spatial".into(),
                            operator: Operator::Eq,
                            right_operand: RightOperand::String("_:EU".into()),
                        })]),
                    }]),
                    prohibition: None,
                    obligation: None,
                },
            },
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub(crate) enum Profile {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rule {
    pub(crate) action: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) constraint: Option<Vec<Constraint>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub(crate) enum Constraint {
    Logical(LogicalConstraint),
    Atomic(AtomicConstraint),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub(crate) enum LogicalConstraint {
    And {
        and: Vec<Constraint>,
    },
    AndSequence {
        #[serde(rename = "andSequence")]
        and_sequence: Vec<Constraint>,
    },
    Or {
        or: Vec<Constraint>,
    },
    Xone {
        xone: Vec<Constraint>,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AtomicConstraint {
    pub(crate) left_operand: String,
    pub(crate) operator: Operator,
    pub(crate) right_operand: RightOperand,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub(crate) enum RightOperand {
    String(String),
    Object(HashMap<String, Value>),
    Array(Vec<Value>),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Operator {
    Eq,
    Gt,
    Gteq,
    Lteq,
    HasPart,
    IsA,
    IsAllOf,
    IsAnyOf,
    IsNoneOf,
    IsPartOf,
    Lt,
    #[serde(rename = "term-lteq")]
    TermLteq,
    Neq,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MessageOffer {
    #[serde(flatten)]
    pub(crate) policy_class: PolicyClass,

    pub(crate) target: String,
}

impl MessageOffer {
    #[cfg(feature = "tck")]
    pub(crate) fn new_tck(dataset_id: String, offer_id: String) -> Self {
        Self {
            policy_class: PolicyClass {
                r#type: "Offer".into(),
                resource: offer_id.into(),
                policy: Policy {
                    profile: None,
                    permission: Some(vec![Rule {
                        action: "use".to_owned(),
                        constraint: Some(vec![]),
                    }]),
                    prohibition: None,
                    obligation: None,
                },
            },
            target: dataset_id,
        }
    }

    pub(crate) fn new(dataset_id: String, policy: Policy) -> Self {
        let offer_id = format!("urn:uuid:{}", uuid::Uuid::new_v4().to_string());
        Self {
            policy_class: PolicyClass {
                r#type: "Offer".into(),
                resource: offer_id.into(),
                policy,
            },
            target: dataset_id,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Agreement {
    #[serde(flatten)]
    pub(crate) policy_class: PolicyClass,

    pub(crate) target: String,
    pub(crate) assigner: String,
    pub(crate) assignee: String,

    pub(crate) timestamp: String, // FIXME: the schema defines timestamp as optional
}

impl Agreement {
    pub(crate) fn new(id: String, offer: MessageOffer, assigner: String, assignee: String) -> Self {
        let mut policy_class = offer.policy_class;
        policy_class.r#type = "Agreement".into();
        policy_class.resource.id = id;
        let timestamp = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

        Self {
            policy_class,
            target: offer.target.clone(),
            assigner,
            assignee,
            timestamp,
        }
    }

    #[cfg(feature = "tck")]
    pub(crate) fn new_tck(agreement_id: String, assigner: String, assignee: String) -> Self {
        let offer = MessageOffer::new_tck("some-dataset-id".into(), "some-offer-id".into());
        Agreement::new(agreement_id, offer, assigner, assignee)
    }
}
