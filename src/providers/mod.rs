pub mod claude;
pub mod codex;
pub mod copilot;

use std::path::PathBuf;

use crate::event::{Provider, TimestampedEvent};

pub struct DiscoveredSession {
    pub session_id: String,
    pub provider: Provider,
    pub path: PathBuf,
    pub cwd: String,
    pub summary: String,
    pub is_active: bool,
}

pub trait SessionProvider {
    fn provider_type(&self) -> Provider;
    fn base_dirs(&self) -> Vec<PathBuf>;
    fn discover_sessions(&self) -> Vec<DiscoveredSession>;
    fn parse_events(
        &self,
        path: &std::path::Path,
        offset: u64,
    ) -> anyhow::Result<(Vec<TimestampedEvent>, u64)>;
}
