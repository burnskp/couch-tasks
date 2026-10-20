use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Project {
    pub id: i64,
    pub uuid: String,
    pub name: String,
    pub name_normalized: String,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Task {
    pub id: i64,
    pub uuid: String,
    pub project_id: i64,
    pub project_name: String,
    pub sort_order: i64,
    pub title: String,
    pub notes: String,
    pub status: TaskStatus,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending,
    #[serde(rename = "completed")]
    Done,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Done => "done",
        }
    }
}

impl TryFrom<&str> for TaskStatus {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "pending" => Ok(Self::Pending),
            "done" => Ok(Self::Done),
            other => Err(format!("unknown task status: {other}")),
        }
    }
}

pub fn normalize_project_name(name: &str) -> String {
    name.trim().to_lowercase()
}
