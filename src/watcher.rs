use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use notify::{EventKind, RecursiveMode, Watcher as NotifyWatcher, recommended_watcher};

#[derive(Debug)]
pub enum WatchEvent {
    FileChanged(PathBuf),
}

pub struct Watcher {
    _watcher: Box<dyn NotifyWatcher>,
    _thread: thread::JoinHandle<()>,
}

impl Watcher {
    pub fn start(dirs: Vec<PathBuf>, tx: mpsc::Sender<WatchEvent>) -> anyhow::Result<Self> {
        let (notify_tx, notify_rx) = mpsc::channel::<notify::Result<notify::Event>>();

        let mut watcher = recommended_watcher(move |res| {
            let _ = notify_tx.send(res);
        })?;

        for dir in &dirs {
            if !dir.exists() {
                eprintln!("warn: skipping non-existent watch dir: {}", dir.display());
                continue;
            }
            if let Err(e) = watcher.watch(dir, RecursiveMode::Recursive) {
                eprintln!("warn: failed to watch {}: {e}", dir.display());
            }
        }

        let thread = thread::spawn(move || {
            while let Ok(res) = notify_rx.recv() {
                match res {
                    Ok(event) => {
                        if !matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                            continue;
                        }

                        for path in event.paths {
                            if !is_watched_file(&path) {
                                continue;
                            }
                            if tx.send(WatchEvent::FileChanged(path)).is_err() {
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("warn: file watcher error: {e}");
                    }
                }
            }
        });

        Ok(Self {
            _watcher: Box::new(watcher),
            _thread: thread,
        })
    }
}

fn is_watched_file(path: &std::path::Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("jsonl" | "yaml" | "yml")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;
    use tempfile::TempDir;

    #[test]
    fn is_watched_file_accepts_jsonl() {
        assert!(is_watched_file(&PathBuf::from("events.jsonl")));
    }

    #[test]
    fn is_watched_file_accepts_yaml() {
        assert!(is_watched_file(&PathBuf::from("config.yaml")));
        assert!(is_watched_file(&PathBuf::from("config.yml")));
    }

    #[test]
    fn is_watched_file_rejects_lock() {
        assert!(!is_watched_file(&PathBuf::from("file.lock")));
    }

    #[test]
    fn is_watched_file_rejects_json() {
        assert!(!is_watched_file(&PathBuf::from("data.json")));
    }

    #[test]
    fn watcher_detects_jsonl_changes() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test.jsonl");
        fs::write(&file, "").unwrap();

        let (tx, rx) = mpsc::channel();
        let _watcher = Watcher::start(vec![dir.path().to_path_buf()], tx).unwrap();

        // Give the watcher time to register
        thread::sleep(Duration::from_millis(200));

        // Trigger a write
        fs::write(&file, "new content\n").unwrap();

        // Wait for the event (with timeout)
        let event = rx.recv_timeout(Duration::from_secs(5));
        assert!(
            event.is_ok(),
            "expected a FileChanged event for .jsonl write"
        );

        if let Ok(WatchEvent::FileChanged(path)) = event {
            // macOS resolves /var -> /private/var, so canonicalize both
            assert_eq!(path.canonicalize().unwrap(), file.canonicalize().unwrap());
        }
    }

    #[test]
    fn watcher_ignores_non_watched_extensions() {
        let dir = TempDir::new().unwrap();
        let lock_file = dir.path().join("test.lock");
        let json_file = dir.path().join("test.json");
        fs::write(&lock_file, "").unwrap();
        fs::write(&json_file, "").unwrap();

        let (tx, rx) = mpsc::channel();
        let _watcher = Watcher::start(vec![dir.path().to_path_buf()], tx).unwrap();

        thread::sleep(Duration::from_millis(200));

        fs::write(&lock_file, "lock content\n").unwrap();
        fs::write(&json_file, "json content\n").unwrap();

        // Should not receive events for .lock or .json
        let event = rx.recv_timeout(Duration::from_secs(1));
        assert!(
            event.is_err(),
            "should not receive events for non-watched file types"
        );
    }

    #[test]
    fn watcher_skips_nonexistent_dirs() {
        let (tx, _rx) = mpsc::channel();
        let result = Watcher::start(
            vec![PathBuf::from("/nonexistent/dir/that/does/not/exist")],
            tx,
        );
        // Should succeed (skips the dir with a warning, doesn't crash)
        assert!(result.is_ok());
    }
}
