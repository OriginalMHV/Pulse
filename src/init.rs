use std::path::PathBuf;

use dialoguer::{Confirm, Select, theme::ColorfulTheme};

use crate::config::{self, Config, MenubarConfig, ModeConfig, ProviderConfig};

struct DetectedProvider {
    name: &'static str,
    dir: PathBuf,
    session_count: usize,
    found: bool,
}

pub fn run() -> anyhow::Result<()> {
    println!();
    println!("  \x1b[1;32mPulse Setup\x1b[0m");
    println!("  ──────────────────────────────────────");
    println!();

    // Detect providers
    let providers = detect_providers();
    println!("  Detected AI tools:");
    println!();
    for p in &providers {
        if p.found {
            println!(
                "    \x1b[32m✓\x1b[0m {:<24} {} ({} sessions)",
                p.name,
                p.dir.display(),
                p.session_count
            );
        } else {
            println!(
                "    \x1b[90m✗ {:<24} {} (not found)\x1b[0m",
                p.name,
                p.dir.display()
            );
        }
    }
    println!();

    let any_found = providers.iter().any(|p| p.found);
    if !any_found {
        println!("  No AI session directories found.");
        println!("  Pulse will still work — run an AI tool first, then start Pulse.");
        println!();
    }

    // Select default mode
    let is_macos = cfg!(target_os = "macos");

    let mode_options = if is_macos {
        vec![
            "Menu bar widget     Always visible in your macOS menu bar",
            "Focus TUI           Compact single-session terminal view",
            "Full dashboard      Multi-session TUI with all details",
            "CLI only            Use --list and --stats flags",
        ]
    } else {
        vec![
            "Focus TUI           Compact single-session terminal view",
            "Full dashboard      Multi-session TUI with all details",
            "CLI only            Use --list and --stats flags",
        ]
    };

    println!("  How would you like to use Pulse?");
    println!();

    let mode_idx = Select::with_theme(&ColorfulTheme::default())
        .items(&mode_options)
        .default(0)
        .interact()?;

    let (default_mode, wants_menubar) = if is_macos {
        match mode_idx {
            0 => ("menubar", true),
            1 => ("focus", false),
            2 => ("dashboard", false),
            _ => ("cli", false),
        }
    } else {
        match mode_idx {
            0 => ("focus", false),
            1 => ("dashboard", false),
            _ => ("cli", false),
        }
    };

    // Auto-start on login?
    let auto_start = if wants_menubar && is_macos {
        println!();
        Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("  Start menu bar automatically on login?")
            .default(true)
            .interact()?
    } else {
        false
    };

    // Build config
    let config = Config {
        mode: ModeConfig {
            default: default_mode.into(),
        },
        menubar: MenubarConfig {
            auto_start,
            poll_interval: 2,
        },
        providers: ProviderConfig {
            copilot: providers.iter().any(|p| p.name == "GitHub Copilot CLI" && p.found),
            claude: providers.iter().any(|p| p.name == "Claude Code" && p.found),
            codex: providers.iter().any(|p| p.name == "OpenAI Codex CLI" && p.found),
        },
    };

    // Save config
    config::save(&config)?;

    println!();
    println!(
        "  \x1b[32m✓\x1b[0m Configuration saved to {}",
        config::config_path().display()
    );

    // Apply: start menubar and/or install LaunchAgent
    if wants_menubar && is_macos {
        // Kill any existing daemon first
        kill_existing_daemons();

        let exe = std::env::current_exe()?;

        if auto_start {
            // Install LaunchAgent — launchctl will start the daemon
            install_launch_agent_quiet(&exe)?;
            println!("  \x1b[32m✓\x1b[0m LaunchAgent installed and started");
        } else {
            // No auto-start — just spawn the daemon directly
            let child = std::process::Command::new(&exe)
                .arg("--menubar-daemon")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()?;
            println!(
                "  \x1b[32m✓\x1b[0m Menu bar started (pid {})",
                child.id()
            );
        }
    }

    // Summary
    println!();
    println!("  \x1b[1mYou're all set!\x1b[0m");
    println!();

    match default_mode {
        "menubar" => {
            println!("  The menu bar widget is now running.");
            println!();
            println!("  Quick reference:");
            println!("    pulse                Run default mode (menu bar)");
            println!("    pulse --focus        Compact TUI view");
            println!("    pulse --stats        Quick stats in terminal");
            println!("    pulse --uninstall    Remove auto-start and stop daemon");
        }
        "focus" => {
            println!("  Run \x1b[1mpulse\x1b[0m to start the focus TUI.");
            println!();
            println!("  Quick reference:");
            println!("    pulse                Run default mode (focus TUI)");
            println!("    pulse --menubar      Start menu bar widget");
            println!("    pulse --stats        Quick stats in terminal");
        }
        "dashboard" => {
            println!("  Run \x1b[1mpulse\x1b[0m to start the full dashboard.");
            println!();
            println!("  Quick reference:");
            println!("    pulse                Run default mode (dashboard)");
            println!("    pulse --focus        Compact TUI view");
            println!("    pulse --menubar      Start menu bar widget");
        }
        _ => {
            println!("  Quick reference:");
            println!("    pulse --list         List sessions");
            println!("    pulse --stats        Aggregate statistics");
            println!("    pulse --focus        Compact TUI view");
            println!("    pulse --menubar      Start menu bar widget");
        }
    }

    println!();

    Ok(())
}

fn detect_providers() -> Vec<DetectedProvider> {
    let home = dirs::home_dir().unwrap_or_default();

    let copilot_dir = home.join(".copilot/session-state");
    let claude_dir = home.join(".claude/projects");
    let codex_dir = home.join(".codex/sessions");

    vec![
        DetectedProvider {
            name: "GitHub Copilot CLI",
            session_count: count_sessions(&copilot_dir),
            found: copilot_dir.is_dir(),
            dir: copilot_dir,
        },
        DetectedProvider {
            name: "Claude Code",
            session_count: count_sessions(&claude_dir),
            found: claude_dir.is_dir(),
            dir: claude_dir,
        },
        DetectedProvider {
            name: "OpenAI Codex CLI",
            session_count: count_sessions(&codex_dir),
            found: codex_dir.is_dir(),
            dir: codex_dir,
        },
    ]
}

fn count_sessions(dir: &PathBuf) -> usize {
    if !dir.is_dir() {
        return 0;
    }
    std::fs::read_dir(dir)
        .map(|entries| entries.filter_map(|e| e.ok()).count())
        .unwrap_or(0)
}

fn install_launch_agent_quiet(exe: &std::path::Path) -> anyhow::Result<()> {
    let exe_path = exe.display().to_string();
    let label = "dev.pulse.menubar";

    let plist_dir = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?
        .join("Library/LaunchAgents");

    std::fs::create_dir_all(&plist_dir)?;

    let plist_path = plist_dir.join(format!("{label}.plist"));

    let plist_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
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

    let _ = std::process::Command::new("launchctl")
        .args(["load", "-w"])
        .arg(&plist_path)
        .status();

    Ok(())
}

fn kill_existing_daemons() {
    let own_pid = std::process::id();
    if let Ok(output) = std::process::Command::new("pgrep")
        .args(["-f", "pulse.*--menubar-daemon"])
        .output()
    {
        let pids = String::from_utf8_lossy(&output.stdout);
        for line in pids.lines() {
            if let Ok(pid) = line.trim().parse::<u32>() {
                if pid != own_pid {
                    let _ = std::process::Command::new("kill")
                        .arg(pid.to_string())
                        .status();
                }
            }
        }
    }
}
