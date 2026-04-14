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
theme = "catppuccin-mocha"
task_list_sort = "due-asc"
project_list_sort = "manual"
persist_project_list_sort = false
tag_list_sort = "manual"
persist_tag_list_sort = false
filter_list_sort = "manual"
persist_filter_list_sort = false
hide_completed_tasks = true

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
  theme: catppuccin-mocha
  task_list_sort: due-asc
  project_list_sort: manual
  persist_project_list_sort: false
  tag_list_sort: manual
  persist_tag_list_sort: false
  filter_list_sort: manual
  persist_filter_list_sort: false
  hide_completed_tasks: true

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

`ui.theme` selects the active palette.

Built-in themes:

- `catppuccin-latte`
- `catppuccin-frappe`
- `catppuccin-macchiato`
- `catppuccin-mocha`

Default:

- `catppuccin-mocha`

`ui.task_list_sort` controls the default sort order used in panel 5.

Allowed values:

- `due-asc`
- `due-desc`
- `title-asc`
- `title-desc`
- `created-newest`
- `created-oldest`
- `priority-high`
- `priority-low`

Default:

- `due-asc`

`ui.hide_completed_tasks` controls whether completed tasks are hidden in panel
5 by default.

Allowed values:

- `true`
- `false`

Default:

- `true`

`ui.project_list_sort` controls the default project-tree ordering used in panel
5 (Projects tab in the Navigation panel).

Allowed values:

- `name-asc`
- `name-desc`
- `task-count-asc`
- `task-count-desc`
- `manual`

Default:

- `manual`

`ui.persist_project_list_sort` controls startup behavior for project sorting.
When `false` (default), Triginta always starts with `manual` sorting in the
Projects tab, regardless of the previously selected sort. When `true`,
Triginta persists and restores the selected project sort order.

Allowed values:

- `true`
- `false`

Default:

- `false`

`ui.tag_list_sort` controls the default tag-list ordering used in panel 5
(Tags tab in the Navigation panel).

Allowed values:

- `name-asc`
- `name-desc`
- `task-count-asc`
- `task-count-desc`
- `manual`

Default:

- `manual`

`ui.persist_tag_list_sort` controls startup behavior for tag sorting.
When `false` (default), Triginta always starts with `manual` sorting in the
Tags tab, regardless of the previously selected sort. When `true`,
Triginta persists and restores the selected tag sort order.

Allowed values:

- `true`
- `false`

Default:

- `false`

`ui.filter_list_sort` controls the default filter-list ordering used in panel 6
(Filters tab in the Navigation panel).

Allowed values:

- `name-asc`
- `name-desc`
- `task-count-asc`
- `task-count-desc`
- `manual`

Default:

- `manual`

`ui.persist_filter_list_sort` controls startup behavior for filter sorting.
When `false` (default), Triginta always starts with `manual` sorting in the
Filters tab, regardless of the previously selected sort. When `true`,
Triginta persists and restores the selected filter sort order.

Allowed values:

- `true`
- `false`

Default:

- `false`

## Theme Files

Custom themes live in the app config `themes/` directory.

Typical locations:

- Linux: `~/.config/triginta/themes/`
- macOS: `~/Library/Application Support/triginta/themes/`
- Windows: `%APPDATA%\triginta\themes\`

To use a custom theme, set `ui.theme` to the file name without the extension.
For example, `ui.theme = "forest"` loads `themes/forest.toml` or the YAML
equivalent.

Theme files support these keys:

- `text`
- `subtle_text`
- `border`
- `accent`
- `timer_work`
- `timer_short_break`
- `timer_long_break`
- `success`
- `error`
- `priority_1`
- `priority_2`
- `priority_3`

Each value must be a 6-digit hex color like `"#cdd6f4"`.
If `priority_1..priority_3` are omitted, Triginta uses Catppuccin-compatible
defaults that approximate Todoist's red/orange/blue priority colors.

Example:

```toml
text = "#ddeedd"
subtle_text = "#99aa99"
border = "#557755"
accent = "#88cc66"
timer_work = "#77dd77"
timer_short_break = "#66cccc"
timer_long_break = "#4488cc"
success = "#66dd88"
error = "#dd6677"
priority_1 = "#f38ba8"
priority_2 = "#fab387"
priority_3 = "#89b4fa"
```

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

### Reset local debug data

```bash
mise exec -- cargo run -- --reset-data
```

This removes the local debug SQLite files before startup:

- `triginta-dbg.sqlite3`
- `triginta-dbg.sqlite3-wal`
- `triginta-dbg.sqlite3-shm`
