use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

use ratatui::text::Line;

use crate::legacy_core::config::Config;
use crate::tui::RestoreMode;
use crate::tui::Tui;
use crate::tui_contributions::PluginTerminalCommand;
use crate::tui_contributions::PluginTerminalCommandResult;

const MAX_PLUGIN_TERMINAL_TEXT_OUTPUT_LEN: usize = 16 * 1024;

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum PluginTerminalCommandOutcome {
    NoOutput,
    Text(Vec<Line<'static>>),
    ResumeThread(String),
}

pub(crate) async fn run_plugin_terminal_command(
    tui: &mut Tui,
    config: &Config,
    command: PluginTerminalCommand,
) -> Result<PluginTerminalCommandOutcome, String> {
    let codex_home = config.codex_home.to_path_buf();
    let cwd = config.cwd.to_path_buf();
    tui.with_restored(RestoreMode::Full, move || async move {
        tokio::task::spawn_blocking(move || {
            run_plugin_terminal_command_blocking(command, codex_home, cwd)
        })
        .await
        .map_err(|err| format!("Plugin terminal command failed to finish: {err}"))?
    })
    .await
}

fn run_plugin_terminal_command_blocking(
    command: PluginTerminalCommand,
    codex_home: PathBuf,
    cwd: PathBuf,
) -> Result<PluginTerminalCommandOutcome, String> {
    let output = Command::new(command.command.as_path())
        .args(command.args.iter())
        .current_dir(cwd)
        .env("CODEX_HOME", codex_home)
        .env("CODEX_PLUGIN_ID", command.plugin_id.as_str())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|err| {
            format!(
                "Failed to run plugin terminal command from '{}': {err}",
                command.plugin_id
            )
        })?;

    if !output.status.success() {
        let status = output.status.code().map_or_else(
            || "terminated by signal".to_string(),
            |code| format!("exit code {code}"),
        );
        return Err(format!(
            "Plugin terminal command from '{}' exited with {status}.",
            command.plugin_id
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(interpret_plugin_terminal_stdout(
        command.result,
        stdout.as_ref(),
    ))
}

fn interpret_plugin_terminal_stdout(
    result: PluginTerminalCommandResult,
    stdout: &str,
) -> PluginTerminalCommandOutcome {
    match result {
        PluginTerminalCommandResult::None => {
            let text = stdout.trim();
            if text.is_empty() {
                return PluginTerminalCommandOutcome::NoOutput;
            }
            let text = text
                .chars()
                .take(MAX_PLUGIN_TERMINAL_TEXT_OUTPUT_LEN)
                .collect::<String>();
            PluginTerminalCommandOutcome::Text(
                text.lines()
                    .map(|line| Line::from(line.to_string()))
                    .collect(),
            )
        }
        PluginTerminalCommandResult::ResumeThread => stdout
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(|thread| PluginTerminalCommandOutcome::ResumeThread(thread.to_string()))
            .unwrap_or(PluginTerminalCommandOutcome::NoOutput),
    }
}

#[cfg(test)]
#[path = "plugin_terminal_command_tests.rs"]
mod tests;
