use std::fs;

use anyhow::{Context, Result, anyhow, bail};
use ratatui::style::Color;
use serde::Deserialize;

use crate::config::AppPaths;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemePalette {
    pub text: Color,
    pub subtle_text: Color,
    pub border: Color,
    pub accent: Color,
    pub timer_work: Color,
    pub timer_short_break: Color,
    pub timer_long_break: Color,
    pub success: Color,
    pub error: Color,
}

impl ThemePalette {
    pub fn load(paths: &AppPaths, theme_name: &str) -> Result<Self> {
        if let Some(theme) = builtin_theme(theme_name) {
            return Ok(theme);
        }

        let candidates = [
            paths.themes_dir.join(format!("{theme_name}.toml")),
            paths.themes_dir.join(format!("{theme_name}.yaml")),
            paths.themes_dir.join(format!("{theme_name}.yml")),
        ];

        let Some(theme_path) = candidates.iter().find(|path| path.exists()) else {
            bail!(
                "unknown theme '{theme_name}'. Use a built-in Catppuccin theme or create {}.toml/.yaml in {}",
                theme_name,
                paths.themes_dir.display()
            );
        };

        let theme_text = fs::read_to_string(theme_path)
            .with_context(|| format!("failed to read theme at {}", theme_path.display()))?;

        let theme_file = match theme_path.extension().and_then(|ext| ext.to_str()) {
            Some("toml") => toml::from_str::<ThemeFile>(&theme_text).with_context(|| {
                format!("failed to parse TOML theme at {}", theme_path.display())
            })?,
            Some("yaml" | "yml") => {
                serde_yaml::from_str::<ThemeFile>(&theme_text).with_context(|| {
                    format!("failed to parse YAML theme at {}", theme_path.display())
                })?
            }
            Some(other) => bail!("unsupported theme file extension: {other}"),
            None => bail!("theme file {} has no extension", theme_path.display()),
        };

        theme_file.into_palette()
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ThemeFile {
    text: String,
    subtle_text: String,
    border: String,
    accent: String,
    timer_work: String,
    timer_short_break: String,
    timer_long_break: String,
    success: String,
    error: String,
}

impl ThemeFile {
    fn into_palette(self) -> Result<ThemePalette> {
        Ok(ThemePalette {
            text: parse_hex_color(&self.text).context("invalid theme color for text")?,
            subtle_text: parse_hex_color(&self.subtle_text)
                .context("invalid theme color for subtle_text")?,
            border: parse_hex_color(&self.border).context("invalid theme color for border")?,
            accent: parse_hex_color(&self.accent).context("invalid theme color for accent")?,
            timer_work: parse_hex_color(&self.timer_work)
                .context("invalid theme color for timer_work")?,
            timer_short_break: parse_hex_color(&self.timer_short_break)
                .context("invalid theme color for timer_short_break")?,
            timer_long_break: parse_hex_color(&self.timer_long_break)
                .context("invalid theme color for timer_long_break")?,
            success: parse_hex_color(&self.success).context("invalid theme color for success")?,
            error: parse_hex_color(&self.error).context("invalid theme color for error")?,
        })
    }
}

fn builtin_theme(name: &str) -> Option<ThemePalette> {
    match name {
        "catppuccin-latte" => Some(ThemePalette {
            text: rgb(76, 79, 105),
            subtle_text: rgb(124, 127, 147),
            border: rgb(156, 160, 176),
            accent: rgb(136, 57, 239),
            timer_work: rgb(64, 160, 43),
            timer_short_break: rgb(4, 165, 229),
            timer_long_break: rgb(32, 159, 181),
            success: rgb(64, 160, 43),
            error: rgb(210, 15, 57),
        }),
        "catppuccin-frappe" => Some(ThemePalette {
            text: rgb(198, 208, 245),
            subtle_text: rgb(165, 173, 206),
            border: rgb(115, 121, 148),
            accent: rgb(202, 158, 230),
            timer_work: rgb(166, 209, 137),
            timer_short_break: rgb(153, 209, 219),
            timer_long_break: rgb(133, 193, 220),
            success: rgb(166, 209, 137),
            error: rgb(231, 130, 132),
        }),
        "catppuccin-macchiato" => Some(ThemePalette {
            text: rgb(202, 211, 245),
            subtle_text: rgb(165, 173, 203),
            border: rgb(110, 115, 141),
            accent: rgb(198, 160, 246),
            timer_work: rgb(166, 218, 149),
            timer_short_break: rgb(145, 215, 227),
            timer_long_break: rgb(125, 196, 228),
            success: rgb(166, 218, 149),
            error: rgb(237, 135, 150),
        }),
        "catppuccin-mocha" => Some(ThemePalette {
            text: rgb(205, 214, 244),
            subtle_text: rgb(166, 173, 200),
            border: rgb(108, 112, 134),
            accent: rgb(203, 166, 247),
            timer_work: rgb(166, 227, 161),
            timer_short_break: rgb(137, 220, 235),
            timer_long_break: rgb(116, 199, 236),
            success: rgb(166, 227, 161),
            error: rgb(243, 139, 168),
        }),
        _ => None,
    }
}

fn rgb(red: u8, green: u8, blue: u8) -> Color {
    Color::Rgb(red, green, blue)
}

fn parse_hex_color(value: &str) -> Result<Color> {
    let hex = value.trim().strip_prefix('#').unwrap_or(value.trim());
    if hex.len() != 6 {
        bail!("expected a 6-digit hex color, got '{value}'");
    }

    let red = u8::from_str_radix(&hex[0..2], 16)
        .map_err(|_| anyhow!("expected a valid hex color, got '{value}'"))?;
    let green = u8::from_str_radix(&hex[2..4], 16)
        .map_err(|_| anyhow!("expected a valid hex color, got '{value}'"))?;
    let blue = u8::from_str_radix(&hex[4..6], 16)
        .map_err(|_| anyhow!("expected a valid hex color, got '{value}'"))?;

    Ok(rgb(red, green, blue))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use ratatui::style::Color;

    use crate::config::AppPaths;

    use super::ThemePalette;

    #[test]
    fn loads_builtin_catppuccin_theme() {
        let base = tempfile::tempdir().expect("tempdir should be created");
        let paths =
            AppPaths::from_data_dir(base.path().to_path_buf()).expect("paths should resolve");

        let palette =
            ThemePalette::load(&paths, "catppuccin-mocha").expect("built-in theme should load");
        assert_eq!(palette.accent, Color::Rgb(203, 166, 247));
    }

    #[test]
    fn loads_custom_theme_from_toml_file() {
        let base = tempfile::tempdir().expect("tempdir should be created");
        let paths =
            AppPaths::from_data_dir(base.path().to_path_buf()).expect("paths should resolve");
        paths.ensure_dirs().expect("dirs should exist");

        fs::write(
            paths.themes_dir.join("forest.toml"),
            r##"
text = "#ddeedd"
subtle_text = "#99aa99"
border = "#557755"
accent = "#88cc66"
timer_work = "#77dd77"
timer_short_break = "#66cccc"
timer_long_break = "#4488cc"
success = "#66dd88"
error = "#dd6677"
"##,
        )
        .expect("theme should be written");

        let palette = ThemePalette::load(&paths, "forest").expect("custom theme should load");
        assert_eq!(palette.border, Color::Rgb(85, 119, 85));
        assert_eq!(palette.error, Color::Rgb(221, 102, 119));
    }
}
