use std::process::Command;
use std::time::{Duration, Instant};

use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::TrayIconBuilder;

use crate::metrics;
use crate::providers::SessionProvider;
use crate::providers::claude::ClaudeProvider;
use crate::providers::codex::CodexProvider;
use crate::providers::copilot::CopilotProvider;
use crate::scanner::Scanner;
use crate::session::Session;

struct MenuItems {
    // Session header
    name_item: MenuItem,
    provider_item: MenuItem,
    directory_item: MenuItem,
    // Model and timing
    model_item: MenuItem,
    duration_item: MenuItem,
    turns_item: MenuItem,
    // Tokens and cost
    tokens_item: MenuItem,
    cache_item: MenuItem,
    cost_item: MenuItem,
    // Activity
    tools_item: MenuItem,
    files_item: MenuItem,
    // Actions
    focus_item: MenuItem,
    dashboard_item: MenuItem,
    quit_item: MenuItem,
}

impl MenuItems {
    fn build(menu: &Menu) -> anyhow::Result<Self> {
        let name_item = MenuItem::new("Scanning...", false, None);
        let provider_item = MenuItem::new("", false, None);
        let directory_item = MenuItem::new("", false, None);

        let model_item = MenuItem::new("", false, None);
        let duration_item = MenuItem::new("", false, None);
        let turns_item = MenuItem::new("", false, None);

        let tokens_item = MenuItem::new("", false, None);
        let cache_item = MenuItem::new("", false, None);
        let cost_item = MenuItem::new("", false, None);

        let tools_item = MenuItem::new("", false, None);
        let files_item = MenuItem::new("", false, None);

        let focus_item = MenuItem::new("Open Focus TUI", true, None);
        let dashboard_item = MenuItem::new("Open Dashboard", true, None);
        let quit_item = MenuItem::new("Quit Pulse", true, None);

        // Session header
        menu.append(&name_item)?;
        menu.append(&provider_item)?;
        menu.append(&directory_item)?;
        menu.append(&PredefinedMenuItem::separator())?;

        // Model and timing
        menu.append(&model_item)?;
        menu.append(&duration_item)?;
        menu.append(&turns_item)?;
        menu.append(&PredefinedMenuItem::separator())?;

        // Tokens and cost
        menu.append(&tokens_item)?;
        menu.append(&cache_item)?;
        menu.append(&cost_item)?;
        menu.append(&PredefinedMenuItem::separator())?;

        // Activity
        menu.append(&tools_item)?;
        menu.append(&files_item)?;
        menu.append(&PredefinedMenuItem::separator())?;

        // Actions
        menu.append(&focus_item)?;
        menu.append(&dashboard_item)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&quit_item)?;

        Ok(Self {
            name_item,
            provider_item,
            directory_item,
            model_item,
            duration_item,
            turns_item,
            tokens_item,
            cache_item,
            cost_item,
            tools_item,
            files_item,
            focus_item,
            dashboard_item,
            quit_item,
        })
    }

    fn update_active(&self, tray: &tray_icon::TrayIcon, session: &Session) {
        let duration = crate::session::human_duration(session.duration());
        let in_tok = metrics::format_tokens(session.tokens.input);
        let out_tok = metrics::format_tokens(session.tokens.output);
        let cost = metrics::format_cost(session.estimated_cost());
        let indicator = if session.is_active { "●" } else { "○" };

        // Menu bar title — spaced for readability
        let title = format!("{indicator}  {duration}   ↓{in_tok}  ↑{out_tok}   {cost}");
        tray.set_title(Some(&title));

        // Session header
        let name = session_display_name(session);
        self.name_item.set_text(name);

        let provider = session.provider.short_label();
        self.provider_item.set_text(format!("Provider        {provider}"));

        let cwd = crate::session::shorten_path(&session.cwd);
        self.directory_item.set_text(format!("Directory       {cwd}"));

        // Model — smart fallback based on provider
        let model_label = model_display(session);
        self.model_item.set_text(format!("Model           {model_label}"));

        self.duration_item.set_text(format!("Duration        {duration}"));

        let turns = session.turn_count;
        let messages = session.user_message_count;
        self.turns_item
            .set_text(format!("Turns           {turns} turns, {messages} messages"));

        // Tokens
        let cache_rate = metrics::cache_hit_rate(&session.tokens);
        self.tokens_item
            .set_text(format!("Tokens          ↓ {in_tok} in   ↑ {out_tok} out"));
        self.cache_item
            .set_text(format!("Cache           {cache_rate:.0}% hit rate"));
        self.cost_item
            .set_text(format!("Estimated       {cost}"));

        // Activity
        let tools = session.total_tool_calls();
        let failed: u32 = session.tool_calls.values().map(|s| s.failures).sum();
        let tools_label = if failed > 0 {
            format!("Tools           {tools} calls ({failed} failed)")
        } else {
            format!("Tools           {tools} calls")
        };
        self.tools_item.set_text(tools_label);

        let files = session.files.len();
        self.files_item
            .set_text(format!("Files           {files} touched"));
    }

    fn update_idle(&self, tray: &tray_icon::TrayIcon) {
        tray.set_title(Some("○  Pulse"));
        self.name_item.set_text("No active sessions");
        self.provider_item.set_text("");
        self.directory_item.set_text("");
        self.model_item.set_text("");
        self.duration_item.set_text("");
        self.turns_item.set_text("");
        self.tokens_item.set_text("");
        self.cache_item.set_text("");
        self.cost_item.set_text("");
        self.tools_item.set_text("");
        self.files_item.set_text("");
    }
}

fn session_display_name(session: &Session) -> String {
    if !session.summary.is_empty() {
        return session.summary.clone();
    }
    // Fall back to last path component of cwd
    session
        .cwd
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or(&session.cwd)
        .to_string()
}

fn model_display(session: &Session) -> String {
    if let Some(ref model) = session.model {
        return model.clone();
    }
    // Smart fallback based on provider
    match session.provider {
        crate::event::Provider::Copilot => "Claude (via Copilot)".into(),
        crate::event::Provider::Claude => "Claude (detecting...)".into(),
        crate::event::Provider::Codex => "GPT (detecting...)".into(),
    }
}

pub fn run() -> anyhow::Result<()> {
    let event_loop = EventLoopBuilder::new().build();

    let menu = Menu::new();
    let items = MenuItems::build(&menu)?;

    let focus_id = items.focus_item.id().clone();
    let dashboard_id = items.dashboard_item.id().clone();
    let quit_id = items.quit_item.id().clone();

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_title("○  Pulse")
        .with_tooltip("Pulse — AI Session Monitor")
        .build()?;

    let menu_channel = MenuEvent::receiver();

    let providers: Vec<Box<dyn SessionProvider>> = vec![
        Box::new(CopilotProvider),
        Box::new(ClaudeProvider),
        Box::new(CodexProvider),
    ];
    let mut scanner = Scanner::new(providers);
    let mut last_scan = Instant::now() - Duration::from_secs(10);

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_secs(2));

        if let Ok(event) = menu_channel.try_recv() {
            if event.id == quit_id {
                *control_flow = ControlFlow::Exit;
                return;
            }
            if event.id == focus_id {
                open_terminal_with("--focus");
            }
            if event.id == dashboard_id {
                open_terminal_with("");
            }
        }

        if let Event::NewEvents(StartCause::ResumeTimeReached { .. } | StartCause::Init) = event {
            if last_scan.elapsed() >= Duration::from_secs(2) {
                scanner.scan_all();
                last_scan = Instant::now();

                let sessions = scanner.sessions();
                let best = sessions.iter().find(|s| s.is_active).or(sessions.first());

                if let Some(session) = best {
                    items.update_active(&tray, session);
                } else {
                    items.update_idle(&tray);
                }
            }
        }
    });
}

fn open_terminal_with(args: &str) {
    let pulse_bin = std::env::current_exe()
        .unwrap_or_else(|_| "pulse".into())
        .display()
        .to_string();

    let cmd = if args.is_empty() {
        pulse_bin
    } else {
        format!("{pulse_bin} {args}")
    };

    let script = format!(
        r#"tell application "Terminal"
    activate
    do script "{cmd}"
end tell"#
    );

    let _ = Command::new("osascript").args(["-e", &script]).spawn();
}
