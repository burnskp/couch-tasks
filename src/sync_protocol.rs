use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncRequest {
    pub checkpoint: i64,
    pub projects: Vec<SyncProject>,
    pub tasks: Vec<SyncTask>,
    pub tombstones: Vec<SyncTombstone>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncResponse {
    pub checkpoint: i64,
    pub projects: Vec<SyncProject>,
    pub tasks: Vec<SyncTask>,
    pub tombstones: Vec<SyncTombstone>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncProject {
    pub uuid: String,
    pub name: String,
    pub name_normalized: String,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
    pub sync_revision: i64,
    pub sync_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncTask {
    pub uuid: String,
    pub project_uuid: String,
    pub sort_order: i64,
    pub title: String,
    pub notes: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    pub sync_revision: i64,
    pub sync_dirty: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncEntityKind {
    Project,
    Task,
}

impl SyncEntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Task => "task",
        }
    }
}

impl TryFrom<&str> for SyncEntityKind {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "project" => Ok(Self::Project),
            "task" => Ok(Self::Task),
            other => Err(format!("unknown sync entity kind: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncTombstone {
    pub entity_type: SyncEntityKind,
    pub uuid: String,
    pub deleted_at: String,
    pub sync_revision: i64,
    pub sync_dirty: bool,
}
