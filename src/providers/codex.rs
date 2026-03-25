use std::fs;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::event::{Provider, PulseEvent, TimestampedEvent, TokenUsage};
use crate::providers::{DiscoveredSession, SessionProvider};

const CODEX_PREFIX: &str = "## My request for Codex:";

pub struct CodexProvider;

impl SessionProvider for CodexProvider {
    fn provider_type(&self) -> Provider {
        Provider::Codex
    }

    fn base_dirs(&self) -> Vec<PathBuf> {
        dirs::home_dir()
            .map(|h| {
                vec![
                    h.join(".codex").join("sessions"),
                    h.join(".codex").join("archived_sessions"),
                ]
            })
            .unwrap_or_default()
    }

    fn discover_sessions(&self) -> Vec<DiscoveredSession> {
        let mut sessions = Vec::new();
        for base in self.base_dirs() {
            collect_rollout_files(&base, &mut sessions);
        }
        sessions
    }

    fn parse_events(
        &self,
        path: &std::path::Path,
        offset: u64,
    ) -> Result<(Vec<TimestampedEvent>, u64)> {
        let file = fs::File::open(path)?;
        let mut reader = BufReader::new(file);
        reader.seek(SeekFrom::Start(offset))?;

        let mut events = Vec::new();
        let mut current_offset = offset;

        let mut line = String::new();
        loop {
            line.clear();
            let bytes_read = reader.read_line(&mut line)?;
            if bytes_read == 0 {
                break;
            }
            current_offset += bytes_read as u64;

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let v: Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let event_type = match v.get("type").and_then(|t| t.as_str()) {
                Some(t) => t,
                None => continue,
            };

            match event_type {
                "session_meta" => {
                    let payload = v.get("payload").cloned().unwrap_or(Value::Null);
                    let id = payload
                        .get("id")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    let cwd = payload
                        .get("cwd")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    let timestamp = payload
                        .get("timestamp")
                        .and_then(|t| t.as_str())
                        .and_then(|s| s.parse::<DateTime<Utc>>().ok())
                        .unwrap_or_else(Utc::now);

                    events.push(TimestampedEvent {
                        timestamp,
                        event: PulseEvent::SessionStart {
                            session_id: id,
                            cwd,
                            provider: Provider::Codex,
                        },
                    });
                }
                "response_item" => {
                    let timestamp = v
                        .get("timestamp")
                        .and_then(|t| t.as_str())
                        .and_then(|s| s.parse::<DateTime<Utc>>().ok())
                        .unwrap_or_else(Utc::now);

                    let role = v.get("role").and_then(|r| r.as_str()).unwrap_or("");

                    if role == "user" {
                        if let Some(content_array) = v.get("content").and_then(|c| c.as_array()) {
                            for item in content_array {
                                let item_type =
                                    item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                if item_type == "input_text" {
                                    let raw_text =
                                        item.get("text").and_then(|t| t.as_str()).unwrap_or("");
                                    let content = strip_codex_prefix(raw_text);
                                    events.push(TimestampedEvent {
                                        timestamp,
                                        event: PulseEvent::UserMessage { content },
                                    });
                                }
                            }
                        }
                    } else if role == "assistant" {
                        let mut text_parts = Vec::new();
                        if let Some(content_array) = v.get("content").and_then(|c| c.as_array()) {
                            for item in content_array {
                                let item_type =
                                    item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                if item_type == "output_text" || item_type == "text" {
                                    if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                        text_parts.push(text.to_string());
                                    }
                                }
                            }
                        }
                        if !text_parts.is_empty() {
                            events.push(TimestampedEvent {
                                timestamp,
                                event: PulseEvent::AssistantMessage {
                                    content: text_parts.join("\n"),
                                    tokens: TokenUsage::default(),
                                    model: None,
                                },
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        Ok((events, current_offset))
    }
}

fn strip_codex_prefix(text: &str) -> String {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix(CODEX_PREFIX) {
        rest.trim().to_string()
    } else {
        trimmed.to_string()
    }
}

fn collect_rollout_files(dir: &PathBuf, sessions: &mut Vec<DiscoveredSession>) {
    if !dir.is_dir() {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rollout_files(&path, sessions);
        } else {
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if name.starts_with("rollout-") && name.ends_with(".jsonl") {
                if let Some(session) = read_session_meta(&path) {
                    sessions.push(session);
                }
            }
        }
    }
}

fn read_session_meta(path: &PathBuf) -> Option<DiscoveredSession> {
    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);

    for line in reader.lines() {
        let line = line.ok()?;
        let v: Value = serde_json::from_str(line.trim()).ok()?;
        if v.get("type").and_then(|t| t.as_str()) == Some("session_meta") {
            let payload = v.get("payload")?;
            let id = payload.get("id")?.as_str()?.to_string();
            let cwd = payload
                .get("cwd")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();

            let is_active = is_recently_modified(path, 60);

            return Some(DiscoveredSession {
                session_id: id,
                provider: Provider::Codex,
                path: path.clone(),
                cwd,
                summary: String::new(),
                is_active,
            });
        }
    }
    None
}

fn is_recently_modified(path: &PathBuf, secs: u64) -> bool {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|modified| {
            SystemTime::now()
                .duration_since(modified)
                .map(|d| d.as_secs() < secs)
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn parse_session_meta() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("rollout-abc.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"session_meta","payload":{{"id":"sess-1","cwd":"/home/user/project","timestamp":"2025-01-15T10:00:00Z"}}}}"#
        )
        .unwrap();

        let (events, offset) = CodexProvider.parse_events(&path, 0).unwrap();
        assert_eq!(events.len(), 1);
        assert!(offset > 0);
        match &events[0].event {
            PulseEvent::SessionStart {
                session_id,
                cwd,
                provider,
            } => {
                assert_eq!(session_id, "sess-1");
                assert_eq!(cwd, "/home/user/project");
                assert_eq!(*provider, Provider::Codex);
            }
            other => panic!("Expected SessionStart, got {other:?}"),
        }
    }

    #[test]
    fn parse_user_message_strips_prefix() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("rollout-abc.jsonl");
        let line = format!(
            r#"{{"type":"response_item","timestamp":"2025-01-15T10:01:00Z","role":"user","content":[{{"type":"input_text","text":"{prefix} fix the bug"}}]}}"#,
            prefix = CODEX_PREFIX,
        );
        fs::write(&path, format!("{line}\n")).unwrap();

        let (events, _) = CodexProvider.parse_events(&path, 0).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0].event {
            PulseEvent::UserMessage { content } => assert_eq!(content, "fix the bug"),
            other => panic!("Expected UserMessage, got {other:?}"),
        }
    }

    #[test]
    fn parse_user_message_without_prefix() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("rollout-abc.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"response_item","timestamp":"2025-01-15T10:01:00Z","role":"user","content":[{{"type":"input_text","text":"plain message"}}]}}"#
        )
        .unwrap();

        let (events, _) = CodexProvider.parse_events(&path, 0).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0].event {
            PulseEvent::UserMessage { content } => assert_eq!(content, "plain message"),
            other => panic!("Expected UserMessage, got {other:?}"),
        }
    }

    #[test]
    fn parse_assistant_message() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("rollout-abc.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"response_item","timestamp":"2025-01-15T10:02:00Z","role":"assistant","content":[{{"type":"output_text","text":"here is the fix"}}]}}"#
        )
        .unwrap();

        let (events, _) = CodexProvider.parse_events(&path, 0).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0].event {
            PulseEvent::AssistantMessage { content, .. } => {
                assert_eq!(content, "here is the fix");
            }
            other => panic!("Expected AssistantMessage, got {other:?}"),
        }
    }

    #[test]
    fn parse_incremental_offset() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("rollout-abc.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"session_meta","payload":{{"id":"s1","cwd":"/","timestamp":"2025-01-15T10:00:00Z"}}}}"#
        )
        .unwrap();

        let (events1, offset1) = CodexProvider.parse_events(&path, 0).unwrap();
        assert_eq!(events1.len(), 1);

        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"response_item","timestamp":"2025-01-15T10:01:00Z","role":"user","content":[{{"type":"input_text","text":"hello"}}]}}"#
        )
        .unwrap();

        let (events2, offset2) = CodexProvider.parse_events(&path, offset1).unwrap();
        assert_eq!(events2.len(), 1);
        assert!(offset2 > offset1);
    }

    #[test]
    fn discover_finds_rollout_files_recursively() {
        let tmp = TempDir::new().unwrap();
        let nested = tmp.path().join("2025").join("01").join("15");
        fs::create_dir_all(&nested).unwrap();

        let mut f = fs::File::create(nested.join("rollout-abc.jsonl")).unwrap();
        writeln!(
            f,
            r#"{{"type":"session_meta","payload":{{"id":"deep-sess","cwd":"/project","timestamp":"2025-01-15T10:00:00Z"}}}}"#
        )
        .unwrap();

        fs::write(nested.join("other.jsonl"), "{}").unwrap();

        let mut sessions = Vec::new();
        collect_rollout_files(&tmp.path().to_path_buf(), &mut sessions);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "deep-sess");
    }

    #[test]
    fn parse_skips_malformed_lines() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("rollout-abc.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "not json").unwrap();
        writeln!(f, r#"{{"no_type": true}}"#).unwrap();
        writeln!(
            f,
            r#"{{"type":"session_meta","payload":{{"id":"s1","cwd":"/","timestamp":"2025-01-15T10:00:00Z"}}}}"#
        )
        .unwrap();

        let (events, _) = CodexProvider.parse_events(&path, 0).unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn parse_unknown_type_skipped() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("rollout-abc.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"unknown_future_type","data":{{"something":"here"}}}}"#
        )
        .unwrap();

        let (events, _) = CodexProvider.parse_events(&path, 0).unwrap();
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn strip_codex_prefix_works() {
        let with_newline = format!("{CODEX_PREFIX}\ndo the thing");
        assert_eq!(strip_codex_prefix(&with_newline), "do the thing");

        let inline = format!("{CODEX_PREFIX} inline");
        assert_eq!(strip_codex_prefix(&inline), "inline");

        assert_eq!(strip_codex_prefix("no prefix here"), "no prefix here");
    }
}
