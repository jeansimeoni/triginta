# Configuration

Triginta uses a single application config file stored in the platform-standard
config directory for the current user.

Typical locations:

- Linux: `~/.config/triginta/config.toml`
- macOS: `~/Library/Application Support/triginta/config.toml`
- Windows: `%APPDATA%\triginta\config.toml`

Triginta supports exactly one config file at a time. Supported formats:

- `config.toml`
- `config.yaml`
- `config.yml`

If more than one of those files exists in the config directory, startup fails
with an explicit error so the active config stays unambiguous.

## Structure

The config is organized by sections.

### TOML example

```toml
[ui]
glyph_mode = "nerd-fonts"

[timer]
pomodoro_length = "25m"
short_break_length = "5m"
long_break_length = "15m"
long_break_interval = 4
```

### YAML example

```yaml
ui:
  glyph_mode: nerd-fonts

timer:
  pomodoro_length: 25m
  short_break_length: 5m
  long_break_length: 15m
  long_break_interval: 4
```

## UI

`ui.glyph_mode` controls whether the interface prefers Nerd Font glyphs or
plain ASCII fallback.

Allowed values:

- `nerd-fonts`
- `ascii`

Default:

- `nerd-fonts`

## Timer

The `timer` section controls pomodoro and break durations plus the long-break
cycle.

Fields:

- `pomodoro_length`
- `short_break_length`
- `long_break_length`
- `long_break_interval`

Duration fields accept either:

- An integer number of seconds, such as `1500`
- A duration string using `s`, `m`, or `h`, such as `30s`, `25m`, or `1h`

Defaults:

- `pomodoro_length = "25m"`
- `short_break_length = "5m"`
- `long_break_length = "15m"`
- `long_break_interval = 4`

## Debug Overrides

Debug builds support CLI flags that override file configuration for local
testing.

### ASCII mode

```bash
mise exec -- cargo run -- --ascii
```

This forces `ui.glyph_mode = "ascii"` regardless of config file contents.

### Short timer mode

```bash
mise exec -- cargo run -- --short-timer
```

This forces:

- `pomodoro_length = "30s"`
- `short_break_length = "10s"`
- `long_break_length = "20s"`
- `long_break_interval = 4`

You can combine both debug overrides:

```bash
mise exec -- cargo run -- --ascii --short-timer
```
