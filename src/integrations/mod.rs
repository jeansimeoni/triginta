use anyhow::{Result, bail};

// Traits are Rust's interface mechanism.
// Compared with C, this is closer to defining a vtable contract up front and
// letting multiple concrete implementations satisfy it.
pub trait TaskSyncProvider {
    fn provider_name(&self) -> &'static str;
    fn is_configured(&self) -> bool;
    fn sync(&self) -> Result<()>;
}

// A zero-sized struct can still implement behavior. This one acts as a
// placeholder strategy object: it has no fields because there is no real
// Todoist client state yet.
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
