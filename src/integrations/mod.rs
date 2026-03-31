use anyhow::{Result, bail};

pub trait TaskSyncProvider {
    fn provider_name(&self) -> &'static str;
    fn is_configured(&self) -> bool;
    fn sync(&self) -> Result<()>;
}

#[derive(Debug, Clone, Default)]
pub struct DisabledTodoistProvider;

impl TaskSyncProvider for DisabledTodoistProvider {
    fn provider_name(&self) -> &'static str {
        "todoist"
    }

    fn is_configured(&self) -> bool {
        false
    }

    fn sync(&self) -> Result<()> {
        bail!("Todoist integration is not configured yet")
    }
}
