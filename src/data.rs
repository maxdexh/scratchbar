use std::sync::Arc;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct WorkspaceId(Arc<str>);
impl From<&str> for WorkspaceId {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}
impl From<String> for WorkspaceId {
    fn from(value: String) -> Self {
        Self(value.into())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BasicWorkspace {
    pub id: WorkspaceId,
    pub name: Arc<str>,
    pub monitor: Option<Arc<str>>,
    pub is_active: bool,
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct BasicDesktopState {
    pub workspaces: Vec<BasicWorkspace>,
}
