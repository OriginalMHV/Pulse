use std::sync::mpsc;
use std::time::Instant;

use crate::event::Provider;
use crate::scanner::Scanner;
use crate::session::Session;
use crate::watcher::WatchEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Normal,
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    Feed,
    Tools,
    Files,
}

impl DetailTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Feed => "Feed",
            Self::Tools => "Tools",
            Self::Files => "Files",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::Feed => 0,
            Self::Tools => 1,
            Self::Files => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFilter {
    All,
    Copilot,
    Claude,
    Codex,
}

impl ProviderFilter {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Copilot => "Copilot",
            Self::Claude => "Claude",
            Self::Codex => "Codex",
        }
    }
}

pub struct App {
    scanner: Scanner,
    rx: mpsc::Receiver<WatchEvent>,
    pub started_at: Instant,
    pub selected: usize,
    pub feed_scroll: usize,
    pub detail_tab: DetailTab,
    pub provider_filter: ProviderFilter,
    pub view_mode: ViewMode,
    pub search_query: String,
    filtered_indices: Vec<usize>,
}

impl App {
    pub fn new(scanner: Scanner, rx: mpsc::Receiver<WatchEvent>) -> Self {
        let mut app = Self {
            scanner,
            rx,
            started_at: Instant::now(),
            selected: 0,
            feed_scroll: 0,
            detail_tab: DetailTab::Feed,
            provider_filter: ProviderFilter::All,
            view_mode: ViewMode::Normal,
            search_query: String::new(),
            filtered_indices: Vec::new(),
        };
        app.apply_filter();
        app
    }

    pub fn mode(&self) -> ViewMode {
        self.view_mode
    }

    // ── Navigation ──────────────────────────────────────────────

    pub fn select_next(&mut self) {
        if !self.filtered_indices.is_empty() {
            self.selected = (self.selected + 1).min(self.filtered_indices.len() - 1);
            self.feed_scroll = 0;
        }
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        self.feed_scroll = 0;
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
        self.feed_scroll = 0;
    }

    pub fn select_last(&mut self) {
        if !self.filtered_indices.is_empty() {
            self.selected = self.filtered_indices.len() - 1;
        }
        self.feed_scroll = 0;
    }

    // ── Tabs & Filters ─────────────────────────────────────────

    pub fn cycle_tab(&mut self) {
        self.detail_tab = match self.detail_tab {
            DetailTab::Feed => DetailTab::Tools,
            DetailTab::Tools => DetailTab::Files,
            DetailTab::Files => DetailTab::Feed,
        };
    }

    pub fn cycle_provider_filter(&mut self) {
        self.provider_filter = match self.provider_filter {
            ProviderFilter::All => ProviderFilter::Copilot,
            ProviderFilter::Copilot => ProviderFilter::Claude,
            ProviderFilter::Claude => ProviderFilter::Codex,
            ProviderFilter::Codex => ProviderFilter::All,
        };
        self.apply_filter();
    }

    // ── Search ──────────────────────────────────────────────────

    pub fn enter_search(&mut self) {
        self.view_mode = ViewMode::Search;
    }

    pub fn exit_search(&mut self) {
        self.view_mode = ViewMode::Normal;
    }

    pub fn search_input(&mut self, c: char) {
        self.search_query.push(c);
        self.apply_filter();
    }

    pub fn search_backspace(&mut self) {
        self.search_query.pop();
        self.apply_filter();
    }

    pub fn clear_search(&mut self) {
        self.search_query.clear();
        self.apply_filter();
    }

    // ── Feed Scroll ─────────────────────────────────────────────

    pub fn scroll_feed_up(&mut self) {
        self.feed_scroll = self.feed_scroll.saturating_sub(1);
    }

    pub fn scroll_feed_down(&mut self) {
        self.feed_scroll += 1;
    }

    // ── Data ────────────────────────────────────────────────────

    pub fn refresh(&mut self) {
        self.scanner.scan_all();
        self.apply_filter();
    }

    pub fn poll_events(&mut self) {
        let mut paths = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            match event {
                WatchEvent::FileChanged(path) => paths.push(path),
            }
        }
        if !paths.is_empty() {
            for path in paths {
                self.scanner.update_session(&path);
            }
            self.apply_filter();
        }
    }

    pub fn sessions(&self) -> Vec<&Session> {
        self.filtered_indices
            .iter()
            .filter_map(|&i| self.scanner.sessions().get(i))
            .collect()
    }

    pub fn selected_session(&self) -> Option<&Session> {
        self.filtered_indices
            .get(self.selected)
            .and_then(|&i| self.scanner.sessions().get(i))
    }

    pub fn apply_filter(&mut self) {
        let sessions = self.scanner.sessions();
        self.filtered_indices = sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| self.matches_provider(s) && self.matches_search(s))
            .map(|(i, _)| i)
            .collect();

        if self.filtered_indices.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.filtered_indices.len() {
            self.selected = self.filtered_indices.len() - 1;
        }
    }

    fn matches_provider(&self, session: &Session) -> bool {
        match self.provider_filter {
            ProviderFilter::All => true,
            ProviderFilter::Copilot => session.provider == Provider::Copilot,
            ProviderFilter::Claude => session.provider == Provider::Claude,
            ProviderFilter::Codex => session.provider == Provider::Codex,
        }
    }

    fn matches_search(&self, session: &Session) -> bool {
        if self.search_query.is_empty() {
            return true;
        }
        let q = self.search_query.to_lowercase();
        session.cwd.to_lowercase().contains(&q)
            || session.summary.to_lowercase().contains(&q)
            || session.provider.short_label().to_lowercase().contains(&q)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_session(id: &str, provider: Provider, cwd: &str) -> Session {
        let mut s = Session::new(id.into(), provider, cwd.into(), Utc::now());
        s.summary = format!("{id} summary");
        s
    }

    fn make_app(sessions: Vec<Session>) -> App {
        let scanner = Scanner::with_sessions(sessions);
        let (_tx, rx) = mpsc::channel();
        App::new(scanner, rx)
    }

    #[test]
    fn initial_state() {
        let app = make_app(vec![
            make_session("s1", Provider::Copilot, "/a"),
            make_session("s2", Provider::Claude, "/b"),
        ]);
        assert_eq!(app.mode(), ViewMode::Normal);
        assert_eq!(app.selected, 0);
        assert_eq!(app.detail_tab, DetailTab::Feed);
        assert_eq!(app.provider_filter, ProviderFilter::All);
        assert_eq!(app.sessions().len(), 2);
    }

    #[test]
    fn navigation_next_prev_bounds() {
        let mut app = make_app(vec![
            make_session("s1", Provider::Copilot, "/a"),
            make_session("s2", Provider::Claude, "/b"),
            make_session("s3", Provider::Codex, "/c"),
        ]);

        app.select_next();
        assert_eq!(app.selected, 1);
        app.select_next();
        assert_eq!(app.selected, 2);
        app.select_next();
        assert_eq!(app.selected, 2);

        app.select_prev();
        assert_eq!(app.selected, 1);
        app.select_prev();
        assert_eq!(app.selected, 0);
        app.select_prev();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn select_first_last() {
        let mut app = make_app(vec![
            make_session("s1", Provider::Copilot, "/a"),
            make_session("s2", Provider::Claude, "/b"),
            make_session("s3", Provider::Codex, "/c"),
        ]);

        app.select_last();
        assert_eq!(app.selected, 2);
        app.select_first();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn cycle_tab() {
        let mut app = make_app(vec![]);
        assert_eq!(app.detail_tab, DetailTab::Feed);
        app.cycle_tab();
        assert_eq!(app.detail_tab, DetailTab::Tools);
        app.cycle_tab();
        assert_eq!(app.detail_tab, DetailTab::Files);
        app.cycle_tab();
        assert_eq!(app.detail_tab, DetailTab::Feed);
    }

    #[test]
    fn provider_filter_cycle_and_filtering() {
        let mut app = make_app(vec![
            make_session("s1", Provider::Copilot, "/a"),
            make_session("s2", Provider::Claude, "/b"),
            make_session("s3", Provider::Codex, "/c"),
        ]);

        assert_eq!(app.sessions().len(), 3);

        app.cycle_provider_filter();
        assert_eq!(app.provider_filter, ProviderFilter::Copilot);
        assert_eq!(app.sessions().len(), 1);
        assert_eq!(app.sessions()[0].provider, Provider::Copilot);

        app.cycle_provider_filter();
        assert_eq!(app.provider_filter, ProviderFilter::Claude);
        assert_eq!(app.sessions().len(), 1);
        assert_eq!(app.sessions()[0].provider, Provider::Claude);

        app.cycle_provider_filter();
        assert_eq!(app.provider_filter, ProviderFilter::Codex);
        assert_eq!(app.sessions().len(), 1);

        app.cycle_provider_filter();
        assert_eq!(app.provider_filter, ProviderFilter::All);
        assert_eq!(app.sessions().len(), 3);
    }

    #[test]
    fn search_mode_and_filtering() {
        let mut app = make_app(vec![
            make_session("s1", Provider::Copilot, "/home/user/project-alpha"),
            make_session("s2", Provider::Claude, "/home/user/project-beta"),
        ]);

        app.enter_search();
        assert_eq!(app.mode(), ViewMode::Search);

        app.search_input('a');
        app.search_input('l');
        app.search_input('p');
        app.search_input('h');
        app.search_input('a');
        assert_eq!(app.search_query, "alpha");
        assert_eq!(app.sessions().len(), 1);
        assert_eq!(app.sessions()[0].cwd, "/home/user/project-alpha");

        app.search_backspace();
        assert_eq!(app.search_query, "alph");

        app.clear_search();
        assert_eq!(app.search_query, "");
        assert_eq!(app.sessions().len(), 2);

        app.exit_search();
        assert_eq!(app.mode(), ViewMode::Normal);
    }

    #[test]
    fn selected_session_returns_correct_session() {
        let mut app = make_app(vec![
            make_session("s1", Provider::Copilot, "/a"),
            make_session("s2", Provider::Claude, "/b"),
        ]);

        assert_eq!(app.selected_session().unwrap().id, "s1");
        app.select_next();
        assert_eq!(app.selected_session().unwrap().id, "s2");
    }

    #[test]
    fn empty_sessions_navigation_is_safe() {
        let mut app = make_app(vec![]);
        assert_eq!(app.sessions().len(), 0);
        assert!(app.selected_session().is_none());

        app.select_next();
        app.select_prev();
        app.select_first();
        app.select_last();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn filter_clamps_selection() {
        let mut app = make_app(vec![
            make_session("s1", Provider::Copilot, "/a"),
            make_session("s2", Provider::Claude, "/b"),
            make_session("s3", Provider::Codex, "/c"),
        ]);

        app.select_last();
        assert_eq!(app.selected, 2);

        app.cycle_provider_filter();
        assert_eq!(app.provider_filter, ProviderFilter::Copilot);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn scroll_feed() {
        let mut app = make_app(vec![]);
        assert_eq!(app.feed_scroll, 0);

        app.scroll_feed_down();
        app.scroll_feed_down();
        assert_eq!(app.feed_scroll, 2);

        app.scroll_feed_up();
        assert_eq!(app.feed_scroll, 1);

        app.scroll_feed_up();
        app.scroll_feed_up();
        assert_eq!(app.feed_scroll, 0);
    }
}
