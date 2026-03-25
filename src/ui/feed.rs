use chrono::Local;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};

use crate::app::App;
use crate::event::PulseEvent;

use super::{ALERT_RED, AMBER, CYAN, DIM_GRAY, PANEL_BORDER, PULSE_GREEN};

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
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

    let lines: Vec<Line> = session
        .events
        .iter()
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
                Span::styled(format!("{label:<18}"), Style::default().fg(label_color)),
                Span::styled(format!(" {summary}"), Style::default().fg(Color::White)),
            ])
        })
        .collect();

    let visible_height = area.height.saturating_sub(2) as usize; // account for borders
    let max_scroll = lines.len().saturating_sub(visible_height);
    let scroll = app.feed_scroll.min(max_scroll) as u16;

    let feed = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(feed, area);
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
