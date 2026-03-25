use chrono::Local;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};

use crate::app::App;
use crate::event::PulseEvent;
use crate::metrics;
use crate::session::Session;

use super::{
    ALERT_RED, AMBER, CLAUDE_AMBER, CODEX_TEAL, COPILOT_PURPLE, CYAN, DIM_GRAY, PANEL_BORDER,
    PULSE_GREEN,
};

/// Compact single-session view. Designed for tmux side panes and focused monitoring.
pub fn draw(frame: &mut Frame, app: &App) {
    let session = match app.focus_session() {
        Some(s) => s,
        None => {
            render_no_session(frame);
            return;
        }
    };

    let area = frame.area();
    let is_narrow = area.width < 50;

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Length(3), // metrics bar
            Constraint::Min(6),    // event feed
            Constraint::Length(3), // tool summary
        ])
        .split(area);

    render_focus_header(frame, session, layout[0], is_narrow);
    render_metrics_bar(frame, session, layout[1]);
    render_live_feed(frame, app, session, layout[2]);
    render_tool_summary(frame, session, layout[3]);
}

fn render_no_session(frame: &mut Frame) {
    let block = Block::bordered()
        .title(Line::from(vec![
            Span::styled(
                " Pulse ",
                Style::default()
                    .fg(PULSE_GREEN)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Focus ", Style::default().fg(AMBER)),
        ]))
        .border_style(Style::default().fg(PANEL_BORDER));

    let msg = Paragraph::new(Line::from(vec![Span::styled(
        " Waiting for active session...",
        Style::default().fg(DIM_GRAY),
    )]))
    .block(block);

    frame.render_widget(msg, frame.area());
}

fn render_focus_header(frame: &mut Frame, session: &Session, area: Rect, is_narrow: bool) {
    let provider_color = match session.provider {
        crate::event::Provider::Copilot => COPILOT_PURPLE,
        crate::event::Provider::Claude => CLAUDE_AMBER,
        crate::event::Provider::Codex => CODEX_TEAL,
    };

    let active_indicator = if session.is_active {
        Span::styled("● ", Style::default().fg(PULSE_GREEN))
    } else {
        Span::styled("○ ", Style::default().fg(DIM_GRAY))
    };

    let duration = crate::session::human_duration(session.duration());
    let model_label = session.model.as_deref().unwrap_or("unknown");

    let cwd = crate::session::shorten_path(&session.cwd);
    let max_cwd = if is_narrow { 20 } else { 40 };
    let cwd_display = if cwd.chars().count() > max_cwd {
        let mut s: String = cwd.chars().take(max_cwd - 1).collect();
        s.push('…');
        s
    } else {
        cwd
    };

    let info = Line::from(vec![
        Span::styled(" ", Style::default()),
        active_indicator,
        Span::styled(
            session.provider.short_label(),
            Style::default()
                .fg(provider_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" | ", Style::default().fg(DIM_GRAY)),
        Span::styled(cwd_display, Style::default().fg(Color::White)),
        Span::styled(" | ", Style::default().fg(DIM_GRAY)),
        Span::styled(duration, Style::default().fg(Color::White)),
        Span::styled(" | ", Style::default().fg(DIM_GRAY)),
        Span::styled(model_label, Style::default().fg(DIM_GRAY)),
    ]);

    let block = Block::bordered()
        .title(Line::from(vec![
            Span::styled(
                " Pulse ",
                Style::default()
                    .fg(PULSE_GREEN)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Focus ", Style::default().fg(AMBER)),
        ]))
        .border_style(Style::default().fg(PANEL_BORDER));

    let header = Paragraph::new(info).block(block);
    frame.render_widget(header, area);
}

fn render_metrics_bar(frame: &mut Frame, session: &Session, area: Rect) {
    let in_tok = metrics::format_tokens(session.tokens.input);
    let out_tok = metrics::format_tokens(session.tokens.output);
    let cache_rate = metrics::cache_hit_rate(&session.tokens);
    let tools = session.total_tool_calls();
    let files = session.files.len();
    let cost = metrics::format_cost(session.estimated_cost());

    let info = Line::from(vec![
        Span::styled(" ", Style::default()),
        Span::styled(format!("↓{in_tok}"), Style::default().fg(AMBER)),
        Span::styled(" ", Style::default()),
        Span::styled(format!("↑{out_tok}"), Style::default().fg(PULSE_GREEN)),
        Span::styled("  Cache: ", Style::default().fg(DIM_GRAY)),
        Span::styled(
            format!("{cache_rate:.0}%"),
            Style::default().fg(Color::White),
        ),
        Span::styled("  Tools: ", Style::default().fg(DIM_GRAY)),
        Span::styled(format!("{tools}"), Style::default().fg(Color::White)),
        Span::styled("  Files: ", Style::default().fg(DIM_GRAY)),
        Span::styled(format!("{files}"), Style::default().fg(Color::White)),
        Span::styled("  ", Style::default()),
        Span::styled(cost, Style::default().fg(AMBER)),
    ]);

    let block = Block::bordered().border_style(Style::default().fg(PANEL_BORDER));

    let bar = Paragraph::new(info).block(block);
    frame.render_widget(bar, area);
}

fn render_live_feed(frame: &mut Frame, app: &App, session: &Session, area: Rect) {
    let block = Block::bordered()
        .title(Line::from(Span::styled(
            " Live ",
            Style::default().fg(PULSE_GREEN),
        )))
        .border_style(Style::default().fg(PANEL_BORDER));

    let visible_height = area.height.saturating_sub(2) as usize;

    let lines: Vec<Line> = session
        .events
        .iter()
        .filter(|e| !matches!(e.event, PulseEvent::Progress { .. }))
        .map(|ts_event| {
            let time = ts_event
                .timestamp
                .with_timezone(&Local)
                .format("%H:%M:%S")
                .to_string();

            let label = ts_event.event.label();
            let label_color = event_color(&ts_event.event);
            let summary = ts_event.event.summary();

            Line::from(vec![
                Span::styled(format!("{time} "), Style::default().fg(DIM_GRAY)),
                Span::styled(format!("{label:<16}"), Style::default().fg(label_color)),
                Span::styled(format!(" {summary}"), Style::default().fg(Color::White)),
            ])
        })
        .collect();

    // Auto-scroll to bottom unless user has scrolled up
    let total_lines = lines.len();
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll = if app.focus_auto_scroll {
        max_scroll
    } else {
        app.feed_scroll.min(max_scroll)
    };

    let feed = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((scroll as u16, 0));

    frame.render_widget(feed, area);
}

fn render_tool_summary(frame: &mut Frame, session: &Session, area: Rect) {
    let mut tools: Vec<_> = session.tool_calls.iter().collect();
    tools.sort_by(|a, b| b.1.calls.cmp(&a.1.calls));

    let top_tools: Vec<Span> = tools
        .iter()
        .take(6)
        .flat_map(|(name, stats)| {
            let fail_count = stats.failures;
            let mut spans = vec![
                Span::styled(name.to_string(), Style::default().fg(Color::White)),
                Span::styled(format!("({})", stats.calls), Style::default().fg(AMBER)),
            ];
            if fail_count > 0 {
                spans.push(Span::styled(
                    format!("[{fail_count}!]"),
                    Style::default().fg(ALERT_RED),
                ));
            }
            spans.push(Span::styled("  ", Style::default()));
            spans
        })
        .collect();

    let mut line_spans = vec![Span::styled(" ", Style::default())];
    line_spans.extend(top_tools);

    let block = Block::bordered()
        .title(Line::from(Span::styled(
            " Tools ",
            Style::default().fg(DIM_GRAY),
        )))
        .border_style(Style::default().fg(PANEL_BORDER));

    let bar = Paragraph::new(Line::from(line_spans)).block(block);
    frame.render_widget(bar, area);
}

fn event_color(event: &PulseEvent) -> Color {
    match event {
        PulseEvent::UserMessage { .. } => PULSE_GREEN,
        PulseEvent::AssistantMessage { .. } => CYAN,
        PulseEvent::ToolStart { .. } => AMBER,
        PulseEvent::ToolComplete { success, .. } => {
            if *success {
                AMBER
            } else {
                ALERT_RED
            }
        }
        PulseEvent::Warning { .. } => ALERT_RED,
        PulseEvent::TurnStart { .. } | PulseEvent::TurnEnd { .. } => DIM_GRAY,
        PulseEvent::Progress { .. } => DIM_GRAY,
        PulseEvent::SessionStart { .. } => PULSE_GREEN,
    }
}
