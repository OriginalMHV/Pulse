mod feed;
mod sessions;
mod stats;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Tabs};

use crate::app::{App, DetailTab, ViewMode};

// ── Color Palette ───────────────────────────────────────────────

pub const PULSE_GREEN: Color = Color::Rgb(0, 230, 118);
pub const AMBER: Color = Color::Rgb(255, 179, 0);
pub const ALERT_RED: Color = Color::Rgb(255, 82, 82);
pub const DIM_GRAY: Color = Color::Rgb(100, 100, 100);
pub const PANEL_BORDER: Color = Color::Rgb(60, 60, 60);
pub const COPILOT_PURPLE: Color = Color::Rgb(187, 134, 252);
pub const CLAUDE_AMBER: Color = Color::Rgb(255, 179, 0);
pub const CODEX_TEAL: Color = Color::Rgb(0, 188, 212);
pub const CYAN: Color = Color::Rgb(0, 229, 255);
pub const HIGHLIGHT_BG: Color = Color::Rgb(30, 30, 60);

// ── Main Draw ───────────────────────────────────────────────────

pub fn draw(frame: &mut Frame, app: &App) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(frame.area());

    render_header(frame, app, outer[0]);

    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(outer[1]);

    sessions::render(frame, app, main[0]);
    render_detail(frame, app, main[1]);
    stats::render_stats_bar(frame, app, outer[2]);
}

// ── Header ──────────────────────────────────────────────────────

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let sessions = app.sessions();
    let active = sessions.iter().filter(|s| s.is_active).count();
    let monitored = sessions.len();

    let elapsed = app.started_at.elapsed();
    let hours = elapsed.as_secs() / 3600;
    let minutes = (elapsed.as_secs() % 3600) / 60;
    let uptime = if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    };

    let filter_label = app.provider_filter.label();

    let search_indicator = if app.view_mode == ViewMode::Search {
        format!("  /{}\u{23F1}", app.search_query)
    } else if !app.search_query.is_empty() {
        format!("  [{}]", app.search_query)
    } else {
        String::new()
    };

    let info = Line::from(vec![
        Span::styled(" Active: ", Style::default().fg(DIM_GRAY)),
        Span::styled(format!("{active}"), Style::default().fg(PULSE_GREEN)),
        Span::styled("  \u{2502}  Monitored: ", Style::default().fg(DIM_GRAY)),
        Span::styled(format!("{monitored}"), Style::default().fg(Color::White)),
        Span::styled("  \u{2502}  Filter: ", Style::default().fg(DIM_GRAY)),
        Span::styled(filter_label, Style::default().fg(AMBER)),
        Span::styled("  \u{2502}  Uptime: ", Style::default().fg(DIM_GRAY)),
        Span::styled(uptime, Style::default().fg(Color::White)),
        Span::styled(search_indicator, Style::default().fg(PULSE_GREEN)),
    ]);

    let block = Block::bordered()
        .title(Line::from(vec![
            Span::styled(" \u{26A1}", Style::default().fg(AMBER)),
            Span::styled(
                "Pulse ",
                Style::default()
                    .fg(PULSE_GREEN)
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
        .border_style(Style::default().fg(PANEL_BORDER));

    let header = Paragraph::new(info).block(block);
    frame.render_widget(header, area);
}

// ── Detail Panel ────────────────────────────────────────────────

fn render_detail(frame: &mut Frame, app: &App, area: Rect) {
    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(area);

    let tabs = Tabs::new(vec!["Feed", "Tools", "Files"])
        .select(app.detail_tab.index())
        .style(Style::default().fg(DIM_GRAY))
        .highlight_style(
            Style::default()
                .fg(PULSE_GREEN)
                .add_modifier(Modifier::BOLD),
        )
        .divider("\u{2502}")
        .block(
            Block::bordered()
                .border_style(Style::default().fg(PANEL_BORDER))
                .title_bottom(Line::from(vec![Span::styled(
                    " Tab to switch ",
                    Style::default().fg(DIM_GRAY),
                )])),
        );
    frame.render_widget(tabs, inner[0]);

    match app.detail_tab {
        DetailTab::Feed => feed::render(frame, app, inner[1]),
        DetailTab::Tools => stats::render_tools_tab(frame, app, inner[1]),
        DetailTab::Files => stats::render_files_tab(frame, app, inner[1]),
    }
}
