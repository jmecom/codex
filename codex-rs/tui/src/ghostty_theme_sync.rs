//! Generate a Codex syntax theme from Ghostty's resolved terminal colors.
//!
//! This backs the declarative plugin setting `theme.source = "ghostty"`. It is intentionally
//! narrow: plugins can opt into this built-in Ghostty reader, but they cannot ask the TUI to run
//! arbitrary theme-generation commands.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::time::Duration;

use crate::legacy_core::config::Config;
use crate::tui_contributions::PluginThemeContribution;
use codex_utils_absolute_path::AbsolutePathBuf;

const GHOSTTY_BIN_ENV: &str = "GHOSTTY_BIN";
const GHOSTTY_BIN_DEFAULT: &str = "ghostty";
const GHOSTTY_SYNC_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Eq, PartialEq)]
struct GhosttySnapshot {
    theme_spec: Option<String>,
    active_theme_name: Option<String>,
    background: String,
    foreground: String,
    selection_background: Option<String>,
    selection_foreground: Option<String>,
    palette: BTreeMap<u16, String>,
    is_light: bool,
}

#[derive(Default)]
struct ParsedGhosttyConfig {
    theme_spec: Option<String>,
    background: Option<String>,
    foreground: Option<String>,
    selection_background: Option<String>,
    selection_foreground: Option<String>,
    palette: BTreeMap<u16, String>,
}

pub(crate) async fn sync_theme_from_ghostty(
    config: &mut Config,
    contribution: &PluginThemeContribution,
) -> Result<(), String> {
    let output = read_ghostty_config(config.cwd.as_path()).await?;
    let snapshot = read_ghostty_snapshot(&output)?
        .ok_or_else(|| "Ghostty config did not expose background/foreground colors".to_string())?;
    let theme_path = generated_theme_path(config, &contribution.name);
    ensure_generated_theme_file(&theme_path, &contribution.name, &snapshot).await?;

    config.tui_theme = Some(contribution.name.clone());
    let theme = crate::render::highlight::resolve_theme_by_name(
        &contribution.name,
        Some(config.codex_home.as_path()),
    )
    .ok_or_else(|| {
        format!(
            "Generated Ghostty theme could not be loaded from {}",
            theme_path.display()
        )
    })?;
    crate::render::highlight::set_syntax_theme(theme);

    Ok(())
}

async fn read_ghostty_config(cwd: &std::path::Path) -> Result<String, String> {
    let bin = std::env::var_os(GHOSTTY_BIN_ENV)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| OsString::from(GHOSTTY_BIN_DEFAULT));
    let command_display = format!("{} +show-config", bin.to_string_lossy());
    let mut command = tokio::process::Command::new(&bin);
    command.arg("+show-config").current_dir(cwd);

    let output = match tokio::time::timeout(GHOSTTY_SYNC_TIMEOUT, command.output()).await {
        Ok(result) => result.map_err(|err| format!("Failed to run {command_display}: {err}"))?,
        Err(_) => return Err(format!("{command_display} timed out")),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let details = if stderr.is_empty() { stdout } else { stderr };
        let details = if details.is_empty() {
            format!("{command_display} exited with {}", output.status)
        } else {
            details
        };
        return Err(details);
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn read_ghostty_snapshot(output: &str) -> Result<Option<GhosttySnapshot>, String> {
    let parsed = parse_ghostty_config(output);
    let Some(background) = parsed.background else {
        return Ok(None);
    };
    let Some(foreground) = parsed.foreground else {
        return Ok(None);
    };

    let is_light = is_light_background(&background)?;
    let active_theme_name = select_theme_name(parsed.theme_spec.as_deref(), is_light);

    Ok(Some(GhosttySnapshot {
        theme_spec: parsed.theme_spec,
        active_theme_name,
        background,
        foreground,
        selection_background: parsed.selection_background,
        selection_foreground: parsed.selection_foreground,
        palette: parsed.palette,
        is_light,
    }))
}

fn parse_ghostty_config(output: &str) -> ParsedGhosttyConfig {
    let mut config = ParsedGhosttyConfig::default();

    for raw_line in output.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();

        match key {
            "theme" => config.theme_spec = Some(unquote(value).to_string()),
            "background" => config.background = normalize_hex(value).or(config.background),
            "foreground" => config.foreground = normalize_hex(value).or(config.foreground),
            "selection-background" => {
                config.selection_background = normalize_hex(value).or(config.selection_background);
            }
            "selection-foreground" => {
                config.selection_foreground = normalize_hex(value).or(config.selection_foreground);
            }
            "palette" => {
                if let Some((index, color)) = parse_palette_entry(value) {
                    config.palette.insert(index, color);
                }
            }
            _ => {}
        }
    }

    config
}

fn parse_palette_entry(value: &str) -> Option<(u16, String)> {
    let (index, color) = value.split_once('=')?;
    let index = index.trim().parse::<u16>().ok()?;
    let color = normalize_hex(color.trim())?;
    Some((index, color))
}

fn split_theme_spec(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
}

fn select_theme_name(theme_spec: Option<&str>, is_light: bool) -> Option<String> {
    let theme_spec = theme_spec?;
    let mut single_theme = None;
    let mut light_theme = None;
    let mut dark_theme = None;

    for part in split_theme_spec(theme_spec) {
        let Some((mode, name)) = part.split_once(':') else {
            single_theme = Some(part.to_string());
            continue;
        };
        match mode.trim().to_ascii_lowercase().as_str() {
            "light" => light_theme = Some(name.trim().to_string()),
            "dark" => dark_theme = Some(name.trim().to_string()),
            _ => single_theme = Some(part.to_string()),
        }
    }

    if light_theme.is_some() || dark_theme.is_some() {
        if is_light {
            light_theme.or(dark_theme)
        } else {
            dark_theme.or(light_theme)
        }
    } else {
        single_theme
    }
}

fn generated_theme_path(config: &Config, theme_name: &str) -> AbsolutePathBuf {
    config
        .codex_home
        .join("themes")
        .join(format!("{theme_name}.tmTheme"))
}

async fn ensure_generated_theme_file(
    theme_path: &std::path::Path,
    theme_name: &str,
    snapshot: &GhosttySnapshot,
) -> Result<(), String> {
    let contents = build_tmtheme(theme_name, snapshot)?;
    if let Ok(existing) = tokio::fs::read_to_string(theme_path).await
        && existing == contents
    {
        return Ok(());
    }

    let parent = theme_path
        .parent()
        .ok_or_else(|| format!("Theme path has no parent: {}", theme_path.display()))?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    tokio::fs::write(theme_path, contents)
        .await
        .map_err(|err| format!("Failed to write {}: {err}", theme_path.display()))?;
    Ok(())
}

fn palette_color(snapshot: &GhosttySnapshot, index: u16, fallback: &str) -> String {
    snapshot
        .palette
        .get(&index)
        .cloned()
        .unwrap_or_else(|| fallback.to_string())
}

fn build_tmtheme(theme_name: &str, snapshot: &GhosttySnapshot) -> Result<String, String> {
    let bg = snapshot.background.as_str();
    let fg = snapshot.foreground.as_str();
    let red = palette_color(
        snapshot,
        1,
        if snapshot.is_light {
            "#c63b52"
        } else {
            "#ff6b6b"
        },
    );
    let green = palette_color(
        snapshot,
        2,
        if snapshot.is_light {
            "#4d7a36"
        } else {
            "#7bd88f"
        },
    );
    let yellow = palette_color(
        snapshot,
        3,
        if snapshot.is_light {
            "#8a6a35"
        } else {
            "#ffd866"
        },
    );
    let blue = palette_color(
        snapshot,
        4,
        if snapshot.is_light {
            "#286ad9"
        } else {
            "#72b7ff"
        },
    );
    let magenta = palette_color(
        snapshot,
        5,
        if snapshot.is_light {
            "#8845d8"
        } else {
            "#c792ea"
        },
    );
    let cyan = palette_color(
        snapshot,
        6,
        if snapshot.is_light {
            "#00708c"
        } else {
            "#78dce8"
        },
    );
    let dim = mix(fg, bg, if snapshot.is_light { 0.30 } else { 0.34 })?;
    let border = mix(fg, bg, if snapshot.is_light { 0.18 } else { 0.22 })?;
    let insert_bg = mix(&green, bg, if snapshot.is_light { 0.06 } else { 0.10 })?;
    let delete_bg = mix(&red, bg, if snapshot.is_light { 0.06 } else { 0.10 })?;
    let selection_bg = match snapshot.selection_background.clone() {
        Some(selection_background) => selection_background,
        None => mix(&blue, bg, if snapshot.is_light { 0.22 } else { 0.28 })?,
    };
    let selection_fg = snapshot.selection_foreground.as_deref().unwrap_or(fg);
    let active_name = snapshot
        .active_theme_name
        .as_deref()
        .or(snapshot.theme_spec.as_deref())
        .unwrap_or("custom");
    let name = format!("{theme_name} ({active_name})");

    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
<key>name</key><string>{}</string>
<key>settings</key>
<array>
{}
{}
{}
{}
{}
{}
{}
{}
{}
{}
{}
{}
</array>
</dict>
</plist>
"#,
        escape_xml(&name),
        default_setting(fg, bg, &selection_bg, selection_fg, &border),
        scoped_setting("comment", Some(&dim), None, Some("italic")),
        scoped_setting("keyword, storage", Some(&magenta), None, None),
        scoped_setting(
            "entity.name.function, support.function",
            Some(&blue),
            None,
            None
        ),
        scoped_setting("string", Some(&green), None, None),
        scoped_setting(
            "constant.numeric, constant.language",
            Some(&yellow),
            None,
            None
        ),
        scoped_setting("entity.name.type, support.type", Some(&cyan), None, None),
        scoped_setting("variable, keyword.operator", Some(fg), None, None),
        scoped_setting("markup.heading", Some(&blue), None, Some("bold")),
        scoped_setting(
            "markup.raw, markup.raw.inline, markup.fenced_code",
            Some(&green),
            None,
            None
        ),
        scoped_setting(
            "markup.inserted, diff.inserted",
            Some(&green),
            Some(&insert_bg),
            None
        ),
        scoped_setting(
            "markup.deleted, diff.deleted",
            Some(&red),
            Some(&delete_bg),
            None
        ),
    ))
}

fn default_setting(
    foreground: &str,
    background: &str,
    selection_background: &str,
    selection_foreground: &str,
    border: &str,
) -> String {
    format!(
        r#"<dict><key>settings</key><dict>
<key>foreground</key><string>{}</string>
<key>background</key><string>{}</string>
<key>caret</key><string>{}</string>
<key>selection</key><string>{}</string>
<key>selectionForeground</key><string>{}</string>
<key>lineHighlight</key><string>{}</string>
</dict></dict>"#,
        escape_xml(foreground),
        escape_xml(background),
        escape_xml(foreground),
        escape_xml(selection_background),
        escape_xml(selection_foreground),
        escape_xml(border),
    )
}

fn scoped_setting(
    scope: &str,
    foreground: Option<&str>,
    background: Option<&str>,
    font_style: Option<&str>,
) -> String {
    let mut settings = String::new();
    if let Some(foreground) = foreground {
        settings.push_str(&format!(
            "<key>foreground</key><string>{}</string>",
            escape_xml(foreground)
        ));
    }
    if let Some(background) = background {
        settings.push_str(&format!(
            "<key>background</key><string>{}</string>",
            escape_xml(background)
        ));
    }
    if let Some(font_style) = font_style {
        settings.push_str(&format!(
            "<key>fontStyle</key><string>{}</string>",
            escape_xml(font_style)
        ));
    }

    format!(
        r#"<dict><key>scope</key><string>{}</string><key>settings</key><dict>{settings}</dict></dict>"#,
        escape_xml(scope),
    )
}

fn normalize_hex(value: &str) -> Option<String> {
    let value = unquote(value.trim());
    if value.len() == 7
        && value.starts_with('#')
        && value[1..].chars().all(|ch| ch.is_ascii_hexdigit())
    {
        return Some(value.to_ascii_lowercase());
    }
    if value.len() == 6 && value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Some(format!("#{}", value.to_ascii_lowercase()));
    }
    None
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn parse_hex(hex: &str) -> Result<(f64, f64, f64), String> {
    let normalized = normalize_hex(hex).ok_or_else(|| format!("Invalid hex color: {hex}"))?;
    let channel = |range: std::ops::Range<usize>| {
        u8::from_str_radix(&normalized[range], 16)
            .map(f64::from)
            .map_err(|err| format!("Invalid hex color {hex}: {err}"))
    };
    Ok((channel(1..3)?, channel(3..5)?, channel(5..7)?))
}

fn to_hex_channel(value: f64) -> String {
    let value = value.round().clamp(0.0, 255.0) as u8;
    format!("{value:02x}")
}

fn mix(color_a: &str, color_b: &str, ratio_a: f64) -> Result<String, String> {
    let (ar, ag, ab) = parse_hex(color_a)?;
    let (br, bg, bb) = parse_hex(color_b)?;
    let ratio_a = ratio_a.clamp(0.0, 1.0);
    let ratio_b = 1.0 - ratio_a;
    Ok(format!(
        "#{}{}{}",
        to_hex_channel(ar * ratio_a + br * ratio_b),
        to_hex_channel(ag * ratio_a + bg * ratio_b),
        to_hex_channel(ab * ratio_a + bb * ratio_b),
    ))
}

fn relative_luminance(color: &str) -> Result<f64, String> {
    let (r, g, b) = parse_hex(color)?;
    let convert = |channel: f64| {
        let value = channel / 255.0;
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    Ok(0.2126 * convert(r) + 0.7152 * convert(g) + 0.0722 * convert(b))
}

fn is_light_background(color: &str) -> Result<bool, String> {
    Ok(relative_luminance(color)? >= 0.45)
}

#[cfg(test)]
#[path = "ghostty_theme_sync_tests.rs"]
mod tests;
