use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Provider {
    Copilot,
    Claude,
    Codex,
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Copilot => write!(f, "Copilot"),
            Self::Claude => write!(f, "Claude Code"),
            Self::Codex => write!(f, "Codex CLI"),
        }
    }
}

impl Provider {
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Copilot => "Copilot",
            Self::Claude => "Claude",
            Self::Codex => "Codex",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl TokenUsage {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }

    pub fn merge(&mut self, other: &TokenUsage) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
    }
}

#[derive(Debug, Clone)]
pub enum PulseEvent {
    SessionStart {
        session_id: String,
        cwd: String,
        provider: Provider,
    },
    UserMessage {
        content: String,
    },
    AssistantMessage {
        content: String,
        tokens: TokenUsage,
        model: Option<String>,
    },
    ToolStart {
        name: String,
        arguments: serde_json::Value,
    },
    ToolComplete {
        name: String,
        success: bool,
        duration_ms: Option<u64>,
    },
    TurnStart {
        turn_id: String,
    },
    TurnEnd {
        turn_id: String,
    },
    Progress {
        message: String,
    },
    Warning {
        message: String,
    },
}

impl PulseEvent {
    pub fn label(&self) -> &'static str {
        match self {
            Self::SessionStart { .. } => "session.start",
            Self::UserMessage { .. } => "user.message",
            Self::AssistantMessage { .. } => "assistant.message",
            Self::ToolStart { .. } => "tool.start",
            Self::ToolComplete { .. } => "tool.complete",
            Self::TurnStart { .. } => "turn.start",
            Self::TurnEnd { .. } => "turn.end",
            Self::Progress { .. } => "progress",
            Self::Warning { .. } => "warning",
        }
    }

    pub fn summary(&self) -> String {
        match self {
            Self::SessionStart { cwd, provider, .. } => {
                format!("{provider} session in {cwd}")
            }
            Self::UserMessage { content } => truncate(content, 80),
            Self::AssistantMessage { content, .. } => truncate(content, 80),
            Self::ToolStart { name, arguments } => {
                let args_preview = arguments
                    .as_object()
                    .and_then(|o| {
                        o.get("command")
                            .or(o.get("file_path"))
                            .or(o.get("path"))
                            .or(o.get("intent"))
                    })
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if args_preview.is_empty() {
                    name.clone()
                } else {
                    format!("{name}: {}", truncate(args_preview, 60))
                }
            }
            Self::ToolComplete { name, success, .. } => {
                let status = if *success { "ok" } else { "FAIL" };
                format!("{name} [{status}]")
            }
            Self::TurnStart { turn_id } => format!("turn {turn_id}"),
            Self::TurnEnd { turn_id } => format!("turn {turn_id} done"),
            Self::Progress { message } => truncate(message, 80),
            Self::Warning { message } => truncate(message, 80),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TimestampedEvent {
    pub timestamp: DateTime<Utc>,
    pub event: PulseEvent,
}

fn truncate(s: &str, max: usize) -> String {
    let trimmed = s.trim().replace('\n', " ");
    if trimmed.chars().count() <= max {
        trimmed
    } else {
        let mut result: String = trimmed.chars().take(max - 1).collect();
        result.push('…');
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_usage_total() {
        let t = TokenUsage {
            input: 100,
            output: 50,
            cache_read: 200,
            cache_write: 30,
        };
        assert_eq!(t.total(), 380);
    }

    #[test]
    fn token_usage_merge() {
        let mut a = TokenUsage {
            input: 10,
            output: 5,
            cache_read: 0,
            cache_write: 0,
        };
        let b = TokenUsage {
            input: 20,
            output: 15,
            cache_read: 100,
            cache_write: 50,
        };
        a.merge(&b);
        assert_eq!(a.input, 30);
        assert_eq!(a.output, 20);
        assert_eq!(a.cache_read, 100);
        assert_eq!(a.cache_write, 50);
    }

    #[test]
    fn provider_display() {
        assert_eq!(format!("{}", Provider::Copilot), "Copilot");
        assert_eq!(format!("{}", Provider::Claude), "Claude Code");
        assert_eq!(format!("{}", Provider::Codex), "Codex CLI");
    }

    #[test]
    fn truncate_short_string() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string() {
        let long = "a".repeat(100);
        let result = truncate(&long, 20);
        assert_eq!(result.chars().count(), 20);
        assert!(result.ends_with('…'));
    }

    #[test]
    fn truncate_strips_newlines() {
        assert_eq!(truncate("hello\nworld", 50), "hello world");
    }

    #[test]
    fn event_label() {
        let e = PulseEvent::UserMessage {
            content: "test".into(),
        };
        assert_eq!(e.label(), "user.message");
    }

    #[test]
    fn event_summary_tool_start_with_command() {
        let e = PulseEvent::ToolStart {
            name: "bash".into(),
            arguments: serde_json::json!({"command": "cargo test"}),
        };
        assert_eq!(e.summary(), "bash: cargo test");
    }

    #[test]
    fn event_summary_tool_complete() {
        let e = PulseEvent::ToolComplete {
            name: "edit".into(),
            success: true,
            duration_ms: Some(150),
        };
        assert_eq!(e.summary(), "edit [ok]");
    }
}
