use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::event::{Provider, PulseEvent, TimestampedEvent, TokenUsage};
use crate::metrics;

#[derive(Debug, Clone, Default)]
pub struct FileStats {
    pub reads: u32,
    pub writes: u32,
    pub last_op: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub provider: Provider,
    pub cwd: String,
    pub summary: String,
    pub model: Option<String>,
    pub started_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    pub is_active: bool,
    pub events: Vec<TimestampedEvent>,
    pub tokens: TokenUsage,
    pub tool_calls: HashMap<String, ToolStats>,
    pub files: HashMap<String, FileStats>,
    pub turn_count: u32,
    pub user_message_count: u32,
}

#[derive(Debug, Clone, Default)]
pub struct ToolStats {
    pub calls: u32,
    pub successes: u32,
    pub failures: u32,
    pub total_duration_ms: u64,
    pub last_used: Option<DateTime<Utc>>,
}

impl Session {
    pub fn new(id: String, provider: Provider, cwd: String, started_at: DateTime<Utc>) -> Self {
        Self {
            id,
            provider,
            cwd,
            summary: String::new(),
            model: None,
            started_at,
            last_activity: started_at,
            is_active: false,
            events: Vec::new(),
            tokens: TokenUsage::default(),
            tool_calls: HashMap::new(),
            files: HashMap::new(),
            turn_count: 0,
            user_message_count: 0,
        }
    }

    pub fn ingest(&mut self, ts_event: TimestampedEvent) {
        if ts_event.timestamp > self.last_activity {
            self.last_activity = ts_event.timestamp;
        }

        match &ts_event.event {
            PulseEvent::UserMessage { .. } => {
                self.user_message_count += 1;
            }
            PulseEvent::AssistantMessage { tokens, model, .. } => {
                self.tokens.merge(tokens);
                if self.model.is_none() {
                    self.model = model.clone();
                }
            }
            PulseEvent::ToolStart { name, arguments } => {
                let entry = self.tool_calls.entry(name.clone()).or_default();
                entry.calls += 1;
                entry.last_used = Some(ts_event.timestamp);

                if let Some(path) = extract_file_path(name, arguments) {
                    let file = self.files.entry(path).or_default();
                    if is_read_tool(name) {
                        file.reads += 1;
                        file.last_op = Some("read".into());
                    } else if is_write_tool(name) {
                        file.writes += 1;
                        file.last_op = Some("write".into());
                    }
                }
            }
            PulseEvent::ToolComplete {
                name,
                success,
                duration_ms,
            } => {
                let entry = self.tool_calls.entry(name.clone()).or_default();
                if *success {
                    entry.successes += 1;
                } else {
                    entry.failures += 1;
                }
                if let Some(ms) = duration_ms {
                    entry.total_duration_ms += ms;
                }
            }
            PulseEvent::TurnEnd { .. } => {
                self.turn_count += 1;
            }
            _ => {}
        }

        self.events.push(ts_event);
    }

    pub fn estimated_cost(&self) -> f64 {
        let model_name = self.model.as_deref().unwrap_or("");
        metrics::estimate_cost(model_name, &self.tokens)
    }

    pub fn total_tool_calls(&self) -> u32 {
        self.tool_calls.values().map(|s| s.calls).sum()
    }

    pub fn duration(&self) -> chrono::Duration {
        self.last_activity - self.started_at
    }

    pub fn age_label(&self) -> String {
        human_duration(Utc::now() - self.last_activity)
    }
}

fn extract_file_path(_tool_name: &str, args: &serde_json::Value) -> Option<String> {
    let obj = args.as_object()?;
    obj.get("file_path")
        .or(obj.get("path"))
        .and_then(|v| v.as_str())
        .map(shorten_path)
}

fn is_read_tool(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("read") || lower == "view" || lower == "glob" || lower == "grep"
}

fn is_write_tool(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("edit") || lower.contains("write") || lower.contains("create")
}

pub fn shorten_path(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Some(rest) = path.strip_prefix(home.to_str().unwrap_or("")) {
            return format!("~{rest}");
        }
    }
    path.to_string()
}

pub fn human_duration(d: chrono::Duration) -> String {
    let secs = d.num_seconds().max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        if m > 0 {
            format!("{h}h {m}m")
        } else {
            format!("{h}h")
        }
    } else {
        format!("{}d", secs / 86400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ingest_user_message() {
        let mut s = Session::new("s1".into(), Provider::Copilot, "/tmp".into(), Utc::now());
        s.ingest(TimestampedEvent {
            timestamp: Utc::now(),
            event: PulseEvent::UserMessage {
                content: "hello".into(),
            },
        });
        assert_eq!(s.user_message_count, 1);
        assert_eq!(s.events.len(), 1);
    }

    #[test]
    fn session_ingest_assistant_merges_tokens() {
        let mut s = Session::new("s1".into(), Provider::Claude, "/tmp".into(), Utc::now());
        s.ingest(TimestampedEvent {
            timestamp: Utc::now(),
            event: PulseEvent::AssistantMessage {
                content: "hi".into(),
                tokens: TokenUsage {
                    input: 100,
                    output: 50,
                    cache_read: 200,
                    cache_write: 0,
                },
                model: Some("claude-sonnet-4".into()),
            },
        });
        s.ingest(TimestampedEvent {
            timestamp: Utc::now(),
            event: PulseEvent::AssistantMessage {
                content: "more".into(),
                tokens: TokenUsage {
                    input: 50,
                    output: 25,
                    cache_read: 100,
                    cache_write: 10,
                },
                model: Some("claude-sonnet-4".into()),
            },
        });
        assert_eq!(s.tokens.input, 150);
        assert_eq!(s.tokens.output, 75);
        assert_eq!(s.tokens.cache_read, 300);
        assert_eq!(s.model.as_deref(), Some("claude-sonnet-4"));
    }

    #[test]
    fn session_tool_tracking() {
        let mut s = Session::new("s1".into(), Provider::Copilot, "/tmp".into(), Utc::now());
        s.ingest(TimestampedEvent {
            timestamp: Utc::now(),
            event: PulseEvent::ToolStart {
                name: "bash".into(),
                arguments: serde_json::json!({"command": "cargo test"}),
            },
        });
        s.ingest(TimestampedEvent {
            timestamp: Utc::now(),
            event: PulseEvent::ToolComplete {
                name: "bash".into(),
                success: true,
                duration_ms: Some(1500),
            },
        });
        assert_eq!(s.total_tool_calls(), 1);
        let bash = &s.tool_calls["bash"];
        assert_eq!(bash.calls, 1);
        assert_eq!(bash.successes, 1);
        assert_eq!(bash.total_duration_ms, 1500);
    }

    #[test]
    fn session_file_tracking() {
        let mut s = Session::new("s1".into(), Provider::Claude, "/tmp".into(), Utc::now());
        s.ingest(TimestampedEvent {
            timestamp: Utc::now(),
            event: PulseEvent::ToolStart {
                name: "Read".into(),
                arguments: serde_json::json!({"file_path": "/tmp/test.rs"}),
            },
        });
        s.ingest(TimestampedEvent {
            timestamp: Utc::now(),
            event: PulseEvent::ToolStart {
                name: "Edit".into(),
                arguments: serde_json::json!({"file_path": "/tmp/test.rs"}),
            },
        });
        let file = &s.files["/tmp/test.rs"];
        assert_eq!(file.reads, 1);
        assert_eq!(file.writes, 1);
    }

    #[test]
    fn human_duration_seconds() {
        assert_eq!(human_duration(chrono::Duration::seconds(30)), "30s");
    }

    #[test]
    fn human_duration_minutes() {
        assert_eq!(human_duration(chrono::Duration::seconds(150)), "2m");
    }

    #[test]
    fn human_duration_hours_and_minutes() {
        assert_eq!(human_duration(chrono::Duration::seconds(5400)), "1h 30m");
    }

    #[test]
    fn human_duration_days() {
        assert_eq!(human_duration(chrono::Duration::seconds(172800)), "2d");
    }

    #[test]
    fn is_read_tool_variants() {
        assert!(is_read_tool("Read"));
        assert!(is_read_tool("view"));
        assert!(is_read_tool("glob"));
        assert!(!is_read_tool("bash"));
    }

    #[test]
    fn is_write_tool_variants() {
        assert!(is_write_tool("Edit"));
        assert!(is_write_tool("create"));
        assert!(is_write_tool("TodoWrite"));
        assert!(!is_write_tool("bash"));
    }
}
