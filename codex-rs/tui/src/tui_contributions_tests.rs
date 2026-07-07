use super::*;
use pretty_assertions::assert_eq;
use std::collections::HashSet;

#[test]
fn parses_status_line_layout_and_prompt_commands() {
    let contribution = parse_tui_contribution(
        "demo@test",
        "Demo",
        r#"{
  "statusLine": [" model-with-reasoning ", "", "current-dir"],
  "layout": { "footer": "minimal" },
  "theme": { "source": "ghostty" },
  "startup": {
    "splashBanner": "hidden",
    "announcementTip": "hidden"
  },
  "slashCommands": [
    {
      "name": "/PiPlan",
      "description": "  Plan   with project context  ",
      "submitPrompt": "Make a plan before editing."
    }
  ]
}"#,
    )
    .expect("valid contribution");

    assert_eq!(
        contribution,
        TuiContributionSet {
            status_line: Some(vec![
                "model-with-reasoning".to_string(),
                "current-dir".to_string(),
            ]),
            footer_layout: Some(FooterLayoutPreset::Minimal),
            theme: Some(PluginThemeContribution {
                source: PluginThemeSource::Ghostty,
                name: DEFAULT_GHOSTTY_SYNC_THEME_NAME.to_string(),
            }),
            startup: StartupUiContribution {
                splash_banner: Some(StartupElementVisibility::Hidden),
                announcement_tip: Some(StartupElementVisibility::Hidden),
            },
            slash_commands: vec![PluginSlashCommand {
                plugin_id: "demo@test".to_string(),
                name: "piplan".to_string(),
                description: "Plan with project context".to_string(),
                submit_prompt: "Make a plan before editing.".to_string(),
            }],
        }
    );
}

#[test]
fn ignores_commands_that_conflict_or_would_run_shell() {
    let contribution = parse_tui_contribution(
        "demo@test",
        "Demo",
        r#"{
  "slashCommands": [
    { "name": "model", "submitPrompt": "This conflicts with a built-in." },
    { "name": "local", "submitPrompt": "!rm -rf target" },
    { "name": "ok", "submitPrompt": "Explain the current diff." }
  ]
}"#,
    )
    .expect("valid contribution");

    assert_eq!(
        contribution.slash_commands,
        vec![PluginSlashCommand {
            plugin_id: "demo@test".to_string(),
            name: "ok".to_string(),
            description: "Run a Demo plugin prompt".to_string(),
            submit_prompt: "Explain the current diff.".to_string(),
        }]
    );
}

#[test]
fn parses_custom_theme_name() {
    let contribution = parse_tui_contribution(
        "demo@test",
        "Demo",
        r#"{
  "theme": { "source": "ghostty", "name": "my-ghostty-sync" }
}"#,
    )
    .expect("valid contribution");

    assert_eq!(
        contribution.theme,
        Some(PluginThemeContribution {
            source: PluginThemeSource::Ghostty,
            name: "my-ghostty-sync".to_string(),
        })
    );
}

#[test]
fn merge_keeps_first_duplicate_command_and_last_singleton_layout_and_theme() {
    let first = TuiContributionSet {
        status_line: Some(vec!["model-name".to_string()]),
        footer_layout: None,
        theme: Some(PluginThemeContribution {
            source: PluginThemeSource::Ghostty,
            name: "first-theme".to_string(),
        }),
        startup: StartupUiContribution {
            splash_banner: Some(StartupElementVisibility::Hidden),
            announcement_tip: None,
        },
        slash_commands: vec![PluginSlashCommand {
            plugin_id: "alpha@test".to_string(),
            name: "plan-extra".to_string(),
            description: "First".to_string(),
            submit_prompt: "First prompt".to_string(),
        }],
    };
    let second = TuiContributionSet {
        status_line: Some(vec!["current-dir".to_string()]),
        footer_layout: Some(FooterLayoutPreset::Minimal),
        theme: Some(PluginThemeContribution {
            source: PluginThemeSource::Ghostty,
            name: "second-theme".to_string(),
        }),
        startup: StartupUiContribution {
            splash_banner: Some(StartupElementVisibility::Visible),
            announcement_tip: Some(StartupElementVisibility::Hidden),
        },
        slash_commands: vec![PluginSlashCommand {
            plugin_id: "beta@test".to_string(),
            name: "plan-extra".to_string(),
            description: "Second".to_string(),
            submit_prompt: "Second prompt".to_string(),
        }],
    };

    let mut merged = TuiContributionSet::default();
    let mut seen = HashSet::new();
    merged.merge_from_plugin(first, &mut seen);
    merged.merge_from_plugin(second, &mut seen);

    assert_eq!(
        merged,
        TuiContributionSet {
            status_line: Some(vec!["current-dir".to_string()]),
            footer_layout: Some(FooterLayoutPreset::Minimal),
            theme: Some(PluginThemeContribution {
                source: PluginThemeSource::Ghostty,
                name: "second-theme".to_string(),
            }),
            startup: StartupUiContribution {
                splash_banner: Some(StartupElementVisibility::Visible),
                announcement_tip: Some(StartupElementVisibility::Hidden),
            },
            slash_commands: vec![PluginSlashCommand {
                plugin_id: "alpha@test".to_string(),
                name: "plan-extra".to_string(),
                description: "First".to_string(),
                submit_prompt: "First prompt".to_string(),
            }],
        }
    );
}
