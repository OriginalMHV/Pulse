use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState};

use crate::app::App;
use crate::event::Provider;
use crate::metrics;
use crate::session::Session;

use super::{
    AMBER, CLAUDE_AMBER, CODEX_TEAL, COPILOT_PURPLE, DIM_GRAY, HIGHLIGHT_BG, PANEL_BORDER,
    PULSE_GREEN,
};

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let sessions = app.sessions();

    let items: Vec<ListItem> = sessions.iter().map(|s| session_item(s)).collect();

    let count = sessions.len();
    let block = Block::bordered()
        .title(Line::from(vec![
            Span::styled(" Sessions ", Style::default().fg(Color::White)),
            Span::styled(format!("({count}) "), Style::default().fg(DIM_GRAY)),
        ]))
        .border_style(Style::default().fg(PANEL_BORDER));

    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .bg(HIGHLIGHT_BG)
            .add_modifier(Modifier::BOLD),
    );

    let mut state = ListState::default().with_selected(Some(app.selected));
    frame.render_stateful_widget(list, area, &mut state);
}

fn session_item(session: &Session) -> ListItem<'static> {
    let (indicator, indicator_color) = if session.is_active {
        ("●", PULSE_GREEN)
    } else {
        ("○", DIM_GRAY)
    };

    let provider_color = provider_badge_color(session.provider);
    let provider_label = session.provider.short_label();

    let summary = if session.summary.is_empty() {
        truncate_cwd(&session.cwd, 30)
    } else {
        truncate_str(&session.summary, 30)
    };

    let age = session.age_label();

    let in_tok = metrics::format_tokens(session.tokens.input);
    let out_tok = metrics::format_tokens(session.tokens.output);
    let cost = metrics::format_cost(session.estimated_cost());

    let line1 = Line::from(vec![
        Span::styled(
            format!("{indicator} "),
            Style::default().fg(indicator_color),
        ),
        Span::styled(
            format!("{provider_label:<7} "),
            Style::default().fg(provider_color),
        ),
        Span::styled(summary, Style::default().fg(Color::White)),
    ]);

    let line2 = Line::from(vec![
        Span::styled(format!("  {age:>6}  "), Style::default().fg(DIM_GRAY)),
        Span::styled(format!("↓{in_tok}"), Style::default().fg(AMBER)),
        Span::styled(" ", Style::default()),
        Span::styled(format!("↑{out_tok}"), Style::default().fg(PULSE_GREEN)),
        Span::styled(format!("  {cost}"), Style::default().fg(DIM_GRAY)),
    ]);

    ListItem::new(vec![line1, line2])
}

fn provider_badge_color(provider: Provider) -> Color {
    match provider {
        Provider::Copilot => COPILOT_PURPLE,
        Provider::Claude => CLAUDE_AMBER,
        Provider::Codex => CODEX_TEAL,
    }
}

fn truncate_cwd(path: &str, max: usize) -> String {
    let shortened = crate::session::shorten_path(path);
    truncate_str(&shortened, max)
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut result: String = s.chars().take(max - 1).collect();
        result.push('…');
        result
    }
}
