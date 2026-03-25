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

pub fn run() -> anyhow::Result<()> {
    let event_loop = EventLoopBuilder::new().build();

    let menu = Menu::new();

    let session_item = MenuItem::new("Scanning...", false, None);
    let model_item = MenuItem::new("", false, None);
    let tokens_item = MenuItem::new("", false, None);
    let tools_item = MenuItem::new("", false, None);
    let cost_item = MenuItem::new("", false, None);
    let sep1 = PredefinedMenuItem::separator();
    let focus_item = MenuItem::new("Open Focus TUI", true, None);
    let sep2 = PredefinedMenuItem::separator();
    let quit_item = MenuItem::new("Quit Pulse", true, None);

    menu.append(&session_item)?;
    menu.append(&model_item)?;
    menu.append(&tokens_item)?;
    menu.append(&tools_item)?;
    menu.append(&cost_item)?;
    menu.append(&sep1)?;
    menu.append(&focus_item)?;
    menu.append(&sep2)?;
    menu.append(&quit_item)?;

    let focus_id = focus_item.id().clone();
    let quit_id = quit_item.id().clone();

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_title("○ Pulse")
        .with_tooltip("Pulse - AI Session Monitor")
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
                open_focus_tui();
            }
        }

        if let Event::NewEvents(StartCause::ResumeTimeReached { .. } | StartCause::Init) = event {
            if last_scan.elapsed() >= Duration::from_secs(2) {
                scanner.scan_all();
                last_scan = Instant::now();

                let sessions = scanner.sessions();
                let best = sessions.iter().find(|s| s.is_active).or(sessions.first());

                if let Some(session) = best {
                    update_tray_active(
                        &tray,
                        session,
                        &session_item,
                        &model_item,
                        &tokens_item,
                        &tools_item,
                        &cost_item,
                    );
                } else {
                    tray.set_title(Some("○ Pulse"));
                    session_item.set_text("No active sessions");
                    model_item.set_text("");
                    tokens_item.set_text("");
                    tools_item.set_text("");
                    cost_item.set_text("");
                }
            }
        }
    });
}

fn update_tray_active(
    tray: &tray_icon::TrayIcon,
    session: &Session,
    session_item: &MenuItem,
    model_item: &MenuItem,
    tokens_item: &MenuItem,
    tools_item: &MenuItem,
    cost_item: &MenuItem,
) {
    let duration = crate::session::human_duration(session.duration());
    let in_tok = metrics::format_tokens(session.tokens.input);
    let out_tok = metrics::format_tokens(session.tokens.output);
    let cost = metrics::format_cost(session.estimated_cost());
    let indicator = if session.is_active { "●" } else { "○" };

    let title = format!("{indicator} {duration} ↓{in_tok} ↑{out_tok} {cost}");
    tray.set_title(Some(&title));

    let provider = session.provider.short_label();
    let cwd = crate::session::shorten_path(&session.cwd);
    session_item.set_text(format!("{provider} | {cwd}"));

    let model = session.model.as_deref().unwrap_or("unknown");
    model_item.set_text(format!("Model: {model}"));

    let cache_rate = metrics::cache_hit_rate(&session.tokens);
    tokens_item.set_text(format!(
        "Tokens: ↓{in_tok} ↑{out_tok} | Cache: {cache_rate:.0}%"
    ));

    let tools = session.total_tool_calls();
    let files = session.files.len();
    tools_item.set_text(format!("Tools: {tools} calls | Files: {files} touched"));

    cost_item.set_text(format!("Cost: {cost}"));
}

fn open_focus_tui() {
    let pulse_bin = std::env::current_exe()
        .unwrap_or_else(|_| "pulse".into())
        .display()
        .to_string();

    // Open a new Terminal.app window with pulse --focus
    let script = format!(
        r#"tell application "Terminal"
    activate
    do script "{pulse_bin} --focus"
end tell"#
    );

    let _ = Command::new("osascript").args(["-e", &script]).spawn();
}
