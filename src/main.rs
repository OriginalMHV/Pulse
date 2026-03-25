use std::io;
use std::sync::mpsc;
use std::time::Duration;

use clap::Parser;
use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use pulse::app::App;
use pulse::providers::SessionProvider;
use pulse::providers::claude::ClaudeProvider;
use pulse::providers::codex::CodexProvider;
use pulse::providers::copilot::CopilotProvider;
use pulse::scanner::Scanner;
use pulse::ui;
use pulse::watcher::Watcher;

#[derive(Parser)]
#[command(
    name = "pulse",
    about = "htop for AI coding sessions. Real-time monitoring for Copilot CLI, Claude Code, and Codex CLI."
)]
struct Cli {
    /// List sessions as plain text (no TUI)
    #[arg(long)]
    list: bool,

    /// Show aggregate token/cost statistics
    #[arg(long)]
    stats: bool,

    /// Filter by provider name (copilot, claude, codex)
    #[arg(long)]
    provider: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let providers: Vec<Box<dyn SessionProvider>> = vec![
        Box::new(CopilotProvider),
        Box::new(ClaudeProvider),
        Box::new(CodexProvider),
    ];

    let mut scanner = Scanner::new(providers);
    scanner.scan_all();

    if let Some(ref filter) = cli.provider {
        scanner.filter_provider(filter);
    }

    if cli.stats {
        print_stats(&scanner);
        return Ok(());
    }

    if cli.list {
        print_list(&scanner);
        return Ok(());
    }

    if scanner.sessions().is_empty() {
        println!("No AI coding sessions found.");
        println!("Supported: Copilot CLI, Claude Code, Codex CLI");
        return Ok(());
    }

    run_tui(scanner)
}

fn print_list(scanner: &Scanner) {
    for session in scanner.sessions() {
        let age = session.age_label();
        let tools = session.total_tool_calls();
        let cost = pulse::metrics::format_cost(session.estimated_cost());
        let in_tok = pulse::metrics::format_tokens(session.tokens.input);
        let out_tok = pulse::metrics::format_tokens(session.tokens.output);
        let active = if session.is_active { "●" } else { "○" };
        let summary = if session.summary.is_empty() {
            session.cwd.clone()
        } else {
            session.summary.clone()
        };
        println!(
            "{active} {:<8} {age:>6} | {summary:<40} | ↓{in_tok} ↑{out_tok} | {tools} tools | {cost}",
            session.provider.short_label()
        );
    }
}

fn print_stats(scanner: &Scanner) {
    let sessions = scanner.sessions();
    let active = sessions.iter().filter(|s| s.is_active).count();
    let total_input: u64 = sessions.iter().map(|s| s.tokens.input).sum();
    let total_output: u64 = sessions.iter().map(|s| s.tokens.output).sum();
    let total_cache: u64 = sessions.iter().map(|s| s.tokens.cache_read).sum();
    let total_tools: u32 = sessions.iter().map(|s| s.total_tool_calls()).sum();
    let total_cost: f64 = sessions.iter().map(|s| s.estimated_cost()).sum();

    println!("Sessions: {} ({} active)", sessions.len(), active);
    println!(
        "Tokens:   ↓{} ↑{} (cache: {})",
        pulse::metrics::format_tokens(total_input),
        pulse::metrics::format_tokens(total_output),
        pulse::metrics::format_tokens(total_cache),
    );
    println!("Tools:    {} total calls", total_tools);
    println!("Cost:     {}", pulse::metrics::format_cost(total_cost));
}

fn run_tui(scanner: Scanner) -> anyhow::Result<()> {
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let (tx, rx) = mpsc::channel();
    let watcher = Watcher::start(scanner.watched_dirs(), tx)?;

    let mut app = App::new(scanner, rx);

    loop {
        terminal.draw(|f| ui::draw(f, &app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match app.mode() {
                    pulse::app::ViewMode::Normal => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Up | KeyCode::Char('k') => app.select_prev(),
                        KeyCode::Down | KeyCode::Char('j') => app.select_next(),
                        KeyCode::Tab => app.cycle_tab(),
                        KeyCode::Char('f') => app.cycle_provider_filter(),
                        KeyCode::Char('r') => app.refresh(),
                        KeyCode::Char('/') => app.enter_search(),
                        KeyCode::Char('g') => app.select_first(),
                        KeyCode::Char('G') => app.select_last(),
                        KeyCode::Left => app.scroll_feed_up(),
                        KeyCode::Right => app.scroll_feed_down(),
                        _ => {}
                    },
                    pulse::app::ViewMode::Search => match key.code {
                        KeyCode::Esc | KeyCode::Enter => app.exit_search(),
                        KeyCode::Backspace => app.search_backspace(),
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.clear_search();
                        }
                        KeyCode::Char(c) => app.search_input(c),
                        _ => {}
                    },
                }
            }
        }

        app.poll_events();
    }

    drop(watcher);
    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;

    Ok(())
}
