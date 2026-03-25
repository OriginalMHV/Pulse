use std::collections::HashMap;
use std::path::PathBuf;

use crate::event::Provider;
use crate::providers::{DiscoveredSession, SessionProvider};
use crate::session::Session;

pub struct Scanner {
    providers: Vec<Box<dyn SessionProvider>>,
    sessions: Vec<Session>,
    offsets: HashMap<PathBuf, u64>,
    provider_filter: Option<Provider>,
}

impl Scanner {
    pub fn new(providers: Vec<Box<dyn SessionProvider>>) -> Self {
        Self {
            providers,
            sessions: Vec::new(),
            offsets: HashMap::new(),
            provider_filter: None,
        }
    }

    /// Initial scan: discover all sessions, parse all events.
    pub fn scan_all(&mut self) {
        self.sessions.clear();
        self.offsets.clear();

        let mut discovered: Vec<DiscoveredSession> = Vec::new();
        for provider in &self.providers {
            discovered.extend(provider.discover_sessions());
        }

        for ds in discovered {
            let mut session = Session::new(ds.session_id, ds.provider, ds.cwd, chrono::Utc::now());
            session.summary = ds.summary;
            session.is_active = ds.is_active;

            let provider = self
                .providers
                .iter()
                .find(|p| p.provider_type() == ds.provider);

            if let Some(provider) = provider {
                let offset = self.offsets.get(&ds.path).copied().unwrap_or(0);
                match provider.parse_events(&ds.path, offset) {
                    Ok((events, new_offset)) => {
                        if let Some(first) = events.first() {
                            session.started_at = first.timestamp;
                            session.last_activity = first.timestamp;
                        }
                        for event in events {
                            session.ingest(event);
                        }
                        self.offsets.insert(ds.path.clone(), new_offset);
                    }
                    Err(e) => {
                        eprintln!(
                            "warn: failed to parse events from {}: {e}",
                            ds.path.display()
                        );
                    }
                }
            }

            self.sessions.push(session);
        }

        self.sort_sessions();
    }

    /// Incremental update: re-read events from a changed file.
    pub fn update_session(&mut self, path: &PathBuf) {
        let session_idx = self.sessions.iter().position(|s| {
            self.offsets
                .keys()
                .any(|p| p == path && p.to_string_lossy().contains(&s.id))
        });

        // Find the provider that handles this file extension / path
        let provider = self
            .providers
            .iter()
            .find(|p| p.base_dirs().iter().any(|base| path.starts_with(base)));

        let Some(provider) = provider else {
            return;
        };

        let offset = self.offsets.get(path).copied().unwrap_or(0);
        let (events, new_offset) = match provider.parse_events(path, offset) {
            Ok(result) => result,
            Err(e) => {
                eprintln!("warn: failed to parse events from {}: {e}", path.display());
                return;
            }
        };

        if events.is_empty() {
            return;
        }

        self.offsets.insert(path.clone(), new_offset);

        if let Some(idx) = session_idx {
            for event in events {
                self.sessions[idx].ingest(event);
            }
        } else {
            // Check if any events carry a SessionStart to bootstrap a new session
            let provider_type = provider.provider_type();
            let mut session = Session::new(
                path.to_string_lossy().into_owned(),
                provider_type,
                String::new(),
                chrono::Utc::now(),
            );
            if let Some(first) = events.first() {
                session.started_at = first.timestamp;
                session.last_activity = first.timestamp;
            }
            for event in events {
                session.ingest(event);
            }
            self.sessions.push(session);
        }

        self.sort_sessions();
    }

    /// Filter sessions by provider name string.
    pub fn filter_provider(&mut self, name: &str) {
        let lower = name.to_lowercase();
        self.provider_filter = match lower.as_str() {
            "copilot" => Some(Provider::Copilot),
            "claude" => Some(Provider::Claude),
            "codex" => Some(Provider::Codex),
            _ => None,
        };
    }

    /// Get sorted sessions (active first, then by last_activity descending).
    /// If a provider filter is set, only matching sessions are returned.
    pub fn sessions(&self) -> &[Session] {
        &self.sessions
    }

    /// Get filtered sessions when a provider filter is active.
    pub fn filtered_sessions(&self) -> Vec<&Session> {
        match self.provider_filter {
            Some(provider) => self
                .sessions
                .iter()
                .filter(|s| s.provider == provider)
                .collect(),
            None => self.sessions.iter().collect(),
        }
    }

    /// Get all directories that should be watched.
    pub fn watched_dirs(&self) -> Vec<PathBuf> {
        self.providers
            .iter()
            .flat_map(|p| p.base_dirs())
            .filter(|d| d.exists())
            .collect()
    }

    fn sort_sessions(&mut self) {
        self.sessions.sort_by(|a, b| {
            // Active sessions first
            b.is_active
                .cmp(&a.is_active)
                .then_with(|| b.last_activity.cmp(&a.last_activity))
        });
    }

    #[cfg(test)]
    pub fn with_sessions(sessions: Vec<Session>) -> Self {
        Self {
            providers: Vec::new(),
            sessions,
            offsets: std::collections::HashMap::new(),
            provider_filter: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{PulseEvent, TimestampedEvent};
    use chrono::Utc;
    use std::fs;
    use tempfile::TempDir;

    /// A test provider that discovers sessions from JSONL files in a temp dir.
    struct TestProvider {
        base: PathBuf,
    }

    impl SessionProvider for TestProvider {
        fn provider_type(&self) -> Provider {
            Provider::Claude
        }

        fn base_dirs(&self) -> Vec<PathBuf> {
            vec![self.base.clone()]
        }

        fn discover_sessions(&self) -> Vec<DiscoveredSession> {
            let mut sessions = Vec::new();
            if let Ok(entries) = fs::read_dir(&self.base) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
                        sessions.push(DiscoveredSession {
                            session_id: name.clone(),
                            provider: Provider::Claude,
                            path,
                            cwd: "/test".into(),
                            summary: format!("Test session {name}"),
                            is_active: false,
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
        ) -> anyhow::Result<(Vec<TimestampedEvent>, u64)> {
            let content = fs::read_to_string(path)?;
            let bytes = content.as_bytes();
            let remaining = &bytes[offset as usize..];
            let text = std::str::from_utf8(remaining)?;
            let mut events = Vec::new();
            for line in text.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                // Simple: each line is a user message
                events.push(TimestampedEvent {
                    timestamp: Utc::now(),
                    event: PulseEvent::UserMessage {
                        content: line.to_string(),
                    },
                });
            }
            Ok((events, bytes.len() as u64))
        }
    }

    #[test]
    fn scan_all_discovers_sessions() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("session1.jsonl"), "hello\nworld\n").unwrap();
        fs::write(dir.path().join("session2.jsonl"), "foo\n").unwrap();

        let providers: Vec<Box<dyn SessionProvider>> = vec![Box::new(TestProvider {
            base: dir.path().to_path_buf(),
        })];
        let mut scanner = Scanner::new(providers);
        scanner.scan_all();

        assert_eq!(scanner.sessions().len(), 2);
    }

    #[test]
    fn scan_all_parses_events() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("s1.jsonl"), "msg1\nmsg2\nmsg3\n").unwrap();

        let providers: Vec<Box<dyn SessionProvider>> = vec![Box::new(TestProvider {
            base: dir.path().to_path_buf(),
        })];
        let mut scanner = Scanner::new(providers);
        scanner.scan_all();

        assert_eq!(scanner.sessions().len(), 1);
        assert_eq!(scanner.sessions()[0].user_message_count, 3);
        assert_eq!(scanner.sessions()[0].events.len(), 3);
    }

    #[test]
    fn scan_all_tracks_offsets() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("s1.jsonl");
        fs::write(&path, "line1\nline2\n").unwrap();

        let providers: Vec<Box<dyn SessionProvider>> = vec![Box::new(TestProvider {
            base: dir.path().to_path_buf(),
        })];
        let mut scanner = Scanner::new(providers);
        scanner.scan_all();

        let offset = scanner.offsets.get(&path).copied().unwrap();
        assert_eq!(offset, 12); // "line1\nline2\n" = 12 bytes
    }

    #[test]
    fn update_session_reads_incrementally() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("s1.jsonl");
        fs::write(&path, "initial\n").unwrap();

        let providers: Vec<Box<dyn SessionProvider>> = vec![Box::new(TestProvider {
            base: dir.path().to_path_buf(),
        })];
        let mut scanner = Scanner::new(providers);
        scanner.scan_all();

        assert_eq!(scanner.sessions()[0].user_message_count, 1);

        // Append new content
        use std::io::Write;
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "appended").unwrap();
        drop(file);

        scanner.update_session(&path);
        assert_eq!(scanner.sessions()[0].user_message_count, 2);
    }

    #[test]
    fn filter_provider_filters_sessions() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("s1.jsonl"), "hello\n").unwrap();

        let providers: Vec<Box<dyn SessionProvider>> = vec![Box::new(TestProvider {
            base: dir.path().to_path_buf(),
        })];
        let mut scanner = Scanner::new(providers);
        scanner.scan_all();

        // TestProvider uses Claude, so filtering for Copilot should give 0
        scanner.filter_provider("copilot");
        assert_eq!(scanner.filtered_sessions().len(), 0);

        // Filtering for Claude should give 1
        scanner.filter_provider("claude");
        assert_eq!(scanner.filtered_sessions().len(), 1);
    }

    #[test]
    fn sort_active_first() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("inactive.jsonl"), "a\n").unwrap();
        fs::write(dir.path().join("active.jsonl"), "b\n").unwrap();

        let providers: Vec<Box<dyn SessionProvider>> = vec![Box::new(TestProvider {
            base: dir.path().to_path_buf(),
        })];
        let mut scanner = Scanner::new(providers);
        scanner.scan_all();

        // Manually set one session as active
        if let Some(s) = scanner.sessions.iter_mut().find(|s| s.id == "active") {
            s.is_active = true;
        }
        scanner.sort_sessions();

        assert!(scanner.sessions()[0].is_active);
    }

    #[test]
    fn watched_dirs_returns_existing_dirs() {
        let dir = TempDir::new().unwrap();

        let providers: Vec<Box<dyn SessionProvider>> = vec![Box::new(TestProvider {
            base: dir.path().to_path_buf(),
        })];
        let scanner = Scanner::new(providers);

        let dirs = scanner.watched_dirs();
        assert_eq!(dirs.len(), 1);
        assert_eq!(dirs[0], dir.path());
    }

    #[test]
    fn watched_dirs_skips_nonexistent() {
        let providers: Vec<Box<dyn SessionProvider>> = vec![Box::new(TestProvider {
            base: PathBuf::from("/nonexistent/path/that/does/not/exist"),
        })];
        let scanner = Scanner::new(providers);

        assert!(scanner.watched_dirs().is_empty());
    }

    #[test]
    fn empty_scan_produces_no_sessions() {
        let dir = TempDir::new().unwrap();
        // No JSONL files in dir

        let providers: Vec<Box<dyn SessionProvider>> = vec![Box::new(TestProvider {
            base: dir.path().to_path_buf(),
        })];
        let mut scanner = Scanner::new(providers);
        scanner.scan_all();

        assert!(scanner.sessions().is_empty());
    }
}
