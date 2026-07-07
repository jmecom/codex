//! Plugin-provided TUI customizations.
//!
//! Plugins declare a small JSON file from `plugin.json` via `"tui": "./tui.json"`. The file is
//! declarative on purpose: it gives the TUI stable extension points without exposing ratatui
//! internals or allowing plugin code to run inside the render loop.

use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use codex_core_plugins::PluginsManager;
use codex_core_plugins::manifest::load_plugin_manifest;
use serde::Deserialize;
use tracing::warn;

use crate::key_hint::KeyBinding;
use crate::keymap::parse_keybinding;
use crate::legacy_core::config::Config;
use crate::slash_command::SlashCommand;

const MAX_STATUS_LINE_ITEMS: usize = 32;
const MAX_STATUS_LINE_ITEM_LEN: usize = 64;
const MAX_PLUGIN_SLASH_COMMANDS: usize = 64;
const MAX_PLUGIN_SLASH_COMMAND_NAME_LEN: usize = 40;
const MAX_PLUGIN_SLASH_COMMAND_DESCRIPTION_LEN: usize = 160;
const MAX_PLUGIN_SLASH_COMMAND_PROMPT_LEN: usize = 4000;
const MAX_PLUGIN_KEY_BINDINGS: usize = 64;
const MAX_PLUGIN_KEY_BINDING_SPEC_LEN: usize = 80;
const MAX_PLUGIN_TERMINAL_COMMAND_LEN: usize = 512;
const MAX_PLUGIN_TERMINAL_COMMAND_ARGS: usize = 32;
const MAX_PLUGIN_TERMINAL_COMMAND_ARG_LEN: usize = 512;
const MAX_THEME_NAME_LEN: usize = 80;
pub(crate) const DEFAULT_GHOSTTY_SYNC_THEME_NAME: &str = "ghostty-sync";

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FooterLayoutPreset {
    #[default]
    Default,
    Minimal,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StartupElementVisibility {
    Visible,
    Hidden,
}

impl StartupElementVisibility {
    fn is_visible(self) -> bool {
        matches!(self, Self::Visible)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PluginSlashCommand {
    pub(crate) plugin_id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) action: PluginSlashCommandAction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PluginKeyBinding {
    pub(crate) plugin_id: String,
    pub(crate) key: KeyBinding,
    pub(crate) command: PluginSlashCommand,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PluginSlashCommandAction {
    SubmitPrompt(String),
    TerminalCommand(PluginTerminalCommand),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PluginTerminalCommand {
    pub(crate) plugin_id: String,
    pub(crate) command: PathBuf,
    pub(crate) args: Vec<String>,
    pub(crate) result: PluginTerminalCommandResult,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PluginTerminalCommandResult {
    #[default]
    None,
    ResumeThread,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct TuiContributionSet {
    pub(crate) status_line: Option<Vec<String>>,
    pub(crate) footer_layout: Option<FooterLayoutPreset>,
    pub(crate) theme: Option<PluginThemeContribution>,
    pub(crate) changes: ChangeUiContribution,
    pub(crate) startup: StartupUiContribution,
    pub(crate) slash_commands: Vec<PluginSlashCommand>,
    pub(crate) key_bindings: Vec<PluginKeyBinding>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PluginThemeContribution {
    pub(crate) source: PluginThemeSource,
    pub(crate) name: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PluginThemeSource {
    Ghostty,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum LastChangedFileDiffVisibility {
    Visible,
    Hidden,
}

impl LastChangedFileDiffVisibility {
    fn is_visible(self) -> bool {
        matches!(self, Self::Visible)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ChangeUiContribution {
    pub(crate) last_changed_file_diff: Option<LastChangedFileDiffVisibility>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct StartupUiContribution {
    pub(crate) splash_banner: Option<StartupElementVisibility>,
    pub(crate) announcement_tip: Option<StartupElementVisibility>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTuiContribution {
    #[serde(default)]
    status_line: Option<Vec<String>>,
    #[serde(default)]
    layout: Option<RawTuiLayout>,
    #[serde(default)]
    theme: Option<RawTuiThemeContribution>,
    #[serde(default)]
    changes: Option<RawChangeUiContribution>,
    #[serde(default)]
    startup: Option<RawStartupUiContribution>,
    #[serde(default)]
    slash_commands: Vec<RawTuiSlashCommand>,
    #[serde(default)]
    key_bindings: Vec<RawPluginKeyBinding>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTuiLayout {
    #[serde(default)]
    footer: Option<FooterLayoutPreset>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTuiThemeContribution {
    #[serde(default)]
    source: Option<PluginThemeSource>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawChangeUiContribution {
    #[serde(default)]
    last_changed_file_diff: Option<LastChangedFileDiffVisibility>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawStartupUiContribution {
    #[serde(default)]
    splash_banner: Option<StartupElementVisibility>,
    #[serde(default)]
    announcement_tip: Option<StartupElementVisibility>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTuiSlashCommand {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    submit_prompt: Option<String>,
    #[serde(default)]
    terminal_command: Option<RawPluginTerminalCommand>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPluginTerminalCommand {
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    result: PluginTerminalCommandResult,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPluginKeyBinding {
    #[serde(default)]
    key: String,
    #[serde(default)]
    command: String,
}

impl TuiContributionSet {
    pub(crate) async fn load_enabled_plugins(config: &Config) -> Self {
        let plugins_manager = PluginsManager::new(config.codex_home.to_path_buf());
        let loaded_plugins = plugins_manager
            .plugins_for_config(&config.plugins_config_input())
            .await;

        let mut set = Self::default();
        let mut seen_slash_commands = HashSet::new();
        let mut seen_key_bindings = HashSet::new();
        for plugin in loaded_plugins
            .plugins()
            .iter()
            .filter(|plugin| plugin.is_active())
        {
            let Some(manifest) = load_plugin_manifest(plugin.root.as_path()) else {
                continue;
            };
            let Some(tui_path) = manifest.paths.tui else {
                continue;
            };
            let contents = match tokio::fs::read_to_string(tui_path.as_path()).await {
                Ok(contents) => contents,
                Err(err) => {
                    warn!(
                        plugin = plugin.config_name,
                        path = %tui_path.display(),
                        "failed to read plugin TUI contribution: {err}"
                    );
                    continue;
                }
            };
            let contribution = match parse_tui_contribution(
                &plugin.config_name,
                plugin.display_name(),
                plugin.root.as_path(),
                &contents,
            ) {
                Ok(contribution) => contribution,
                Err(err) => {
                    warn!(
                        plugin = plugin.config_name,
                        path = %tui_path.display(),
                        "failed to parse plugin TUI contribution: {err}"
                    );
                    continue;
                }
            };
            set.merge_from_plugin(
                contribution,
                &mut seen_slash_commands,
                &mut seen_key_bindings,
            );
        }

        set
    }

    pub(crate) fn apply_to_config(&self, config: &mut Config) {
        if let Some(status_line) = self.status_line.clone() {
            config.tui_status_line = Some(status_line);
        }
    }

    pub(crate) fn show_splash_banner(&self) -> bool {
        self.startup
            .splash_banner
            .is_none_or(StartupElementVisibility::is_visible)
    }

    pub(crate) fn show_announcement_tip(&self) -> bool {
        self.startup
            .announcement_tip
            .is_none_or(StartupElementVisibility::is_visible)
    }

    pub(crate) fn show_last_changed_file_diff(&self) -> bool {
        self.changes
            .last_changed_file_diff
            .is_some_and(LastChangedFileDiffVisibility::is_visible)
    }

    fn merge_from_plugin(
        &mut self,
        contribution: TuiContributionSet,
        seen_slash_commands: &mut HashSet<String>,
        seen_key_bindings: &mut HashSet<KeyBinding>,
    ) {
        if contribution.status_line.is_some() {
            self.status_line = contribution.status_line;
        }
        if contribution.footer_layout.is_some() {
            self.footer_layout = contribution.footer_layout;
        }
        if contribution.theme.is_some() {
            self.theme = contribution.theme;
        }
        self.changes.merge_from(contribution.changes);
        self.startup.merge_from(contribution.startup);
        let mut accepted_plugin_commands = HashSet::new();
        for command in contribution.slash_commands {
            if seen_slash_commands.insert(command.name.clone()) {
                accepted_plugin_commands.insert(command.name.clone());
                self.slash_commands.push(command);
            } else {
                warn!(
                    plugin = command.plugin_id,
                    command = command.name,
                    "ignoring duplicate plugin slash command"
                );
            }
        }
        for binding in contribution.key_bindings {
            if !accepted_plugin_commands.contains(&binding.command.name) {
                warn!(
                    plugin = binding.plugin_id,
                    command = binding.command.name,
                    key = %binding.key.display_label(),
                    "ignoring plugin key binding for unavailable command"
                );
                continue;
            }
            if seen_key_bindings.insert(binding.key) {
                self.key_bindings.push(binding);
            } else {
                warn!(
                    plugin = binding.plugin_id,
                    key = %binding.key.display_label(),
                    "ignoring duplicate plugin key binding"
                );
            }
        }
    }
}

impl ChangeUiContribution {
    fn merge_from(&mut self, contribution: Self) {
        if contribution.last_changed_file_diff.is_some() {
            self.last_changed_file_diff = contribution.last_changed_file_diff;
        }
    }
}

impl StartupUiContribution {
    fn merge_from(&mut self, contribution: Self) {
        if contribution.splash_banner.is_some() {
            self.splash_banner = contribution.splash_banner;
        }
        if contribution.announcement_tip.is_some() {
            self.announcement_tip = contribution.announcement_tip;
        }
    }
}

fn parse_tui_contribution(
    plugin_id: &str,
    plugin_display_name: &str,
    plugin_root: &Path,
    contents: &str,
) -> Result<TuiContributionSet, serde_json::Error> {
    let raw = serde_json::from_str::<RawTuiContribution>(contents)?;
    let status_line = raw.status_line.map(normalize_status_line);
    let footer_layout = raw.layout.and_then(|layout| layout.footer);
    let theme = raw.theme.and_then(normalize_theme_contribution);
    let changes = raw.changes.map(normalize_change_ui).unwrap_or_default();
    let startup = raw.startup.map(normalize_startup_ui).unwrap_or_default();
    let slash_commands = normalize_plugin_slash_commands(
        plugin_id,
        plugin_display_name,
        plugin_root,
        raw.slash_commands,
    );
    let key_bindings =
        normalize_plugin_key_bindings(plugin_id, raw.key_bindings, slash_commands.as_slice());

    Ok(TuiContributionSet {
        status_line,
        footer_layout,
        theme,
        changes,
        startup,
        slash_commands,
        key_bindings,
    })
}

fn normalize_theme_contribution(raw: RawTuiThemeContribution) -> Option<PluginThemeContribution> {
    let source = raw.source?;
    let name = raw
        .name
        .as_deref()
        .and_then(normalize_theme_name)
        .unwrap_or_else(|| DEFAULT_GHOSTTY_SYNC_THEME_NAME.to_string());
    Some(PluginThemeContribution { source, name })
}

fn normalize_theme_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_THEME_NAME_LEN {
        return None;
    }

    name.chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_'))
        .then(|| name.to_string())
}

fn normalize_change_ui(raw: RawChangeUiContribution) -> ChangeUiContribution {
    ChangeUiContribution {
        last_changed_file_diff: raw.last_changed_file_diff,
    }
}

fn normalize_startup_ui(raw: RawStartupUiContribution) -> StartupUiContribution {
    StartupUiContribution {
        splash_banner: raw.splash_banner,
        announcement_tip: raw.announcement_tip,
    }
}

fn normalize_status_line(items: Vec<String>) -> Vec<String> {
    items
        .into_iter()
        .filter_map(|item| {
            let item = item.trim();
            (!item.is_empty() && item.chars().count() <= MAX_STATUS_LINE_ITEM_LEN)
                .then(|| item.to_string())
        })
        .take(MAX_STATUS_LINE_ITEMS)
        .collect()
}

fn normalize_plugin_slash_commands(
    plugin_id: &str,
    plugin_display_name: &str,
    plugin_root: &Path,
    commands: Vec<RawTuiSlashCommand>,
) -> Vec<PluginSlashCommand> {
    let mut normalized = Vec::new();
    let mut seen_names = HashSet::new();
    for raw in commands.into_iter().take(MAX_PLUGIN_SLASH_COMMANDS) {
        let Some(name) = normalize_plugin_slash_command_name(&raw.name) else {
            warn!(
                plugin = plugin_id,
                command = raw.name,
                "ignoring plugin slash command with invalid name"
            );
            continue;
        };
        if SlashCommand::from_str(&name).is_ok() {
            warn!(
                plugin = plugin_id,
                command = name,
                "ignoring plugin slash command that conflicts with a built-in command"
            );
            continue;
        }
        if !seen_names.insert(name.clone()) {
            warn!(
                plugin = plugin_id,
                command = name,
                "ignoring duplicate plugin slash command in contribution file"
            );
            continue;
        }

        let Some(action) = normalize_plugin_slash_command_action(
            plugin_id,
            plugin_root,
            raw.submit_prompt.as_deref(),
            raw.terminal_command,
        ) else {
            warn!(
                plugin = plugin_id,
                command = name,
                "ignoring plugin slash command with invalid action"
            );
            continue;
        };
        let description = raw
            .description
            .as_deref()
            .and_then(normalize_plugin_slash_command_description)
            .unwrap_or_else(|| format!("Run a {plugin_display_name} plugin prompt"));

        normalized.push(PluginSlashCommand {
            plugin_id: plugin_id.to_string(),
            name,
            description,
            action,
        });
    }

    normalized
}

fn normalize_plugin_key_bindings(
    plugin_id: &str,
    bindings: Vec<RawPluginKeyBinding>,
    slash_commands: &[PluginSlashCommand],
) -> Vec<PluginKeyBinding> {
    let mut normalized = Vec::new();
    let mut seen_keys = HashSet::new();
    for raw in bindings.into_iter().take(MAX_PLUGIN_KEY_BINDINGS) {
        let key_spec = raw.key.trim();
        if key_spec.is_empty() || key_spec.chars().count() > MAX_PLUGIN_KEY_BINDING_SPEC_LEN {
            warn!(
                plugin = plugin_id,
                key = raw.key,
                "ignoring plugin key binding with invalid key"
            );
            continue;
        }
        let key_spec = key_spec.to_ascii_lowercase();
        let Some(key) = parse_keybinding(&key_spec) else {
            warn!(
                plugin = plugin_id,
                key = key_spec,
                "ignoring plugin key binding with invalid key"
            );
            continue;
        };
        if !seen_keys.insert(key) {
            warn!(
                plugin = plugin_id,
                key = %key.display_label(),
                "ignoring duplicate plugin key binding in contribution file"
            );
            continue;
        }

        let Some(command_name) = normalize_plugin_slash_command_name(&raw.command) else {
            warn!(
                plugin = plugin_id,
                command = raw.command,
                "ignoring plugin key binding with invalid command"
            );
            continue;
        };
        let Some(command) = slash_commands
            .iter()
            .find(|command| command.name == command_name)
        else {
            warn!(
                plugin = plugin_id,
                command = command_name,
                key = %key.display_label(),
                "ignoring plugin key binding for unknown command"
            );
            continue;
        };

        normalized.push(PluginKeyBinding {
            plugin_id: plugin_id.to_string(),
            key,
            command: command.clone(),
        });
    }

    normalized
}

fn normalize_plugin_slash_command_action(
    plugin_id: &str,
    plugin_root: &Path,
    submit_prompt: Option<&str>,
    terminal_command: Option<RawPluginTerminalCommand>,
) -> Option<PluginSlashCommandAction> {
    if let Some(submit_prompt) = submit_prompt {
        return normalize_submit_prompt(Some(submit_prompt))
            .map(PluginSlashCommandAction::SubmitPrompt);
    }

    terminal_command
        .and_then(|command| normalize_terminal_command(plugin_id, plugin_root, command))
        .map(PluginSlashCommandAction::TerminalCommand)
}

fn normalize_terminal_command(
    plugin_id: &str,
    plugin_root: &Path,
    raw: RawPluginTerminalCommand,
) -> Option<PluginTerminalCommand> {
    let command = raw.command?;
    let command = command.trim();
    if command.is_empty()
        || command.chars().count() > MAX_PLUGIN_TERMINAL_COMMAND_LEN
        || command.contains('\0')
    {
        return None;
    }

    let command_path = PathBuf::from(command);
    let command = if command_path.is_relative() && command_path.components().count() > 1 {
        plugin_root.join(command_path)
    } else {
        command_path
    };
    let args = raw
        .args
        .into_iter()
        .take(MAX_PLUGIN_TERMINAL_COMMAND_ARGS)
        .filter(|arg| {
            !arg.contains('\0') && arg.chars().count() <= MAX_PLUGIN_TERMINAL_COMMAND_ARG_LEN
        })
        .collect();

    Some(PluginTerminalCommand {
        plugin_id: plugin_id.to_string(),
        command,
        args,
        result: raw.result,
    })
}

fn normalize_plugin_slash_command_name(name: &str) -> Option<String> {
    let name = name.trim();
    let name = name.strip_prefix('/').unwrap_or(name);
    if name.is_empty() || name.chars().count() > MAX_PLUGIN_SLASH_COMMAND_NAME_LEN {
        return None;
    }
    let name = name.to_ascii_lowercase();
    name.chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
        .then_some(name)
}

fn normalize_plugin_slash_command_description(description: &str) -> Option<String> {
    let description = description.split_whitespace().collect::<Vec<_>>().join(" ");
    (!description.is_empty()
        && description.chars().count() <= MAX_PLUGIN_SLASH_COMMAND_DESCRIPTION_LEN)
        .then_some(description)
}

fn normalize_submit_prompt(prompt: Option<&str>) -> Option<String> {
    let prompt = prompt?;
    let prompt = prompt.trim();
    if prompt.is_empty()
        || prompt.trim_start().starts_with('!')
        || prompt.chars().count() > MAX_PLUGIN_SLASH_COMMAND_PROMPT_LEN
    {
        return None;
    }
    Some(prompt.to_string())
}

#[cfg(test)]
#[path = "tui_contributions_tests.rs"]
mod tests;
