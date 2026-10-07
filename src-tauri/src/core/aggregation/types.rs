use crate::core::{model_policy, models_dev::ModelRef, protocol::ModelProtocol};
use serde::{Deserialize, Serialize};

pub const MAX_AGGREGATES: usize = 128;
pub const MAX_MEMBERS: usize = 64;
pub const MAX_MEMBER_REFERENCES: usize = 4096;
pub const MAX_PRIORITY: u16 = 1000;
pub const MAX_WEIGHT: u16 = 1000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Strategy {
    #[default]
    RoundRobin,
    WeightedRoundRobin,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Affinity {
    #[default]
    Session,
    StatelessRotation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RetryPolicy {
    pub max_attempts: u8,
    pub failure_threshold: u8,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 2,
            failure_threshold: 3,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AggregateMember {
    pub id: String,
    pub provider_id: String,
    pub actual_model_id: String,
    pub enabled: bool,
    pub priority: u16,
    pub weight: u16,
}

impl Default for AggregateMember {
    fn default() -> Self {
        Self {
            id: String::new(),
            provider_id: String::new(),
            actual_model_id: String::new(),
            enabled: true,
            priority: 0,
            weight: 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AggregateModel {
    pub id: String,
    pub public_model_id: String,
    pub canonical_ref: Option<ModelRef>,
    pub display_name: String,
    pub enabled: bool,
    pub members: Vec<AggregateMember>,
    pub strategy: Strategy,
    pub affinity: Affinity,
    pub retry_policy: RetryPolicy,
    pub compression: model_policy::CompressionPolicy,
}

impl Default for AggregateModel {
    fn default() -> Self {
        Self {
            id: String::new(),
            public_model_id: String::new(),
            canonical_ref: None,
            display_name: String::new(),
            enabled: true,
            members: vec![],
            strategy: Strategy::default(),
            affinity: Affinity::default(),
            retry_policy: RetryPolicy::default(),
            compression: model_policy::CompressionPolicy::Inherit,
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberView {
    pub member_id: String,
    pub provider_name: String,
    pub protocol: Option<ModelProtocol>,
    pub available: bool,
    pub reason: Option<String>,
    pub projection: Option<model_policy::ClientProjection>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AggregateView {
    #[serde(flatten)]
    pub model: AggregateModel,
    pub available: bool,
    pub reason: Option<String>,
    pub projection: Option<model_policy::ClientProjection>,
    pub member_states: Vec<MemberView>,
    pub route_revision: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AggregationState {
    pub models: Vec<AggregateView>,
    pub external_enabled: bool,
    pub listener_ready: bool,
}
