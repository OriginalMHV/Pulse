use std::io;
use std::process::Command as ProcessCommand;
#[cfg(target_os = "macos")]
use std::process::Stdio;
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

#[cfg(target_os = "macos")]
const LAUNCH_AGENT_LABEL: &str = "dev.pulse.menubar";

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

    /// Focus mode: compact single-session view for side panes
    #[arg(long)]
    focus: bool,

    /// Launch the menu bar widget (detaches to background)
    #[arg(long)]
    menubar: bool,

    /// Internal: run menu bar event loop in foreground (used by --menubar and launchd)
    #[arg(long, hide = true)]
    menubar_daemon: bool,

    /// Install as a macOS LaunchAgent (auto-starts menu bar on login)
    #[arg(long)]
    install: bool,

    /// Remove the LaunchAgent and stop the menu bar daemon
    #[arg(long)]
    uninstall: bool,

    /// Interactive setup wizard
    #[arg(long)]
    init: bool,

    /// Filter by provider name (copilot, claude, codex)
    #[arg(long)]
    provider: Option<String>,

    /// Attach as a tmux side pane alongside your current terminal
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    attach: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if cli.init {
        return pulse::init::run();
    }

    if let Some(ref session_filter) = cli.attach {
        let filter = if session_filter.is_empty() {
            None
        } else {
            Some(session_filter.as_str())
        };
        return attach_tmux_pane(filter);
    }

    #[cfg(target_os = "macos")]
    if cli.install {
        return install_launch_agent();
    }

    #[cfg(target_os = "macos")]
    if cli.uninstall {
        return uninstall_launch_agent();
    }

    #[cfg(not(target_os = "macos"))]
    if cli.install || cli.uninstall {
        anyhow::bail!("--install and --uninstall are only supported on macOS");
    }

    #[cfg(target_os = "macos")]
    if cli.menubar_daemon {
        return pulse::menubar::run();
    }

    #[cfg(target_os = "macos")]
    if cli.menubar {
        return launch_menubar_background();
    }

    #[cfg(not(target_os = "macos"))]
    if cli.menubar || cli.menubar_daemon {
        anyhow::bail!("--menubar is only supported on macOS");
    }

    // Check if an explicit mode flag was given
    let has_explicit_mode = cli.list || cli.stats || cli.focus;

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
        println!();
        println!("Run `pulse --init` to set up Pulse.");
        return Ok(());
    }

    if cli.focus {
        return run_focus_tui(scanner);
    }

    // No explicit flag — use config default
    if !has_explicit_mode {
        if let Some(config) = pulse::config::load() {
            match config.mode.default.as_str() {
                "focus" => return run_focus_tui(scanner),
                #[cfg(target_os = "macos")]
                "menubar" => return launch_menubar_background(),
                _ => {} // fall through to dashboard
            }
        }
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

fn attach_tmux_pane(session_filter: Option<&str>) -> anyhow::Result<()> {
    if std::env::var("TMUX").is_err() {
        anyhow::bail!(
            "Not inside a tmux session. Run `tmux` first, then use `pulse --attach`.\n\
             Or just run `pulse --focus` directly for the compact TUI."
        );
    }

    let pulse_bin = std::env::current_exe()
        .unwrap_or_else(|_| "pulse".into())
        .display()
        .to_string();

    let pane_cmd = match session_filter {
        Some(filter) => format!("{pulse_bin} --focus --provider {filter}"),
        None => format!("{pulse_bin} --focus"),
    };

    let status = ProcessCommand::new("tmux")
        .args(["split-window", "-h", "-l", "35%", &pane_cmd])
        .status()?;

    if !status.success() {
        anyhow::bail!("Failed to create tmux pane");
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn launch_menubar_background() -> anyhow::Result<()> {
    kill_existing_daemons();

    let exe = std::env::current_exe()?;

    let child = ProcessCommand::new(&exe)
        .arg("--menubar-daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    println!("Pulse menu bar started (pid {})", child.id());
    println!();
    println!("To auto-start on login:  pulse --install");
    println!("To stop:                 pulse --uninstall");

    Ok(())
}

/// Kill any running pulse menubar daemons so we don't end up with duplicates.
#[cfg(target_os = "macos")]
fn kill_existing_daemons() {
    // Find PIDs of existing menubar-daemon processes (excluding our own PID)
    let own_pid = std::process::id();
    if let Ok(output) = ProcessCommand::new("pgrep")
        .args(["-f", "pulse.*--menubar-daemon"])
        .output()
    {
        let pids = String::from_utf8_lossy(&output.stdout);
        for line in pids.lines() {
            if let Ok(pid) = line.trim().parse::<u32>() {
                if pid != own_pid {
                    let _ = ProcessCommand::new("kill").arg(pid.to_string()).status();
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn install_launch_agent() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let exe_path = exe.display().to_string();

    let plist_dir = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?
        .join("Library/LaunchAgents");

    std::fs::create_dir_all(&plist_dir)?;

    let plist_path = plist_dir.join(format!("{LAUNCH_AGENT_LABEL}.plist"));

    let plist_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LAUNCH_AGENT_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe_path}</string>
        <string>--menubar-daemon</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>StandardOutPath</key>
    <string>/tmp/pulse-menubar.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/pulse-menubar.log</string>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>"#
    );

    std::fs::write(&plist_path, plist_content)?;

    // Load the agent
    let status = ProcessCommand::new("launchctl")
        .args(["load", "-w"])
        .arg(&plist_path)
        .status()?;

    if status.success() {
        println!("Pulse menu bar installed and started.");
        println!("It will auto-start on login.");
        println!();
        println!("LaunchAgent: {}", plist_path.display());
        println!("To remove:   pulse --uninstall");
    } else {
        anyhow::bail!(
            "Failed to load LaunchAgent. The plist was written to {}",
            plist_path.display()
        );
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn uninstall_launch_agent() -> anyhow::Result<()> {
    let plist_path = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?
        .join("Library/LaunchAgents")
        .join(format!("{LAUNCH_AGENT_LABEL}.plist"));

    if plist_path.exists() {
        // Unload first (ignore errors — might not be loaded)
        let _ = ProcessCommand::new("launchctl")
            .args(["unload"])
            .arg(&plist_path)
            .status();

        std::fs::remove_file(&plist_path)?;
        println!("Pulse menu bar uninstalled.");
        println!("Removed: {}", plist_path.display());
    } else {
        println!("No LaunchAgent found. Nothing to uninstall.");
    }

    // Also kill any running daemon
    kill_existing_daemons();

    Ok(())
}

fn run_focus_tui(scanner: Scanner) -> anyhow::Result<()> {
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let (tx, rx) = mpsc::channel();
    let watcher = Watcher::start(scanner.watched_dirs(), tx)?;

    let mut app = App::new(scanner, rx);
    app.focus_mode = true;

    loop {
        terminal.draw(|f| ui::focus::draw(f, &app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('r') => app.refresh(),
                    KeyCode::Char('f') => app.cycle_provider_filter(),
                    KeyCode::Char('s') => app.toggle_auto_scroll(),
                    KeyCode::Up | KeyCode::Char('k') => {
                        app.focus_auto_scroll = false;
                        app.scroll_feed_up();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        app.scroll_feed_down();
                    }
                    _ => {}
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
