use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table};

use crate::app::App;
use crate::event::TokenUsage;
use crate::metrics;
use crate::session::shorten_path;

use super::{AMBER, DIM_GRAY, PANEL_BORDER, PULSE_GREEN};

// ── Stats Bar ───────────────────────────────────────────────────

pub fn render_stats_bar(frame: &mut Frame, app: &App, area: Rect) {
    let sessions = app.sessions();

    let mut total_tokens = TokenUsage::default();
    let mut total_tools: u32 = 0;
    let mut total_files: usize = 0;
    let mut total_cost: f64 = 0.0;

    for s in &sessions {
        total_tokens.merge(&s.tokens);
        total_tools += s.total_tool_calls();
        total_files += s.files.len();
        total_cost += s.estimated_cost();
    }

    let cache_rate = metrics::cache_hit_rate(&total_tokens);
    let in_tok = metrics::format_tokens(total_tokens.input);
    let out_tok = metrics::format_tokens(total_tokens.output);
    let cost = metrics::format_cost(total_cost);

    let info = Line::from(vec![
        Span::styled(" Tokens: ", Style::default().fg(DIM_GRAY)),
        Span::styled(format!("↓{in_tok}"), Style::default().fg(AMBER)),
        Span::styled(" ", Style::default()),
        Span::styled(format!("↑{out_tok}"), Style::default().fg(PULSE_GREEN)),
        Span::styled("  │  Cache: ", Style::default().fg(DIM_GRAY)),
        Span::styled(
            format!("{cache_rate:.0}%"),
            Style::default().fg(Color::White),
        ),
        Span::styled("  │  Tools: ", Style::default().fg(DIM_GRAY)),
        Span::styled(format!("{total_tools}"), Style::default().fg(Color::White)),
        Span::styled("  │  Files: ", Style::default().fg(DIM_GRAY)),
        Span::styled(format!("{total_files}"), Style::default().fg(Color::White)),
        Span::styled("  │  ", Style::default().fg(DIM_GRAY)),
        Span::styled(cost, Style::default().fg(AMBER)),
    ]);

    let block = Block::bordered()
        .title(Line::from(Span::styled(
            " Aggregate ",
            Style::default().fg(DIM_GRAY),
        )))
        .border_style(Style::default().fg(PANEL_BORDER));

    let bar = Paragraph::new(info).block(block);
    frame.render_widget(bar, area);
}

// ── Tools Tab ───────────────────────────────────────────────────

pub fn render_tools_tab(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::bordered().border_style(Style::default().fg(PANEL_BORDER));

    let Some(session) = app.selected_session() else {
        let empty = Paragraph::new(Line::from(Span::styled(
            " No session selected",
            Style::default().fg(DIM_GRAY),
        )))
        .block(block);
        frame.render_widget(empty, area);
        return;
    };

    let header = Row::new(vec![
        Cell::from("Tool Name"),
        Cell::from("Calls"),
        Cell::from("Success"),
        Cell::from("Fail"),
        Cell::from("Avg (ms)"),
        Cell::from("Last Used"),
    ])
    .style(
        Style::default()
            .fg(PULSE_GREEN)
            .add_modifier(Modifier::BOLD),
    );

    let mut tools: Vec<_> = session.tool_calls.iter().collect();
    tools.sort_by(|a, b| b.1.calls.cmp(&a.1.calls));

    let rows: Vec<Row> = tools
        .iter()
        .map(|(name, stats)| {
            let avg_ms = if stats.calls > 0 {
                stats.total_duration_ms / stats.calls as u64
            } else {
                0
            };
            let last = stats
                .last_used
                .map(|t| {
                    t.with_timezone(&chrono::Local)
                        .format("%H:%M:%S")
                        .to_string()
                })
                .unwrap_or_else(|| "—".into());

            let fail_style = if stats.failures > 0 {
                Style::default().fg(super::ALERT_RED)
            } else {
                Style::default().fg(DIM_GRAY)
            };

            Row::new(vec![
                Cell::from(name.as_str().to_string()).style(Style::default().fg(Color::White)),
                Cell::from(format!("{}", stats.calls)).style(Style::default().fg(AMBER)),
                Cell::from(format!("{}", stats.successes)).style(Style::default().fg(PULSE_GREEN)),
                Cell::from(format!("{}", stats.failures)).style(fail_style),
                Cell::from(format!("{avg_ms}")).style(Style::default().fg(DIM_GRAY)),
                Cell::from(last).style(Style::default().fg(DIM_GRAY)),
            ])
        })
        .collect();

    let widths = [
        Constraint::Min(16),
        Constraint::Length(7),
        Constraint::Length(9),
        Constraint::Length(6),
        Constraint::Length(10),
        Constraint::Length(12),
    ];

    let table = Table::new(rows, widths).header(header).block(block);
    frame.render_widget(table, area);
}

// ── Files Tab ───────────────────────────────────────────────────

pub fn render_files_tab(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::bordered().border_style(Style::default().fg(PANEL_BORDER));

    let Some(session) = app.selected_session() else {
        let empty = Paragraph::new(Line::from(Span::styled(
            " No session selected",
            Style::default().fg(DIM_GRAY),
        )))
        .block(block);
        frame.render_widget(empty, area);
        return;
    };

    let header = Row::new(vec![
        Cell::from("Path"),
        Cell::from("Reads"),
        Cell::from("Writes"),
        Cell::from("Last Op"),
    ])
    .style(
        Style::default()
            .fg(PULSE_GREEN)
            .add_modifier(Modifier::BOLD),
    );

    let mut files: Vec<_> = session.files.iter().collect();
    files.sort_by(|a, b| {
        let total_b = b.1.reads + b.1.writes;
        let total_a = a.1.reads + a.1.writes;
        total_b.cmp(&total_a)
    });

    let rows: Vec<Row> = files
        .iter()
        .map(|(path, stats)| {
            let short = shorten_path(path);
            let last_op = stats.last_op.as_deref().unwrap_or("—");

            Row::new(vec![
                Cell::from(short).style(Style::default().fg(Color::White)),
                Cell::from(format!("{}", stats.reads)).style(Style::default().fg(AMBER)),
                Cell::from(format!("{}", stats.writes)).style(Style::default().fg(PULSE_GREEN)),
                Cell::from(last_op.to_string()).style(Style::default().fg(DIM_GRAY)),
            ])
        })
        .collect();

    let widths = [
        Constraint::Min(30),
        Constraint::Length(7),
        Constraint::Length(8),
        Constraint::Length(10),
    ];

    let table = Table::new(rows, widths).header(header).block(block);
    frame.render_widget(table, area);
}
