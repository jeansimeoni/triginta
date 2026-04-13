use std::fs;

use anyhow::{Context, Result, anyhow, bail};
use ratatui::style::Color;
use serde::Deserialize;

use crate::config::AppPaths;
use crate::domain::ProjectColor;

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
    pub project_colors: ProjectColorPalette,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectColorPalette {
    pub berry_red: Color,
    pub red: Color,
    pub orange: Color,
    pub yellow: Color,
    pub olive_green: Color,
    pub lime_green: Color,
    pub green: Color,
    pub mint_green: Color,
    pub teal: Color,
    pub sky_blue: Color,
    pub light_blue: Color,
    pub blue: Color,
    pub grape: Color,
    pub violet: Color,
    pub lavender: Color,
    pub magenta: Color,
    pub salmon: Color,
    pub charcoal: Color,
    pub grey: Color,
    pub taupe: Color,
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

    pub fn project_color(self, color: ProjectColor) -> Color {
        match color {
            ProjectColor::BerryRed => self.project_colors.berry_red,
            ProjectColor::Red => self.project_colors.red,
            ProjectColor::Orange => self.project_colors.orange,
            ProjectColor::Yellow => self.project_colors.yellow,
            ProjectColor::OliveGreen => self.project_colors.olive_green,
            ProjectColor::LimeGreen => self.project_colors.lime_green,
            ProjectColor::Green => self.project_colors.green,
            ProjectColor::MintGreen => self.project_colors.mint_green,
            ProjectColor::Teal => self.project_colors.teal,
            ProjectColor::SkyBlue => self.project_colors.sky_blue,
            ProjectColor::LightBlue => self.project_colors.light_blue,
            ProjectColor::Blue => self.project_colors.blue,
            ProjectColor::Grape => self.project_colors.grape,
            ProjectColor::Violet => self.project_colors.violet,
            ProjectColor::Lavender => self.project_colors.lavender,
            ProjectColor::Magenta => self.project_colors.magenta,
            ProjectColor::Salmon => self.project_colors.salmon,
            ProjectColor::Charcoal => self.project_colors.charcoal,
            ProjectColor::Grey => self.project_colors.grey,
            ProjectColor::Taupe => self.project_colors.taupe,
        }
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
    #[serde(default)]
    project_colors: Option<ProjectColorFile>,
}

#[derive(Debug, Clone, Deserialize)]
struct ProjectColorFile {
    berry_red: String,
    red: String,
    orange: String,
    yellow: String,
    olive_green: String,
    lime_green: String,
    green: String,
    mint_green: String,
    teal: String,
    sky_blue: String,
    light_blue: String,
    blue: String,
    grape: String,
    violet: String,
    lavender: String,
    magenta: String,
    salmon: String,
    charcoal: String,
    grey: String,
    taupe: String,
}

impl ThemeFile {
    fn into_palette(self) -> Result<ThemePalette> {
        let project_colors = self
            .project_colors
            .unwrap_or_else(default_project_color_file);
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
            project_colors: ProjectColorPalette {
                berry_red: parse_hex_color(&project_colors.berry_red)
                    .context("invalid project color for berry_red")?,
                red: parse_hex_color(&project_colors.red)
                    .context("invalid project color for red")?,
                orange: parse_hex_color(&project_colors.orange)
                    .context("invalid project color for orange")?,
                yellow: parse_hex_color(&project_colors.yellow)
                    .context("invalid project color for yellow")?,
                olive_green: parse_hex_color(&project_colors.olive_green)
                    .context("invalid project color for olive_green")?,
                lime_green: parse_hex_color(&project_colors.lime_green)
                    .context("invalid project color for lime_green")?,
                green: parse_hex_color(&project_colors.green)
                    .context("invalid project color for green")?,
                mint_green: parse_hex_color(&project_colors.mint_green)
                    .context("invalid project color for mint_green")?,
                teal: parse_hex_color(&project_colors.teal)
                    .context("invalid project color for teal")?,
                sky_blue: parse_hex_color(&project_colors.sky_blue)
                    .context("invalid project color for sky_blue")?,
                light_blue: parse_hex_color(&project_colors.light_blue)
                    .context("invalid project color for light_blue")?,
                blue: parse_hex_color(&project_colors.blue)
                    .context("invalid project color for blue")?,
                grape: parse_hex_color(&project_colors.grape)
                    .context("invalid project color for grape")?,
                violet: parse_hex_color(&project_colors.violet)
                    .context("invalid project color for violet")?,
                lavender: parse_hex_color(&project_colors.lavender)
                    .context("invalid project color for lavender")?,
                magenta: parse_hex_color(&project_colors.magenta)
                    .context("invalid project color for magenta")?,
                salmon: parse_hex_color(&project_colors.salmon)
                    .context("invalid project color for salmon")?,
                charcoal: parse_hex_color(&project_colors.charcoal)
                    .context("invalid project color for charcoal")?,
                grey: parse_hex_color(&project_colors.grey)
                    .context("invalid project color for grey")?,
                taupe: parse_hex_color(&project_colors.taupe)
                    .context("invalid project color for taupe")?,
            },
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
            project_colors: default_project_colors(),
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
            project_colors: default_project_colors(),
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
            project_colors: default_project_colors(),
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
            project_colors: default_project_colors(),
        }),
        _ => None,
    }
}

fn default_project_color_file() -> ProjectColorFile {
    ProjectColorFile {
        berry_red: "#B8255F".to_string(),
        red: "#DC4C3E".to_string(),
        orange: "#C77100".to_string(),
        yellow: "#B29104".to_string(),
        olive_green: "#949C31".to_string(),
        lime_green: "#65A33A".to_string(),
        green: "#369307".to_string(),
        mint_green: "#42A393".to_string(),
        teal: "#148FAD".to_string(),
        sky_blue: "#319DC0".to_string(),
        light_blue: "#6988A4".to_string(),
        blue: "#4180FF".to_string(),
        grape: "#692EC2".to_string(),
        violet: "#CA3FEE".to_string(),
        lavender: "#A4698C".to_string(),
        magenta: "#E05095".to_string(),
        salmon: "#C9766F".to_string(),
        charcoal: "#808080".to_string(),
        grey: "#999999".to_string(),
        taupe: "#8F7A69".to_string(),
    }
}

fn default_project_colors() -> ProjectColorPalette {
    let values = default_project_color_file();
    ProjectColorPalette {
        berry_red: parse_hex_color(&values.berry_red).expect("valid color"),
        red: parse_hex_color(&values.red).expect("valid color"),
        orange: parse_hex_color(&values.orange).expect("valid color"),
        yellow: parse_hex_color(&values.yellow).expect("valid color"),
        olive_green: parse_hex_color(&values.olive_green).expect("valid color"),
        lime_green: parse_hex_color(&values.lime_green).expect("valid color"),
        green: parse_hex_color(&values.green).expect("valid color"),
        mint_green: parse_hex_color(&values.mint_green).expect("valid color"),
        teal: parse_hex_color(&values.teal).expect("valid color"),
        sky_blue: parse_hex_color(&values.sky_blue).expect("valid color"),
        light_blue: parse_hex_color(&values.light_blue).expect("valid color"),
        blue: parse_hex_color(&values.blue).expect("valid color"),
        grape: parse_hex_color(&values.grape).expect("valid color"),
        violet: parse_hex_color(&values.violet).expect("valid color"),
        lavender: parse_hex_color(&values.lavender).expect("valid color"),
        magenta: parse_hex_color(&values.magenta).expect("valid color"),
        salmon: parse_hex_color(&values.salmon).expect("valid color"),
        charcoal: parse_hex_color(&values.charcoal).expect("valid color"),
        grey: parse_hex_color(&values.grey).expect("valid color"),
        taupe: parse_hex_color(&values.taupe).expect("valid color"),
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
