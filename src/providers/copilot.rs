use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::event::{Provider, PulseEvent, TimestampedEvent, TokenUsage};
use crate::providers::{DiscoveredSession, SessionProvider};

pub struct CopilotProvider;

impl SessionProvider for CopilotProvider {
    fn provider_type(&self) -> Provider {
        Provider::Copilot
    }

    fn base_dirs(&self) -> Vec<PathBuf> {
        dirs::home_dir()
            .map(|h| vec![h.join(".copilot").join("session-state")])
            .unwrap_or_default()
    }

    fn discover_sessions(&self) -> Vec<DiscoveredSession> {
        let mut sessions = Vec::new();
        for base in self.base_dirs() {
            let entries = match fs::read_dir(&base) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let dir = entry.path();
                if !dir.is_dir() {
                    continue;
                }
                let events_path = dir.join("events.jsonl");
                if !events_path.exists() {
                    continue;
                }

                let workspace_path = dir.join("workspace.yaml");
                let (session_id, cwd, summary) = parse_workspace_yaml(&workspace_path)
                    .unwrap_or_else(|| {
                        let id = dir
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();
                        (id, String::new(), String::new())
                    });

                let is_active = has_lock_file(&dir);

                sessions.push(DiscoveredSession {
                    session_id,
                    provider: Provider::Copilot,
                    path: events_path,
                    cwd,
                    summary,
                    is_active,
                });
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
        // Track tool start times for duration calculation: tool_call_id -> (tool_name, timestamp)
        let mut tool_starts: HashMap<String, (String, DateTime<Utc>)> = HashMap::new();

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

            let timestamp = parse_timestamp_from_value(&v).unwrap_or_else(Utc::now);
            let data = v.get("data").cloned().unwrap_or(Value::Null);

            match event_type {
                "session.start" => {
                    let session_id = data
                        .get("sessionId")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    let cwd = data
                        .pointer("/context/cwd")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    events.push(TimestampedEvent {
                        timestamp,
                        event: PulseEvent::SessionStart {
                            session_id,
                            cwd,
                            provider: Provider::Copilot,
                        },
                    });
                }
                "user.message" => {
                    let content = data
                        .get("content")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    events.push(TimestampedEvent {
                        timestamp,
                        event: PulseEvent::UserMessage { content },
                    });
                }
                "assistant.turn_start" => {
                    let turn_id = data
                        .get("turnId")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    events.push(TimestampedEvent {
                        timestamp,
                        event: PulseEvent::TurnStart { turn_id },
                    });
                }
                "assistant.message" => {
                    // Emit ToolStart events for each tool request
                    if let Some(tool_requests) = data.get("toolRequests").and_then(|t| t.as_array())
                    {
                        for req in tool_requests {
                            let name = req
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            let arguments = req.get("arguments").cloned().unwrap_or(Value::Null);
                            // Try to parse arguments as JSON if it's a string
                            let arguments = if let Some(s) = arguments.as_str() {
                                serde_json::from_str(s).unwrap_or(Value::String(s.to_string()))
                            } else {
                                arguments
                            };
                            events.push(TimestampedEvent {
                                timestamp,
                                event: PulseEvent::ToolStart { name, arguments },
                            });
                        }
                    }

                    let content = data
                        .get("content")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    let output_tokens = data
                        .get("outputTokens")
                        .and_then(|n| n.as_u64())
                        .unwrap_or(0);
                    let model = data
                        .get("model")
                        .and_then(|s| s.as_str())
                        .map(|s| s.to_string());

                    events.push(TimestampedEvent {
                        timestamp,
                        event: PulseEvent::AssistantMessage {
                            content,
                            tokens: TokenUsage {
                                output: output_tokens,
                                ..Default::default()
                            },
                            model,
                        },
                    });
                }
                "tool.execution_start" => {
                    let tool_call_id = data
                        .get("toolCallId")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    let tool_name = data
                        .get("toolName")
                        .and_then(|s| s.as_str())
                        .unwrap_or("unknown")
                        .to_string();

                    tool_starts.insert(tool_call_id, (tool_name, timestamp));
                }
                "tool.execution_complete" => {
                    let tool_call_id = data
                        .get("toolCallId")
                        .and_then(|s| s.as_str())
                        .unwrap_or("");
                    let success = data
                        .get("success")
                        .and_then(|b| b.as_bool())
                        .unwrap_or(false);

                    let (name, duration_ms) =
                        if let Some((name, start_ts)) = tool_starts.remove(tool_call_id) {
                            let duration = (timestamp - start_ts).num_milliseconds().max(0) as u64;
                            (name, Some(duration))
                        } else {
                            ("unknown".to_string(), None)
                        };

                    events.push(TimestampedEvent {
                        timestamp,
                        event: PulseEvent::ToolComplete {
                            name,
                            success,
                            duration_ms,
                        },
                    });
                }
                "assistant.turn_end" => {
                    let turn_id = data
                        .get("turnId")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    events.push(TimestampedEvent {
                        timestamp,
                        event: PulseEvent::TurnEnd { turn_id },
                    });
                }
                "session.warning" => {
                    let message = data
                        .get("message")
                        .and_then(|s| s.as_str())
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

fn parse_workspace_yaml(path: &PathBuf) -> Option<(String, String, String)> {
    let content = fs::read_to_string(path).ok()?;
    let yaml: Value = serde_yaml::from_str(&content).ok()?;
    let id = yaml.get("id")?.as_str()?.to_string();
    let cwd = yaml
        .get("cwd")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let summary = yaml
        .get("summary")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    Some((id, cwd, summary))
}

fn has_lock_file(dir: &PathBuf) -> bool {
    fs::read_dir(dir)
        .map(|entries| {
            entries.flatten().any(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                name.starts_with("inuse.") && name.ends_with(".lock")
            })
        })
        .unwrap_or(false)
}

fn parse_timestamp_from_value(v: &Value) -> Option<DateTime<Utc>> {
    v.get("timestamp")
        .and_then(parse_ts)
        .or_else(|| v.pointer("/data/startTime").and_then(parse_ts))
        .or_else(|| v.pointer("/data/timestamp").and_then(parse_ts))
}

fn parse_ts(v: &Value) -> Option<DateTime<Utc>> {
    if let Some(s) = v.as_str() {
        s.parse::<DateTime<Utc>>().ok()
    } else if let Some(n) = v.as_f64() {
        let secs = (n / 1000.0) as i64;
        let nanos = ((n % 1000.0) * 1_000_000.0) as u32;
        DateTime::from_timestamp(secs, nanos)
    } else if let Some(n) = v.as_i64() {
        DateTime::from_timestamp(n / 1000, ((n % 1000) * 1_000_000) as u32)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_session_dir(
        base: &std::path::Path,
        id: &str,
        cwd: &str,
        summary: &str,
        active: bool,
    ) -> PathBuf {
        let dir = base.join(id);
        fs::create_dir_all(&dir).unwrap();

        let yaml = format!("id: {id}\ncwd: {cwd}\nsummary: {summary}\n");
        fs::write(dir.join("workspace.yaml"), yaml).unwrap();
        fs::write(dir.join("events.jsonl"), "").unwrap();

        if active {
            fs::write(dir.join("inuse.12345.lock"), "").unwrap();
        }

        dir
    }

    /// Helper to discover sessions using a custom base path (for testing)
    fn discover_in_base(
        _provider: &CopilotProvider,
        base: &std::path::Path,
    ) -> Vec<DiscoveredSession> {
        let mut sessions = Vec::new();
        let entries = match fs::read_dir(base) {
            Ok(e) => e,
            Err(_) => return sessions,
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let events_path = dir.join("events.jsonl");
            if !events_path.exists() {
                continue;
            }
            let workspace_path = dir.join("workspace.yaml");
            let (session_id, cwd, summary) =
                parse_workspace_yaml(&workspace_path).unwrap_or_else(|| {
                    let id = dir
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    (id, String::new(), String::new())
                });
            let is_active = has_lock_file(&dir);
            sessions.push(DiscoveredSession {
                session_id,
                provider: Provider::Copilot,
                path: events_path,
                cwd,
                summary,
                is_active,
            });
        }
        sessions
    }

    #[test]
    fn discover_sessions_finds_directories() {
        let tmp = TempDir::new().unwrap();
        create_session_dir(
            tmp.path(),
            "abc-123",
            "/home/user/project",
            "my session",
            false,
        );
        create_session_dir(
            tmp.path(),
            "def-456",
            "/home/user/other",
            "other session",
            true,
        );

        let provider = CopilotProvider;
        let sessions = discover_in_base(&provider, tmp.path());

        assert_eq!(sessions.len(), 2);
        let ids: Vec<&str> = sessions.iter().map(|s| s.session_id.as_str()).collect();
        assert!(ids.contains(&"abc-123"));
        assert!(ids.contains(&"def-456"));
    }

    #[test]
    fn discover_sessions_detects_active() {
        let tmp = TempDir::new().unwrap();
        create_session_dir(tmp.path(), "active-1", "/proj", "", true);
        create_session_dir(tmp.path(), "inactive-1", "/proj", "", false);

        let sessions = discover_in_base(&CopilotProvider, tmp.path());
        let active = sessions
            .iter()
            .find(|s| s.session_id == "active-1")
            .unwrap();
        let inactive = sessions
            .iter()
            .find(|s| s.session_id == "inactive-1")
            .unwrap();
        assert!(active.is_active);
        assert!(!inactive.is_active);
    }

    #[test]
    fn parse_events_session_start() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("events.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"session.start","timestamp":"2025-01-15T10:00:00Z","data":{{"sessionId":"s1","context":{{"cwd":"/proj"}},"startTime":"2025-01-15T10:00:00Z"}}}}"#
        )
        .unwrap();

        let (events, offset) = CopilotProvider.parse_events(&path, 0).unwrap();
        assert_eq!(events.len(), 1);
        assert!(offset > 0);
        match &events[0].event {
            PulseEvent::SessionStart {
                session_id, cwd, ..
            } => {
                assert_eq!(session_id, "s1");
                assert_eq!(cwd, "/proj");
            }
            other => panic!("Expected SessionStart, got {other:?}"),
        }
    }

    #[test]
    fn parse_events_user_and_assistant() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("events.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"user.message","timestamp":"2025-01-15T10:01:00Z","data":{{"content":"hello world"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"assistant.message","timestamp":"2025-01-15T10:01:05Z","data":{{"content":"hi there","outputTokens":150,"model":"gpt-4.1"}}}}"#
        )
        .unwrap();

        let (events, _) = CopilotProvider.parse_events(&path, 0).unwrap();
        assert_eq!(events.len(), 2);
        match &events[0].event {
            PulseEvent::UserMessage { content } => assert_eq!(content, "hello world"),
            other => panic!("Expected UserMessage, got {other:?}"),
        }
        match &events[1].event {
            PulseEvent::AssistantMessage { tokens, model, .. } => {
                assert_eq!(tokens.output, 150);
                assert_eq!(model.as_deref(), Some("gpt-4.1"));
            }
            other => panic!("Expected AssistantMessage, got {other:?}"),
        }
    }

    #[test]
    fn parse_events_tool_lifecycle() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("events.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"tool.execution_start","timestamp":"2025-01-15T10:01:00Z","data":{{"toolCallId":"tc1","toolName":"bash","arguments":{{"command":"ls"}}}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"tool.execution_complete","timestamp":"2025-01-15T10:01:02Z","data":{{"toolCallId":"tc1","success":true,"result":"file1 file2"}}}}"#
        )
        .unwrap();

        let (events, _) = CopilotProvider.parse_events(&path, 0).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0].event {
            PulseEvent::ToolComplete {
                name,
                success,
                duration_ms,
            } => {
                assert_eq!(name, "bash");
                assert!(success);
                assert_eq!(*duration_ms, Some(2000));
            }
            other => panic!("Expected ToolComplete, got {other:?}"),
        }
    }

    #[test]
    fn parse_events_incremental_offset() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("events.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"user.message","timestamp":"2025-01-15T10:01:00Z","data":{{"content":"first"}}}}"#
        )
        .unwrap();

        let (events1, offset1) = CopilotProvider.parse_events(&path, 0).unwrap();
        assert_eq!(events1.len(), 1);

        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"user.message","timestamp":"2025-01-15T10:02:00Z","data":{{"content":"second"}}}}"#
        )
        .unwrap();

        let (events2, offset2) = CopilotProvider.parse_events(&path, offset1).unwrap();
        assert_eq!(events2.len(), 1);
        assert!(offset2 > offset1);
        match &events2[0].event {
            PulseEvent::UserMessage { content } => assert_eq!(content, "second"),
            other => panic!("Expected UserMessage, got {other:?}"),
        }
    }

    #[test]
    fn parse_events_assistant_with_tool_requests() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("events.jsonl");
        let inner_args = r#"{"command":"cargo test"}"#.replace('"', r#"\""#);
        let line = format!(
            r#"{{"type":"assistant.message","timestamp":"2025-01-15T10:01:00Z","data":{{"content":"let me check","outputTokens":50,"toolRequests":[{{"name":"bash","arguments":"{inner_args}"}}]}}}}"#,
        );
        fs::write(&path, format!("{line}\n")).unwrap();

        let (events, _) = CopilotProvider.parse_events(&path, 0).unwrap();
        assert_eq!(events.len(), 2);
        match &events[0].event {
            PulseEvent::ToolStart { name, arguments } => {
                assert_eq!(name, "bash");
                assert_eq!(
                    arguments.get("command").unwrap().as_str().unwrap(),
                    "cargo test"
                );
            }
            other => panic!("Expected ToolStart, got {other:?}"),
        }
    }

    #[test]
    fn parse_events_turn_start_and_end() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("events.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"assistant.turn_start","timestamp":"2025-01-15T10:01:00Z","data":{{"turnId":"t1"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"assistant.turn_end","timestamp":"2025-01-15T10:01:30Z","data":{{"turnId":"t1"}}}}"#
        )
        .unwrap();

        let (events, _) = CopilotProvider.parse_events(&path, 0).unwrap();
        assert_eq!(events.len(), 2);
        match &events[0].event {
            PulseEvent::TurnStart { turn_id } => assert_eq!(turn_id, "t1"),
            other => panic!("Expected TurnStart, got {other:?}"),
        }
        match &events[1].event {
            PulseEvent::TurnEnd { turn_id } => assert_eq!(turn_id, "t1"),
            other => panic!("Expected TurnEnd, got {other:?}"),
        }
    }

    #[test]
    fn parse_events_warning() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("events.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"session.warning","timestamp":"2025-01-15T10:01:00Z","data":{{"message":"rate limited"}}}}"#
        )
        .unwrap();

        let (events, _) = CopilotProvider.parse_events(&path, 0).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0].event {
            PulseEvent::Warning { message } => assert_eq!(message, "rate limited"),
            other => panic!("Expected Warning, got {other:?}"),
        }
    }

    #[test]
    fn parse_events_skips_malformed_lines() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("events.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "not valid json at all").unwrap();
        writeln!(f, r#"{{"no_type_field": true}}"#).unwrap();
        writeln!(
            f,
            r#"{{"type":"user.message","timestamp":"2025-01-15T10:01:00Z","data":{{"content":"valid"}}}}"#
        )
        .unwrap();

        let (events, _) = CopilotProvider.parse_events(&path, 0).unwrap();
        assert_eq!(events.len(), 1);
    }
}
