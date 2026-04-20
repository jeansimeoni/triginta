# User Guide

Triginta is a keyboard-first Pomodoro timer and task manager. It is local-first:
your local SQLite database is the source of truth, and a fresh empty database is
a valid first-run state.

Open in-app help with `?` at any time. The footer also shows shortcuts for the
currently focused panel.

## Layout

The TUI is organized into focused panels:

- `1`: timer
- `2`: history
- `3`: navigation views
- `4`: projects and sections
- `5`: tags
- `6`: filters
- `7`: favorites
- `8`: task list and details

Use `Tab` / `Shift+Tab` to move focus, or press `1-8` to jump directly.

## Timer Workflow

Focus the timer panel with `1`.

Common timer keys:

- `s`, `Space`, or `Enter`: start or resume
- `p`: pause
- `x` or `Esc`: void or reset the current timer
- `a`: assign a task to the timer
- `u`: clear the assigned task
- `n`: edit the current session note
- `v`: view the current session note
- `N`: clear the current session note

Completed focus sessions appear in the history panel. Break entries are also
tracked so daily and weekly history can show focus and break time together.

## Task Management

Press `c` from most panels to create a task. Press `8` to focus the task list.

Task-list keys:

- `j/k` or `Up/Down`: move through tasks
- `c`: create a task
- `C`: create a subtask under the selected task
- `e`: edit the selected task
- `d`: delete the selected task
- `Space`: toggle completion
- `a`: assign the selected task to the timer
- `r`: reschedule the due date
- `o`: change sort order
- `f`: hide or show completed tasks
- `=` / `-`: expand or collapse subtasks
- `J/K`: reorder sibling tasks when manual reordering is available
- `/`: search the current task list

The quick-create popup understands inline project, tag, due-date, recurrence,
and priority hints. Use `Tab` to accept suggestions. Use `Ctrl+e` to open the
full editor when the popup is not enough.

## Projects, Sections, Tags, And Filters

Focus navigation with `3-6` or use the sidebar tabs:

- `3`: navigation views such as inbox-style task views
- `4`: projects and sections
- `5`: tags
- `6`: filters

Common list keys:

- `j/k` or `Up/Down`: move selection
- `Home/End`: jump to first or last item
- `PgUp/PgDn`: page through the list
- `Enter`: open the selected scope in the task list
- `/`: search the panel

Projects support:

- `C`: create project
- `s`: create section
- `e/d`: edit or delete project or section
- `o`: sort
- `J/K`: reorder when manual sorting is active
- `f`: toggle favorite
- `c`: create a task in the selected project or section

Tags and filters support:

- `C/e/d`: create, edit, or delete
- `o`: sort
- `J/K`: reorder when manual sorting is active
- `f`: toggle favorite
- `Enter`: open matching tasks

Filters use a Todoist-like query syntax for local task views. See
[Configuration](configuration.md) for filter-related settings and
[NLP Locale Packs](nlp-locales.md) for due-date language behavior.

## Favorites

Favorites collect frequently used projects, tags, and filters.

Focus favorites with `7`.

- `j/k` or `Up/Down`: move favorite
- `Enter`: open the favorite in the task list
- `f`: remove the favorite
- `/`: search favorites

## History And Statistics

Focus history with `2`.

- `h/l` or `Left/Right`: switch history range
- `j/k` or `Up/Down`: move session
- `a`: assign a task to a session
- `u`: clear the session task
- `n/v/N`: edit, view, or clear a session note

The statistics view is available from the right-side panel. Use `h/l` or
`Left/Right` to switch right-side tabs when that panel is focused.

## Configuration

Triginta reads exactly one config file named `config.toml`, `config.yaml`, or
`config.yml` from the app config directory. If multiple config files are found,
startup fails with an explicit error.

Common settings:

- `ui.glyph_mode = "nerd-fonts"` or `"ascii"`
- `ui.theme = "catppuccin-mocha"`
- `timer.pomodoro_length = "25m"`
- `timer.short_break_length = "5m"`
- `timer.long_break_length = "15m"`
- `stats.daily_target = "150m"`

See [Configuration](configuration.md) for the full schema.

## Data And Logging

By default, Triginta uses platform-standard app directories for config, data,
the SQLite database, themes, and logs.

Set `TRIGINTA_DATA_DIR` to isolate a run:

```bash
TRIGINTA_DATA_DIR=/tmp/triginta-test triginta
```

With that variable set, Triginta writes:

- Config: `$TRIGINTA_DATA_DIR/config/config.toml` or YAML equivalent
- Themes: `$TRIGINTA_DATA_DIR/config/themes/`
- Release database: `$TRIGINTA_DATA_DIR/triginta.sqlite3`
- Debug database: `$TRIGINTA_DATA_DIR/triginta-dbg.sqlite3`
- Logs: `$TRIGINTA_DATA_DIR/logs/triginta.log`

## Todoist Sync Status

Todoist integration settings exist, and the sync boundary is implemented around
local state, an outbox, token loading, and remote transport. The app remains
usable without Todoist, and local SQLite remains the source of truth.

Treat Todoist sync as an integration feature that may still need careful review
before relying on it for critical task data.
