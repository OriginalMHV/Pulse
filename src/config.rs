use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub mode: ModeConfig,
    #[serde(default)]
    pub menubar: MenubarConfig,
    #[serde(default)]
    pub providers: ProviderConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeConfig {
    /// Default mode when running `pulse` without flags
    #[serde(default = "default_mode")]
    pub default: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenubarConfig {
    /// Auto-start menu bar on login via macOS LaunchAgent
    #[serde(default)]
    pub auto_start: bool,
    /// Poll interval in seconds
    #[serde(default = "default_poll_interval")]
    pub poll_interval: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default = "default_true")]
    pub copilot: bool,
    #[serde(default = "default_true")]
    pub claude: bool,
    #[serde(default = "default_true")]
    pub codex: bool,
}

impl Default for ModeConfig {
    fn default() -> Self {
        Self {
            default: default_mode(),
        }
    }
}

impl Default for MenubarConfig {
    fn default() -> Self {
        Self {
            auto_start: false,
            poll_interval: default_poll_interval(),
        }
    }
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            copilot: true,
            claude: true,
            codex: true,
        }
    }
}

fn default_mode() -> String {
    "dashboard".into()
}

fn default_poll_interval() -> u64 {
    2
}

fn default_true() -> bool {
    true
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap().join(".config"))
        .join("pulse")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn load() -> Option<Config> {
    let path = config_path();
    let content = std::fs::read_to_string(path).ok()?;
    toml::from_str(&content).ok()
}

pub fn save(config: &Config) -> anyhow::Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let content = toml::to_string_pretty(config)?;
    std::fs::write(config_path(), content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_serializes() {
        let config = Config::default();
        let toml = toml::to_string_pretty(&config).unwrap();
        assert!(toml.contains("dashboard"));
        assert!(toml.contains("copilot"));
    }

    #[test]
    fn config_roundtrip() {
        let config = Config {
            mode: ModeConfig {
                default: "menubar".into(),
            },
            menubar: MenubarConfig {
                auto_start: true,
                poll_interval: 5,
            },
            providers: ProviderConfig {
                copilot: true,
                claude: true,
                codex: false,
            },
        };
        let toml = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&toml).unwrap();
        assert_eq!(parsed.mode.default, "menubar");
        assert!(parsed.menubar.auto_start);
        assert!(!parsed.providers.codex);
    }
}
