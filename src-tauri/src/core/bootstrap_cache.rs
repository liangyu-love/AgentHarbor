use super::models::{CoreError, CoreSnapshotPayload, McpServerListPayload, SkillListPayload};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BootstrapStatePayload {
    pub written_at: Option<i64>,
    pub snapshot_progressive: Option<CoreSnapshotPayload>,
    pub mcp_servers: Option<McpServerListPayload>,
    pub installed_skills: Option<SkillListPayload>,
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}

pub fn load(path: &Path) -> BootstrapStatePayload {
    std::fs::read(path)
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

pub fn update(
    path: &Path,
    mut apply: impl FnMut(&mut BootstrapStatePayload),
) -> Result<(), CoreError> {
    let mut cache = load(path);
    apply(&mut cache);
    super::storage::write_atomic(path, &serde_json::to_vec_pretty(&cache)?)
}
