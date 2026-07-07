use super::*;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use syntect::highlighting::ThemeSet;

const GHOSTTY_OUTPUT: &str = r##"
theme = "light:GitHub Light,dark:Catppuccin Mocha"
background = #1e1e2e
foreground = cdd6f4
selection-background = #45475a
selection-foreground = #f5e0dc
palette = 1=#f38ba8
palette = 2=#a6e3a1
palette = 3=#f9e2af
palette = 4=#89b4fa
palette = 5=#cba6f7
palette = 6=#94e2d5
"##;

#[test]
fn parses_ghostty_config_snapshot() {
    let snapshot = read_ghostty_snapshot(GHOSTTY_OUTPUT)
        .expect("valid ghostty output")
        .expect("complete color snapshot");

    assert_eq!(
        snapshot,
        GhosttySnapshot {
            theme_spec: Some("light:GitHub Light,dark:Catppuccin Mocha".to_string()),
            active_theme_name: Some("Catppuccin Mocha".to_string()),
            background: "#1e1e2e".to_string(),
            foreground: "#cdd6f4".to_string(),
            selection_background: Some("#45475a".to_string()),
            selection_foreground: Some("#f5e0dc".to_string()),
            palette: BTreeMap::from([
                (1, "#f38ba8".to_string()),
                (2, "#a6e3a1".to_string()),
                (3, "#f9e2af".to_string()),
                (4, "#89b4fa".to_string()),
                (5, "#cba6f7".to_string()),
                (6, "#94e2d5".to_string()),
            ]),
            is_light: false,
        }
    );
}

#[test]
fn generated_tmtheme_is_parseable_by_syntect() {
    let snapshot = read_ghostty_snapshot(GHOSTTY_OUTPUT)
        .expect("valid ghostty output")
        .expect("complete color snapshot");
    let contents = build_tmtheme("ghostty-sync", &snapshot).expect("theme xml");
    let dir = tempfile::tempdir().expect("tempdir");
    let theme_path = dir.path().join("ghostty-sync.tmTheme");
    std::fs::write(&theme_path, contents).expect("write theme");

    let theme = ThemeSet::get_theme(&theme_path).expect("syntect parses generated theme");

    assert_eq!(
        theme.name.as_deref(),
        Some("ghostty-sync (Catppuccin Mocha)")
    );
}
