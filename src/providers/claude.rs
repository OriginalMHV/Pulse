use std::fs;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::event::{Provider, PulseEvent, TimestampedEvent, TokenUsage};
use crate::providers::{DiscoveredSession, SessionProvider};

pub struct ClaudeProvider;

impl SessionProvider for ClaudeProvider {
    fn provider_type(&self) -> Provider {
        Provider::Claude
    }

    fn base_dirs(&self) -> Vec<PathBuf> {
        dirs::home_dir()
            .map(|h| vec![h.join(".claude").join("projects")])
            .unwrap_or_default()
    }

    fn discover_sessions(&self) -> Vec<DiscoveredSession> {
        let mut sessions = Vec::new();
        for base in self.base_dirs() {
            if !base.is_dir() {
                continue;
            }
            let project_dirs = match fs::read_dir(&base) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for project_entry in project_dirs.flatten() {
                let project_dir = project_entry.path();
                if !project_dir.is_dir() {
                    continue;
                }
                let files = match fs::read_dir(&project_dir) {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                for file_entry in files.flatten() {
                    let path = file_entry.path();
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();

                    if !name.ends_with(".jsonl") || name.starts_with("agent-") {
                        continue;
                    }

                    let is_active = is_recently_modified(&path, 60);
                    let (session_id, cwd) = read_first_user_line(&path).unwrap_or_else(|| {
                        let id = path
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();
                        (id, String::new())
                    });

                    sessions.push(DiscoveredSession {
                        session_id,
                        provider: Provider::Claude,
                        path,
                        cwd,
                        summary: String::new(),
                        is_active,
                    });
                }
            }
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

            let msg_type = match v.get("type").and_then(|t| t.as_str()) {
                Some(t) => t,
                None => continue,
            };

            let timestamp = v
                .get("timestamp")
                .and_then(|t| t.as_str())
                .and_then(|s| s.parse::<DateTime<Utc>>().ok())
                .unwrap_or_else(Utc::now);

            match msg_type {
                "user" => {
                    let content = v
                        .pointer("/message/content")
                        .and_then(|c| c.as_str())
                        .unwrap_or("")
                        .to_string();
                    events.push(TimestampedEvent {
                        timestamp,
                        event: PulseEvent::UserMessage { content },
                    });
                }
                "assistant" => {
                    let model = v
                        .pointer("/message/model")
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string());

                    let usage = v.pointer("/message/usage");
                    let tokens = TokenUsage {
                        input: usage
                            .and_then(|u| u.get("input_tokens"))
                            .and_then(|n| n.as_u64())
                            .unwrap_or(0),
                        output: usage
                            .and_then(|u| u.get("output_tokens"))
                            .and_then(|n| n.as_u64())
                            .unwrap_or(0),
                        cache_read: usage
                            .and_then(|u| u.get("cache_read_input_tokens"))
                            .and_then(|n| n.as_u64())
                            .unwrap_or(0),
                        cache_write: usage
                            .and_then(|u| u.get("cache_creation_input_tokens"))
                            .and_then(|n| n.as_u64())
                            .unwrap_or(0),
                    };

                    let mut text_parts = Vec::new();
                    if let Some(content_array) =
                        v.pointer("/message/content").and_then(|c| c.as_array())
                    {
                        for item in content_array {
                            let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                            match item_type {
                                "text" => {
                                    if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                        text_parts.push(text.to_string());
                                    }
                                }
                                "tool_use" => {
                                    let name = item
                                        .get("name")
                                        .and_then(|n| n.as_str())
                                        .unwrap_or("unknown")
                                        .to_string();
                                    let input = item.get("input").cloned().unwrap_or(Value::Null);
                                    events.push(TimestampedEvent {
                                        timestamp,
                                        event: PulseEvent::ToolStart {
                                            name,
                                            arguments: input,
                                        },
                                    });
                                }
                                _ => {}
                            }
                        }
                    }

                    let content = text_parts.join("\n");
                    events.push(TimestampedEvent {
                        timestamp,
                        event: PulseEvent::AssistantMessage {
                            content,
                            tokens,
                            model,
                        },
                    });
                }
                "system" => {
                    let message = v
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("")
                        .to_string();
                    events.push(TimestampedEvent {
                        timestamp,
                        event: PulseEvent::Warning { message },
                    });
                }
                _ => {}
            }
        }

        Ok((events, current_offset))
    }
}

fn read_first_user_line(path: &PathBuf) -> Option<(String, String)> {
    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);

    for line in reader.lines() {
        let line = line.ok()?;
        let v: Value = serde_json::from_str(line.trim()).ok()?;
        if v.get("type").and_then(|t| t.as_str()) == Some("user") {
            let session_id = v
                .get("sessionId")
                .and_then(|s| s.as_str())
                .unwrap_or_else(|| path.file_stem().unwrap_or_default().to_str().unwrap_or(""))
                .to_string();
            let cwd = v
                .get("cwd")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            return Some((session_id, cwd));
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

    /// Helper to discover sessions in a custom base path (for testing)
    fn discover_in_base(base: &std::path::Path) -> Vec<DiscoveredSession> {
        let mut sessions = Vec::new();
        let project_dirs = match fs::read_dir(base) {
            Ok(e) => e,
            Err(_) => return sessions,
        };
        for project_entry in project_dirs.flatten() {
            let project_dir = project_entry.path();
            if !project_dir.is_dir() {
                continue;
            }
            let files = match fs::read_dir(&project_dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for file_entry in files.flatten() {
                let path = file_entry.path();
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                if !name.ends_with(".jsonl") || name.starts_with("agent-") {
                    continue;
                }
                let (session_id, cwd) = read_first_user_line(&path).unwrap_or_else(|| {
                    let id = path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    (id, String::new())
                });
                sessions.push(DiscoveredSession {
                    session_id,
                    provider: Provider::Claude,
                    path,
                    cwd,
                    summary: String::new(),
                    is_active: false,
                });
            }
        }
        sessions
    }

    #[test]
    fn parse_user_message() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("session.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"user","timestamp":"2025-01-15T10:00:00Z","message":{{"content":"hello claude"}},"sessionId":"s1","cwd":"/proj"}}"#
        )
        .unwrap();

        let (events, offset) = ClaudeProvider.parse_events(&path, 0).unwrap();
        assert_eq!(events.len(), 1);
        assert!(offset > 0);
        match &events[0].event {
            PulseEvent::UserMessage { content } => assert_eq!(content, "hello claude"),
            other => panic!("Expected UserMessage, got {other:?}"),
        }
    }

    #[test]
    fn parse_assistant_with_text_and_tool_use() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("session.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"assistant","timestamp":"2025-01-15T10:01:00Z","message":{{"content":[{{"type":"text","text":"let me check"}},{{"type":"tool_use","name":"bash","input":{{"command":"ls"}}}}],"usage":{{"input_tokens":1000,"output_tokens":500,"cache_read_input_tokens":5000,"cache_creation_input_tokens":200}},"model":"claude-sonnet-4-5"}}}}"#
        )
        .unwrap();

        let (events, _) = ClaudeProvider.parse_events(&path, 0).unwrap();
        assert_eq!(events.len(), 2);

        match &events[0].event {
            PulseEvent::ToolStart { name, arguments } => {
                assert_eq!(name, "bash");
                assert_eq!(arguments.get("command").unwrap().as_str().unwrap(), "ls");
            }
            other => panic!("Expected ToolStart, got {other:?}"),
        }

        match &events[1].event {
            PulseEvent::AssistantMessage {
                content,
                tokens,
                model,
            } => {
                assert_eq!(content, "let me check");
                assert_eq!(tokens.input, 1000);
                assert_eq!(tokens.output, 500);
                assert_eq!(tokens.cache_read, 5000);
                assert_eq!(tokens.cache_write, 200);
                assert_eq!(model.as_deref(), Some("claude-sonnet-4-5"));
            }
            other => panic!("Expected AssistantMessage, got {other:?}"),
        }
    }

    #[test]
    fn parse_system_as_warning() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("session.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"system","timestamp":"2025-01-15T10:00:00Z","subtype":"rate_limit","message":"slow down"}}"#
        )
        .unwrap();

        let (events, _) = ClaudeProvider.parse_events(&path, 0).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0].event {
            PulseEvent::Warning { message } => assert_eq!(message, "slow down"),
            other => panic!("Expected Warning, got {other:?}"),
        }
    }

    #[test]
    fn parse_skips_progress_and_queue() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("session.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"progress","timestamp":"2025-01-15T10:00:00Z","data":{{"type":"thinking","message":"processing"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"queue-operation","timestamp":"2025-01-15T10:00:00Z"}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"file-history-snapshot","timestamp":"2025-01-15T10:00:00Z"}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"user","timestamp":"2025-01-15T10:01:00Z","message":{{"content":"actual message"}},"sessionId":"s1","cwd":"/"}}"#
        )
        .unwrap();

        let (events, _) = ClaudeProvider.parse_events(&path, 0).unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn parse_incremental_read() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("session.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"user","timestamp":"2025-01-15T10:00:00Z","message":{{"content":"first"}},"sessionId":"s1","cwd":"/"}}"#
        )
        .unwrap();

        let (events1, offset1) = ClaudeProvider.parse_events(&path, 0).unwrap();
        assert_eq!(events1.len(), 1);

        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"user","timestamp":"2025-01-15T10:01:00Z","message":{{"content":"second"}},"sessionId":"s1","cwd":"/"}}"#
        )
        .unwrap();

        let (events2, offset2) = ClaudeProvider.parse_events(&path, offset1).unwrap();
        assert_eq!(events2.len(), 1);
        assert!(offset2 > offset1);
        match &events2[0].event {
            PulseEvent::UserMessage { content } => assert_eq!(content, "second"),
            other => panic!("Expected UserMessage, got {other:?}"),
        }
    }

    #[test]
    fn discover_sessions_excludes_agent_files() {
        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path().join("project1");
        fs::create_dir_all(&project_dir).unwrap();

        let mut f = fs::File::create(project_dir.join("abc-123.jsonl")).unwrap();
        writeln!(
            f,
            r#"{{"type":"user","timestamp":"2025-01-15T10:00:00Z","message":{{"content":"hi"}},"sessionId":"abc-123","cwd":"/proj"}}"#
        )
        .unwrap();

        fs::write(project_dir.join("agent-def-456.jsonl"), "{}").unwrap();

        let sessions = discover_in_base(tmp.path());
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "abc-123");
    }

    #[test]
    fn parse_assistant_thinking_ignored() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("session.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"assistant","timestamp":"2025-01-15T10:01:00Z","message":{{"content":[{{"type":"thinking","thinking":"deep thought"}},{{"type":"text","text":"answer"}}],"usage":{{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}},"model":"claude-sonnet-4"}}}}"#
        )
        .unwrap();

        let (events, _) = ClaudeProvider.parse_events(&path, 0).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0].event {
            PulseEvent::AssistantMessage { content, .. } => assert_eq!(content, "answer"),
            other => panic!("Expected AssistantMessage, got {other:?}"),
        }
    }

    #[test]
    fn parse_skips_malformed_lines() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("session.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "garbage").unwrap();
        writeln!(f, r#"{{"no_type": true}}"#).unwrap();
        writeln!(
            f,
            r#"{{"type":"user","timestamp":"2025-01-15T10:00:00Z","message":{{"content":"ok"}},"sessionId":"s1","cwd":"/"}}"#
        )
        .unwrap();

        let (events, _) = ClaudeProvider.parse_events(&path, 0).unwrap();
        assert_eq!(events.len(), 1);
    }
}
