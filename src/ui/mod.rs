use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
        calendar::{CalendarEventStore, Monthly},
    },
};
use time::{Date as TimeDate, Month as TimeMonth};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    app::{
        App, CalendarPickerView, CycleEntryState, DeleteConfirmationView, HistoryPanelTab,
        PanelFocus, RightPanelTab, ScreenData, ShortcutSection, ShortcutTip, TaskEditorView,
        TaskInputView, TaskSearchView, TaskSortPopupView, TaskView, TimerPhase,
    },
    config::GlyphMode,
    domain::{DayHistorySummary, SessionEntry, SessionKind, SessionOutcome, Task, TaskStatus},
    theme::ThemePalette,
};
use chrono::{Datelike, Local, NaiveDate};

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let symbols = Symbols::new(app.glyph_mode());
    let palette = app.theme();
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(2)])
        .split(frame.area());

    render_body(frame, app, layout[0], symbols, palette);
    render_status_bar(frame, app, layout[1], palette);
    render_task_overlay(frame, app, symbols, palette);
}

fn render_body(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    symbols: Symbols,
    palette: ThemePalette,
) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    render_left_column(frame, app, columns[0], symbols, palette);
    render_right_panel(frame, app, columns[1], symbols, palette);
}

fn render_left_column(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    symbols: Symbols,
    palette: ThemePalette,
) {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(26),
            Constraint::Percentage(22),
            Constraint::Percentage(30),
            Constraint::Percentage(22),
        ])
        .split(area);

    render_timer_panel(frame, app, sections[0], symbols, palette);
    render_history_panel(frame, app, sections[1], symbols, palette);
    render_navigation_panel(
        frame,
        app,
        sections[2],
        symbols,
        app.focused_panel(),
        palette,
    );
    render_favorites_panel(
        frame,
        app.screen_data(),
        sections[3],
        symbols,
        app.focused_panel(),
        palette,
    );
}

fn render_right_panel(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    symbols: Symbols,
    palette: ThemePalette,
) {
    match app.active_right_panel_tab() {
        RightPanelTab::Tasks => {
            render_tasks_workspace(frame, app, area, symbols, app.focused_panel(), palette)
        }
        RightPanelTab::Statistics => render_statistics_panel(
            frame,
            app.screen_data(),
            area,
            symbols,
            app.focused_panel(),
            palette,
        ),
    }
}

fn render_tasks_workspace(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    symbols: Symbols,
    focused_panel: PanelFocus,
    palette: ThemePalette,
) {
    let sections = task_workspace_sections(area);

    render_task_list_panel(frame, app, sections[0], symbols, focused_panel, palette);
    render_task_details_panel(frame, app, sections[1], symbols, palette);
}

fn render_timer_panel(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    symbols: Symbols,
    palette: ThemePalette,
) {
    let timer = app.timer_view();
    let block = panel_block(
        Line::from(format!("[1] Pomodoro")),
        app.focused_panel() == PanelFocus::Timer,
        palette,
    );
    let inner = block.inner(area);
    let content = inner.inner(Margin {
        vertical: 0,
        horizontal: 1,
    });
    frame.render_widget(block, area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(content);

    let headline = Paragraph::new(vec![Line::from(vec![Span::styled(
        format!("{} {}", symbols.timer, timer.run_state.label(timer.phase)),
        Style::default()
            .fg(timer_color(timer.phase, palette))
            .add_modifier(Modifier::BOLD),
    )])]);

    let progress = Paragraph::new(Line::from(progress_bar(&timer, symbols, content.width)))
        .style(Style::default().fg(timer_color(timer.phase, palette)))
        .wrap(Wrap { trim: true });

    let progress_meta = Paragraph::new(progress_meta_line(&timer, content.width, palette));
    let assigned_task = Paragraph::new(assigned_task_line(app, symbols, palette, content.width));
    let cycle = Paragraph::new(cycle_line(timer.cycle_entries.as_slice(), symbols, palette));

    frame.render_widget(headline, sections[0]);
    frame.render_widget(progress, sections[1]);
    frame.render_widget(progress_meta, sections[2]);
    frame.render_widget(cycle, sections[4]);
    frame.render_widget(Paragraph::new(""), sections[5]);
    frame.render_widget(assigned_task, sections[6]);
}

fn render_history_panel(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    symbols: Symbols,
    palette: ThemePalette,
) {
    let data = app.screen_data();
    let today_selected = app.history_scroll();
    let (summary, lines, right_indicator) = match app.active_history_panel_tab() {
        HistoryPanelTab::Today => {
            let rows = history_rows(data.history_entries.as_slice(), data.tasks.as_slice());
            let selected = today_selected.min(rows.len().saturating_sub(1));
            let summary = Line::from(format!(
                "{} focus  |  {} break  |  {} sessions",
                format_duration_seconds(data.today_stats.total_work_seconds),
                format_duration_seconds(data.today_stats.total_break_seconds),
                data.today_stats.total_sessions,
            ))
            .right_aligned();
            let lines = if rows.is_empty() {
                vec![Line::from("No pomodoros recorded today.")]
            } else {
                let show_selection = app.focused_panel() == PanelFocus::History;
                let visible_height = area.height.saturating_sub(2) as usize;
                let start = selected.saturating_sub(visible_height.saturating_sub(1));
                let end = (start + visible_height).min(rows.len());
                rows[start..end]
                    .iter()
                    .enumerate()
                    .map(|(index, row)| {
                        format_history_row(
                            row,
                            symbols,
                            palette,
                            area.width.saturating_sub(4),
                            show_selection && start + index == selected,
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let indicator = if rows.len() > area.height.saturating_sub(2) as usize {
                Some((rows.len(), selected))
            } else {
                None
            };
            (summary, lines, indicator)
        }
        HistoryPanelTab::Last7Days => (
            Line::from(format!(
                "{} focus  |  {} break  |  {} sessions",
                format_duration_seconds(data.weekly_stats.total_work_seconds),
                format_duration_seconds(data.weekly_stats.total_break_seconds),
                data.weekly_stats.total_sessions,
            ))
            .right_aligned(),
            render_weekly_history_lines(data.weekly_summaries.as_slice(), palette),
            None,
        ),
    };
    let block = panel_block(
        history_title(app.active_history_panel_tab(), palette),
        app.focused_panel() == PanelFocus::History,
        palette,
    )
    .title_bottom(summary);
    let inner = block.inner(area);
    let content = inner.inner(Margin {
        vertical: 0,
        horizontal: 1,
    });
    frame.render_widget(block, area);

    let history = Paragraph::new(lines);
    frame.render_widget(history, content);

    if let Some((content_length, position)) = right_indicator {
        let mut scrollbar_state = ScrollbarState::default()
            .content_length(content_length)
            .position(position);
        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(None)
            .thumb_symbol("▐")
            .thumb_style(Style::default().fg(palette.subtle_text));
        frame.render_stateful_widget(scrollbar, inner, &mut scrollbar_state);
    }
}

fn render_navigation_panel(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    symbols: Symbols,
    focused_panel: PanelFocus,
    palette: ThemePalette,
) {
    let content_width = area.width.saturating_sub(2);
    let lines = TaskView::all()
        .iter()
        .map(|view| {
            let selected = app.active_task_view() == *view;
            selectable_line(
                &format!("{} {}", task_view_symbol(*view, symbols), view.label()),
                selected,
                content_width,
                palette,
            )
        })
        .collect::<Vec<_>>();

    let task_count = app.visible_tasks().len();
    let summary = Line::from(format!(
        "{}  |  {} tasks",
        app.active_task_view().label(),
        task_count
    ))
    .right_aligned();

    let content = Paragraph::new(lines)
        .block(
            panel_block(
                navigation_title(symbols, palette),
                focused_panel == PanelFocus::Navigation,
                palette,
            )
            .title_bottom(summary),
        )
        .wrap(Wrap { trim: true });

    frame.render_widget(content, area);
}

fn render_favorites_panel(
    frame: &mut Frame<'_>,
    data: &ScreenData,
    area: Rect,
    symbols: Symbols,
    focused_panel: PanelFocus,
    palette: ThemePalette,
) {
    let favorites = favorite_tasks(data.tasks.as_slice());
    let content_width = area.width.saturating_sub(2);
    let mut lines = vec![];

    if favorites.is_empty() {
        lines.push(Line::from("No favorites yet."));
        lines.push(Line::from("Pinned tasks or saved searches can live here."));
    } else {
        for task in favorites {
            lines.push(Line::from(ellipsize_end(
                &format!("{} {}", symbols.favorite, task.title),
                content_width as usize,
            )));
        }
    }

    let panel = Paragraph::new(lines)
        .block(panel_block(
            Line::from("[4] Favorites"),
            focused_panel == PanelFocus::Favorites,
            palette,
        ))
        .wrap(Wrap { trim: true });

    frame.render_widget(panel, area);
}

fn render_task_list_panel(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    symbols: Symbols,
    focused_panel: PanelFocus,
    palette: ThemePalette,
) {
    let visible_tasks = app.visible_tasks();
    let content_width = area.width.saturating_sub(2);
    let mut lines = vec![];

    if visible_tasks.is_empty() {
        match app.active_task_view() {
            TaskView::Today => {
                lines.push(Line::from("No tasks in Today yet."));
                lines.push(Line::from("Tasks due today will appear here."));
            }
            TaskView::Soon => {
                lines.push(Line::from("No tasks in Soon yet."));
                lines.push(Line::from("Upcoming tasks will appear here."));
            }
            TaskView::All | TaskView::Inbox => {
                lines.push(Line::from("No tasks yet."));
                lines.push(Line::from("Press c to create your first task."));
            }
        }
    } else {
        let show_selection = focused_panel == PanelFocus::RightPane;
        for task in visible_tasks.iter().take(12) {
            let selected =
                show_selection && app.selected_task().map(|selected| selected.id) == Some(task.id);
            lines.push(task_summary_line(
                task,
                symbols,
                palette,
                selected,
                content_width,
            ));
        }
    }

    let footer = task_list_footer(app, symbols, palette);
    let footer_hints =
        task_list_footer_hints(symbols, focused_panel == PanelFocus::RightPane, palette);
    let tasks = Paragraph::new(lines)
        .block(
            panel_block(
                right_panel_title(RightPanelTab::Tasks, symbols, palette),
                focused_panel == PanelFocus::RightPane,
                palette,
            )
            .title_bottom(footer)
            .title_bottom(footer_hints),
        )
        .wrap(Wrap { trim: true });

    frame.render_widget(tasks, area);
}

fn render_task_details_panel(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    symbols: Symbols,
    palette: ThemePalette,
) {
    let lines = if let Some(task) = app.task_details_task() {
        let mut lines = vec![
            Line::from(vec![Span::styled(
                format!(
                    "{} {}",
                    task_status_symbol(task.status, symbols),
                    task.title
                ),
                Style::default().add_modifier(Modifier::BOLD),
            )]),
            Line::from(""),
            Line::from(format!(
                "Created: {}",
                task.created_at.format("%Y-%m-%d %H:%M")
            )),
        ];
        if let Some(due) = &task.due {
            let due_text = due
                .datetime
                .map(|datetime| datetime.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| due.date.format("%Y-%m-%d").to_string());
            lines.push(Line::from(format!("Due: {due_text}")));
            if due.is_recurring {
                lines.push(Line::from(format!(
                    "Recurring: {}",
                    format_recurring_rule(due.string.as_str())
                )));
            }
        }
        lines
    } else {
        Vec::new()
    };

    let details = Paragraph::new(lines)
        .block(
            Block::default()
                .title(Span::styled(
                    format!("{} Task Details", symbols.details),
                    Style::default().fg(palette.accent),
                ))
                .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
                .border_style(Style::default().fg(palette.border)),
        )
        .wrap(Wrap { trim: true });

    frame.render_widget(details, area);
}

fn task_status_symbol(status: TaskStatus, symbols: Symbols) -> &'static str {
    match status {
        TaskStatus::Todo => symbols.todo,
        TaskStatus::Done => symbols.done,
    }
}

fn render_statistics_panel(
    frame: &mut Frame<'_>,
    data: &ScreenData,
    area: Rect,
    symbols: Symbols,
    focused_panel: PanelFocus,
    palette: ThemePalette,
) {
    let completed_width = 24usize;
    let total_minutes = data.today_stats.total_minutes;
    let goal_minutes = 150u32;
    let filled = ((total_minutes.min(goal_minutes) as f32 / goal_minutes as f32)
        * completed_width as f32)
        .round() as usize;
    let graph = format!(
        "[{}{}]",
        "#".repeat(filled),
        ".".repeat(completed_width.saturating_sub(filled))
    );

    let lines = vec![
        Line::from(vec![Span::styled(
            format!("{} Pomodoro Statistics", symbols.stats),
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(format!(
            "Sessions today: {}",
            data.today_stats.total_sessions
        )),
        Line::from(format!("Focused minutes: {}", total_minutes)),
        Line::from(format!(
            "Completed tasks: {}",
            data.today_stats.completed_tasks
        )),
        Line::from(""),
        Line::from(format!("Daily goal      {}", graph)),
        Line::from(format!("{total_minutes} / {goal_minutes} minutes")),
        Line::from(""),
        Line::from("This tab is reserved for charts, streaks,"),
        Line::from("distributions, and longer-term summaries."),
    ];

    let stats = Paragraph::new(lines)
        .block(panel_block(
            right_panel_title(RightPanelTab::Statistics, symbols, palette),
            focused_panel == PanelFocus::RightPane,
            palette,
        ))
        .wrap(Wrap { trim: true });

    frame.render_widget(stats, area);
}

fn render_status_bar(frame: &mut Frame<'_>, app: &App, area: Rect, palette: ThemePalette) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(palette.border));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 {
        return;
    }

    let left = Line::from(vec![Span::styled(
        app.app_name(),
        Style::default()
            .fg(palette.accent)
            .add_modifier(Modifier::BOLD),
    )]);
    frame.render_widget(Paragraph::new(left), inner);

    let right_width = app
        .donate_label()
        .width()
        .saturating_add(app.app_version().width())
        .saturating_add(5) as u16;
    let left_width = app.app_name().width() as u16;
    let gutter = 2u16;

    let right_x = inner
        .x
        .saturating_add(inner.width.saturating_sub(right_width.min(inner.width)));
    let right_area = Rect::new(
        right_x,
        inner.y,
        inner.width.saturating_sub(right_x - inner.x),
        1,
    );
    let right = Line::from(vec![
        Span::styled(app.donate_label(), Style::default().fg(palette.accent)),
        Span::raw("  "),
        Span::styled(app.app_version(), Style::default().fg(palette.subtle_text)),
    ])
    .right_aligned();
    frame.render_widget(Paragraph::new(right), right_area);

    let center_x = inner.x.saturating_add(left_width.saturating_add(gutter));
    let reserved_right = right_width.saturating_add(gutter);
    let center_width = inner.width.saturating_sub(
        left_width
            .saturating_add(reserved_right)
            .saturating_add(gutter),
    );

    if center_width == 0 || center_x >= right_x {
        return;
    }

    let center_area = Rect::new(center_x, inner.y, center_width, 1);
    let center_text = footer_shortcuts_line(app, center_width as usize);
    if !center_text.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(center_text).centered())
                .style(Style::default().fg(palette.subtle_text)),
            center_area,
        );
    }
}

fn favorite_tasks(tasks: &[Task]) -> Vec<&Task> {
    tasks
        .iter()
        .filter(|task| task.deleted_at.is_none() && task.status != TaskStatus::Done)
        .take(3)
        .collect()
}

fn task_summary_line(
    task: &Task,
    symbols: Symbols,
    palette: ThemePalette,
    selected: bool,
    width: u16,
) -> Line<'static> {
    let marker = match task.status {
        TaskStatus::Todo => symbols.todo,
        TaskStatus::Done => symbols.done,
    };
    let now = Local::now();
    let due_text = task
        .due
        .as_ref()
        .map(|due| format_due_label(due, now.date_naive()));
    let recurring_marker = task
        .due
        .as_ref()
        .filter(|due| due.is_recurring)
        .map(|_| symbols.recurring);
    let leading_padding = 2usize;
    let due_gap = if due_text.is_some() { 2usize } else { 0usize };

    let due_meta_width = due_text
        .as_ref()
        .map(|text| {
            text.width()
                + recurring_marker
                    .map(|marker| marker.width() + 1)
                    .unwrap_or(0)
        })
        .unwrap_or(0);
    let left_width = (width as usize)
        .saturating_sub(leading_padding)
        .saturating_sub(due_meta_width)
        .saturating_sub(due_gap);
    let title_text = ellipsize_end(&format!("{marker} {}", task.title), left_width);

    let row_style = task_row_style(task, palette, selected, now);
    let due_style = task_due_style(task, palette, selected, now);
    let recurring_style = task_recurring_style(task, palette, selected, now);
    let mut spans = vec![
        Span::styled(" ".repeat(leading_padding), row_style),
        Span::styled(title_text, row_style),
    ];

    if due_meta_width > 0 {
        let current_width = Line::from(spans.clone()).width();
        let padding = (width as usize)
            .saturating_sub(due_meta_width)
            .saturating_sub(current_width);
        if padding > 0 {
            spans.push(Span::styled(" ".repeat(padding), row_style));
        } else if due_gap > 0 {
            spans.push(Span::styled(" ".repeat(due_gap), row_style));
        }

        if let Some(recurring) = recurring_marker {
            spans.push(Span::styled(recurring.to_string(), recurring_style));
            spans.push(Span::styled(" ".to_string(), row_style));
        }
        if let Some(text) = due_text {
            spans.push(Span::styled(text, due_style));
        }
    }

    if selected {
        let current_width = Line::from(spans.clone()).width();
        let padding = (width as usize).saturating_sub(current_width);
        if padding > 0 {
            spans.push(Span::styled(" ".repeat(padding), row_style));
        }
    }

    Line::from(spans)
}

fn render_task_overlay(frame: &mut Frame<'_>, app: &App, symbols: Symbols, palette: ThemePalette) {
    if app.is_help_open() {
        render_help_dialog(frame, app, palette);
        return;
    }

    if let Some(sort_popup) = app.task_sort_popup_view() {
        let anchor = task_sort_popup_anchor(frame.area());
        render_task_sort_popup(frame, &sort_popup, anchor, symbols, palette);
        return;
    }

    if let Some(search) = app.task_search_view() {
        render_task_search_popup(frame, &search, palette);
        return;
    }

    if let Some(editor) = app.task_editor_view() {
        render_task_editor_popup(frame, &editor, symbols, palette);
        return;
    }

    if let Some(input) = app.task_input_view() {
        render_task_input_popup(frame, &input, symbols, palette);
        return;
    }

    if let Some(confirmation) = app.delete_confirmation_view() {
        render_delete_confirmation(frame, &confirmation, palette);
    }
}

fn task_workspace_sections(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area)
        .to_vec()
}

fn task_sort_popup_anchor(area: Rect) -> Rect {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);
    let right_sections = task_workspace_sections(columns[1]);
    let task_list_area = right_sections[0];

    Rect::new(
        task_list_area
            .x
            .saturating_add(task_list_area.width.saturating_sub(20)),
        task_list_area
            .y
            .saturating_add(task_list_area.height.saturating_sub(2)),
        18,
        1,
    )
}

fn render_help_dialog(frame: &mut Frame<'_>, app: &App, palette: ThemePalette) {
    let sections = app.help_sections();
    let lines = help_lines(
        sections.as_slice(),
        frame.area().width.saturating_sub(4) as usize,
        palette,
    );
    let max_height = frame.area().height.saturating_sub(4).max(6);
    let desired_height = (lines.len().saturating_add(2) as u16).min(max_height);
    let area = centered_rect(frame.area(), 84, desired_height);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(Span::styled(
            "Keyboard Shortcuts",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::from("j/k or PgUp/PgDn scroll  Esc or ? closes").right_aligned())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.accent));
    let inner = block.inner(area);
    let visible_height = inner.height as usize;
    let start = app
        .help_scroll()
        .min(lines.len().saturating_sub(visible_height.max(1)));
    let end = (start + visible_height).min(lines.len());
    let visible_lines = if start < end {
        lines[start..end].to_vec()
    } else {
        Vec::new()
    };
    let popup = Paragraph::new(visible_lines).block(block);

    frame.render_widget(popup, area);

    if lines.len() > visible_height {
        let mut scrollbar_state = ScrollbarState::default()
            .content_length(lines.len())
            .position(start);
        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(None)
            .thumb_symbol("▐")
            .thumb_style(Style::default().fg(palette.subtle_text));
        frame.render_stateful_widget(scrollbar, inner, &mut scrollbar_state);
    }
}

fn help_lines(
    sections: &[ShortcutSection],
    width: usize,
    palette: ThemePalette,
) -> Vec<Line<'static>> {
    let key_width = sections
        .iter()
        .flat_map(|section| section.tips.iter())
        .map(|tip| tip.keys.width())
        .max()
        .unwrap_or(0)
        .min(width.saturating_sub(4));
    let mut lines = Vec::new();

    for (index, section) in sections.iter().enumerate() {
        if index > 0 {
            lines.push(Line::from(""));
        }

        lines.push(Line::from(vec![Span::styled(
            section.title,
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));

        for tip in section.tips {
            let padding = " ".repeat(key_width.saturating_sub(tip.keys.width()));
            let text = format!("  {}{}  {}", tip.keys, padding, tip.description);
            lines.push(Line::from(ellipsize_end(
                text.as_str(),
                width.saturating_sub(1),
            )));
        }
    }

    lines
}

fn footer_shortcuts_line(app: &App, width: usize) -> String {
    let mut tips = Vec::new();
    tips.extend_from_slice(&[
        ShortcutTip {
            keys: "1-5",
            description: "focus",
        },
        ShortcutTip {
            keys: "Tab",
            description: "next panel",
        },
        ShortcutTip {
            keys: "?",
            description: "help",
        },
        ShortcutTip {
            keys: "q",
            description: "quit",
        },
    ]);
    tips.extend_from_slice(app.focused_panel_shortcuts());

    let mut parts = Vec::new();
    for tip in tips {
        parts.push(format!("{} {}", tip.keys, tip.description));
    }

    fit_footer_parts(parts.as_slice(), width)
}

fn fit_footer_parts(parts: &[String], width: usize) -> String {
    let separator = "  ·  ";
    let mut rendered = String::new();

    for part in parts {
        let candidate = if rendered.is_empty() {
            part.clone()
        } else {
            format!("{rendered}{separator}{part}")
        };

        if candidate.width() > width {
            break;
        }

        rendered = candidate;
    }

    rendered
}

fn render_task_input_popup(
    frame: &mut Frame<'_>,
    input: &TaskInputView,
    _symbols: Symbols,
    palette: ThemePalette,
) {
    let show_due_preview = input.due_preview.is_some();
    let total_height = if show_due_preview { 9 } else { 3 };
    let area = centered_rect(frame.area(), 72, total_height);
    frame.render_widget(Clear, area);

    if show_due_preview {
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Length(6)])
            .split(area);
        render_task_input_box(frame, sections[0], input, palette);
        render_task_due_preview(frame, sections[1], input, palette);
    } else {
        render_task_input_box(frame, area, input, palette);
    }
}

fn render_task_input_box(
    frame: &mut Frame<'_>,
    area: Rect,
    input: &TaskInputView,
    palette: ThemePalette,
) {
    let visible_width = area.width.saturating_sub(4) as usize;
    let lines = vec![Line::from(input_window_text(
        &input.value,
        input.cursor,
        visible_width,
    ))];
    let popup = Paragraph::new(lines).block(
        Block::default()
            .title(Span::styled(
                input.title,
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.accent)),
    );

    frame.render_widget(popup, area);
}

fn render_task_due_preview(
    frame: &mut Frame<'_>,
    area: Rect,
    input: &TaskInputView,
    palette: ThemePalette,
) {
    let Some(due_preview) = &input.due_preview else {
        return;
    };

    let mut lines = vec![Line::from(vec![
        Span::styled("Due Date: ", Style::default().fg(palette.subtle_text)),
        Span::styled(
            due_preview.date.format("%Y-%m-%d").to_string(),
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
    ])];
    if let Some(datetime) = due_preview.datetime {
        lines.push(Line::from(vec![
            Span::styled("Due Time: ", Style::default().fg(palette.subtle_text)),
            Span::styled(
                datetime.format("%H:%M").to_string(),
                Style::default().fg(palette.text),
            ),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled("Recurring: ", Style::default().fg(palette.subtle_text)),
        Span::styled(
            if due_preview.is_recurring {
                "yes"
            } else {
                "no"
            },
            Style::default().fg(palette.text),
        ),
    ]));
    if due_preview.string.to_ascii_lowercase()
        != due_preview
            .datetime
            .map(|_| {
                format!(
                    "{} {}",
                    due_preview.date.format("%Y-%m-%d"),
                    due_preview.datetime.expect("checked above").format("%H:%M")
                )
            })
            .unwrap_or_else(|| due_preview.date.format("%Y-%m-%d").to_string())
    {
        lines.push(Line::from(vec![
            Span::styled("From: ", Style::default().fg(palette.subtle_text)),
            Span::styled(
                due_preview.string.clone(),
                Style::default()
                    .fg(palette.subtle_text)
                    .add_modifier(Modifier::DIM),
            ),
        ]));
    }

    let panel = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.accent)),
    );

    frame.render_widget(panel, area);
}

fn task_list_footer(app: &App, symbols: Symbols, palette: ThemePalette) -> Line<'static> {
    let sort_prefix = if symbols.tasks == "#" {
        "sort"
    } else {
        symbols.sort
    };
    let filter_prefix = if symbols.tasks == "#" {
        "done"
    } else if app.hides_completed_tasks() {
        symbols.hidden
    } else {
        symbols.visible
    };
    let filter_label = if app.hides_completed_tasks() {
        "hidden"
    } else {
        "shown"
    };

    Line::from(vec![Span::styled(
        format!(
            " {} {}  {} {} ",
            sort_prefix,
            app.task_sort_order().short_label(),
            filter_prefix,
            filter_label,
        ),
        Style::default().fg(palette.subtle_text),
    )])
}

fn task_list_footer_hints(symbols: Symbols, focused: bool, palette: ThemePalette) -> Line<'static> {
    let hints = if focused {
        let _ = symbols;
        " o sort  f done "
    } else {
        ""
    };

    Line::from(vec![Span::styled(
        hints,
        Style::default().fg(palette.subtle_text),
    )])
    .right_aligned()
}

fn render_task_editor_popup(
    frame: &mut Frame<'_>,
    editor: &TaskEditorView,
    symbols: Symbols,
    palette: ThemePalette,
) {
    let area = centered_rect(frame.area(), 96, 17);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(Span::styled(
            editor.title,
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(editor_shortcuts_line(symbols, palette))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.accent));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Min(0),
        ])
        .split(inner);

    render_editor_field(
        frame,
        sections[0],
        "Title [F1]",
        &editor.title_value,
        editor.title_cursor,
        editor.focus.title,
        symbols,
        None,
        palette,
    );
    render_editor_field(
        frame,
        sections[1],
        "Description [F2]",
        &editor.description_value,
        editor.description_cursor,
        editor.focus.description,
        symbols,
        Some("Longer task notes will live here"),
        palette,
    );

    let due_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(sections[2]);
    render_editor_field(
        frame,
        due_row[0],
        "Due Date [F3]",
        &editor.due_date_value,
        editor.due_date_cursor,
        editor.focus.due_date,
        symbols,
        Some("YYYY-MM-DD"),
        palette,
    );
    render_editor_field(
        frame,
        due_row[1],
        "Due Time [F4]",
        &editor.due_time_value,
        editor.due_time_cursor,
        editor.focus.due_time,
        symbols,
        Some("HH:MM"),
        palette,
    );
    render_editor_field(
        frame,
        sections[3],
        "Recurrence [F5]",
        &editor.recurrence_value,
        editor.recurrence_cursor,
        editor.focus.recurrence,
        symbols,
        Some("every monday at 9am"),
        palette,
    );
    render_editor_due_preview_panel(frame, sections[4], editor, palette);

    if let Some(calendar) = editor.calendar {
        let calendar_area = anchored_dropdown_rect(frame.area(), due_row[0], 24, 10);
        render_editor_calendar(frame, calendar_area, calendar, palette);
    }
}

fn render_editor_field(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    value: &str,
    cursor: usize,
    focused: bool,
    symbols: Symbols,
    placeholder: Option<&str>,
    palette: ThemePalette,
) {
    let border_style = if focused {
        Style::default()
            .fg(palette.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(palette.subtle_text)
    };
    let text = if focused {
        vec![Line::from(editor_input_text(
            value,
            cursor,
            area.width.saturating_sub(4) as usize,
            symbols,
            palette,
        ))]
    } else if value.is_empty() {
        vec![Line::from(Span::styled(
            placeholder.unwrap_or(""),
            Style::default()
                .fg(palette.subtle_text)
                .add_modifier(Modifier::DIM),
        ))]
    } else if area.height > 3 {
        vec![Line::from(ellipsize_end(
            value,
            area.width.saturating_sub(4) as usize,
        ))]
    } else {
        vec![Line::from(input_window_text(
            value,
            cursor,
            area.width.saturating_sub(4) as usize,
        ))]
    };
    let widget = Paragraph::new(text).wrap(Wrap { trim: false }).block(
        Block::default()
            .title(Span::styled(label, border_style))
            .borders(Borders::ALL)
            .border_style(border_style),
    );
    frame.render_widget(widget, area);
}

fn editor_input_text(
    value: &str,
    cursor: usize,
    max_width: usize,
    symbols: Symbols,
    palette: ThemePalette,
) -> Span<'static> {
    let cursor_symbol = if symbols.tasks == "#" { "|" } else { "▏" };
    let safe_cursor = cursor.min(value.len());
    let mut with_cursor = value.to_string();
    with_cursor.insert_str(safe_cursor, cursor_symbol);
    Span::styled(
        input_window_text(
            with_cursor.as_str(),
            safe_cursor + cursor_symbol.len(),
            max_width,
        ),
        Style::default().fg(palette.text),
    )
}

fn render_editor_due_preview_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    editor: &TaskEditorView,
    palette: ThemePalette,
) {
    let lines = if let Some(due_preview) = &editor.due_preview {
        let mut lines = vec![Line::from(vec![
            Span::styled("Summary: ", Style::default().fg(palette.subtle_text)),
            Span::styled(
                due_preview.string.clone(),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
        ])];
        lines.push(Line::from(vec![
            Span::styled("Date: ", Style::default().fg(palette.subtle_text)),
            Span::styled(
                due_preview.date.format("%Y-%m-%d").to_string(),
                Style::default().fg(palette.text),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Time: ", Style::default().fg(palette.subtle_text)),
            Span::styled(
                due_preview
                    .datetime
                    .map(|datetime| datetime.format("%H:%M").to_string())
                    .unwrap_or_else(|| "-".to_string()),
                Style::default().fg(palette.text),
            ),
            Span::raw("   "),
            Span::styled("Recurring: ", Style::default().fg(palette.subtle_text)),
            Span::styled(
                if due_preview.is_recurring {
                    "yes"
                } else {
                    "no"
                },
                Style::default().fg(palette.text),
            ),
        ]));
        lines
    } else {
        vec![Line::from(vec![
            Span::styled("Summary: ", Style::default().fg(palette.subtle_text)),
            Span::styled("no due date", Style::default().fg(palette.text)),
        ])]
    };

    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(Span::styled(
                    "Due Preview",
                    Style::default().fg(palette.accent),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(palette.accent)),
        ),
        area,
    );
}

fn render_editor_calendar(
    frame: &mut Frame<'_>,
    area: Rect,
    calendar: CalendarPickerView,
    palette: ThemePalette,
) {
    let mut events = CalendarEventStore::default();
    let selected = time_date(calendar.selected_date);
    events.add(
        selected,
        Style::default()
            .fg(Color::Black)
            .bg(palette.accent)
            .add_modifier(Modifier::BOLD),
    );
    let widget = Monthly::new(
        TimeDate::from_calendar_date(
            calendar.display_date.year(),
            time_month(calendar.display_date.month()),
            1,
        )
        .expect("valid display date"),
        events,
    )
    .show_month_header(
        Style::default()
            .fg(palette.text)
            .add_modifier(Modifier::BOLD),
    )
    .show_weekdays_header(Style::default().fg(palette.subtle_text))
    .show_surrounding(Style::default().fg(palette.subtle_text))
    .default_style(Style::default().fg(palette.text))
    .block(
        Block::default()
            .title(Span::styled(
                "Calendar",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.accent)),
    );
    frame.render_widget(Clear, area);
    frame.render_widget(widget, area);
}

fn editor_shortcuts_line(symbols: Symbols, palette: ThemePalette) -> Line<'static> {
    if symbols.tasks == "#" {
        return Line::from(vec![Span::styled(
            "F6 calendar  F7 clear due  Enter save  Esc cancel",
            Style::default().fg(palette.subtle_text),
        )])
        .right_aligned();
    }

    Line::from(vec![
        Span::styled("F6", Style::default().fg(palette.subtle_text)),
        Span::raw(" 󰃭  "),
        Span::styled("F7", Style::default().fg(palette.subtle_text)),
        Span::raw(" due  "),
        Span::styled("󰄬", Style::default().fg(palette.subtle_text)),
        Span::raw(" ↵  "),
        Span::styled(
            symbols.voided.to_string(),
            Style::default().fg(palette.subtle_text),
        ),
        Span::raw(" esc"),
    ])
    .right_aligned()
}

fn anchored_dropdown_rect(area: Rect, anchor: Rect, width: u16, height: u16) -> Rect {
    let popup_width = width.min(area.width.saturating_sub(2)).max(1);
    let popup_height = height.min(area.height.saturating_sub(2)).max(1);

    let preferred_x = anchor.x;
    let max_x = area
        .x
        .saturating_add(area.width.saturating_sub(popup_width).saturating_sub(1));
    let x = preferred_x.clamp(
        area.x.saturating_add(1),
        max_x.max(area.x.saturating_add(1)),
    );

    let below_y = anchor.y.saturating_add(anchor.height.saturating_sub(1));
    let above_y = anchor.y.saturating_sub(popup_height.saturating_sub(1));
    let max_y = area
        .y
        .saturating_add(area.height.saturating_sub(popup_height).saturating_sub(1));

    let y = if below_y.saturating_add(popup_height) <= area.y.saturating_add(area.height) {
        below_y
    } else {
        above_y.clamp(
            area.y.saturating_add(1),
            max_y.max(area.y.saturating_add(1)),
        )
    };

    Rect::new(x, y, popup_width, popup_height)
}

fn render_delete_confirmation(
    frame: &mut Frame<'_>,
    confirmation: &DeleteConfirmationView,
    palette: ThemePalette,
) {
    let area = centered_rect(frame.area(), 64, 6);
    frame.render_widget(Clear, area);

    let lines = vec![
        Line::from("Remove this task from active lists?"),
        Line::from(""),
        Line::from(Span::styled(
            format!("\"{}\"", confirmation.task_title),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("History links will be preserved."),
        Line::from("Enter/Y confirm  Esc/N cancel"),
    ];
    let popup = Paragraph::new(lines).block(
        Block::default()
            .title(Span::styled(
                "Remove Task",
                Style::default()
                    .fg(palette.error)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.error)),
    );

    frame.render_widget(popup, area);
}

fn render_task_search_popup(frame: &mut Frame<'_>, search: &TaskSearchView, palette: ThemePalette) {
    let area = centered_rect(frame.area(), 72, 8);
    frame.render_widget(Clear, area);

    let visible_width = area.width.saturating_sub(4) as usize;
    let mut lines = vec![Line::from(input_window_text(
        &search.query,
        search.cursor,
        visible_width,
    ))];
    lines.push(Line::from(""));

    if search.results.is_empty() {
        lines.push(Line::from("No matching tasks."));
    } else {
        let visible_count = 4usize;
        let start = search
            .selected_index
            .saturating_sub(visible_count.saturating_sub(1));
        let end = (start + visible_count).min(search.results.len());
        for (offset, result) in search.results[start..end].iter().enumerate() {
            let selected = start + offset == search.selected_index;
            lines.push(selectable_line(
                result.title.as_str(),
                selected,
                area.width.saturating_sub(4),
                palette,
            ));
        }
    }

    let popup = Paragraph::new(lines).block(
        Block::default()
            .title(Span::styled(
                search.title,
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.accent)),
    );

    frame.render_widget(popup, area);
}

fn render_task_sort_popup(
    frame: &mut Frame<'_>,
    popup: &TaskSortPopupView,
    anchor: Rect,
    symbols: Symbols,
    palette: ThemePalette,
) {
    let width = 24;
    let height = popup.options.len() as u16 + 2;
    let area = anchored_dropdown_rect(frame.area(), anchor, width, height);
    frame.render_widget(Clear, area);

    let lines = popup
        .options
        .iter()
        .enumerate()
        .map(|(index, option)| {
            let selected = popup.selected_index == index;
            let marker = if option.is_active {
                if symbols.tasks == "#" { "* " } else { "󰄵 " }
            } else {
                "  "
            };
            selectable_line(
                &format!("{marker}{}", option.label),
                selected,
                area.width.saturating_sub(2),
                palette,
            )
        })
        .collect::<Vec<_>>();

    let widget = Paragraph::new(lines).block(
        Block::default()
            .title(Span::styled(
                popup.title,
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.accent)),
    );

    frame.render_widget(widget, area);
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let popup_width = width.min(area.width.saturating_sub(2)).max(1);
    let popup_height = height.min(area.height.saturating_sub(2)).max(1);
    let x = area.x + area.width.saturating_sub(popup_width) / 2;
    let y = area.y + area.height.saturating_sub(popup_height) / 2;
    Rect::new(x, y, popup_width, popup_height)
}

#[derive(Debug, Clone)]
struct HistoryRow {
    started_at: chrono::DateTime<chrono::Local>,
    focus_outcome: SessionOutcome,
    focus_seconds: u32,
    break_seconds: u32,
    task_title: Option<String>,
}

fn history_rows(entries: &[SessionEntry], tasks: &[Task]) -> Vec<HistoryRow> {
    let mut rows = Vec::new();
    let mut previous_was_focus = false;

    for entry in entries.iter().rev() {
        match entry.kind {
            SessionKind::Focus => {
                let task_title = entry.task_id.and_then(|task_id| {
                    tasks
                        .iter()
                        .find(|task| task.id == task_id)
                        .map(|task| task.title.clone())
                });
                rows.push(HistoryRow {
                    started_at: entry.started_at,
                    focus_outcome: entry.outcome.clone(),
                    focus_seconds: entry.duration_seconds,
                    break_seconds: 0,
                    task_title,
                });
                previous_was_focus = true;
            }
            SessionKind::ShortBreak | SessionKind::LongBreak => {
                if previous_was_focus {
                    if let Some(last) = rows.last_mut() {
                        last.break_seconds = entry.duration_seconds;
                    }
                }
                previous_was_focus = false;
            }
        }
    }

    rows.reverse();
    rows
}

fn format_history_row(
    row: &HistoryRow,
    symbols: Symbols,
    palette: ThemePalette,
    width: u16,
    selected: bool,
) -> Line<'static> {
    let (symbol, accent) = match row.focus_outcome {
        SessionOutcome::Completed => (symbols.done, palette.success),
        SessionOutcome::Voided => (symbols.voided, palette.error),
    };
    let timing = format!(
        "{}/{}",
        format_compact_duration(row.focus_seconds),
        format_compact_duration(row.break_seconds)
    );
    let prefix = format!("{}  {}  {}", row.started_at.format("%H:%M"), symbol, timing);
    let task_text = row.task_title.as_deref().unwrap_or("-");
    let separator = "  ";
    let prefix_width = UnicodeWidthStr::width(prefix.as_str());
    let separator_width = UnicodeWidthStr::width(separator);
    let remaining_width = (width as usize).saturating_sub(prefix_width + separator_width);
    let visible_task_text = ellipsize_end(task_text, remaining_width);
    let visible_text = format!("{prefix}{separator}{visible_task_text}");

    let style = if selected {
        Style::default()
            .fg(palette.text)
            .bg(palette.border)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(accent)
    };

    let mut spans = vec![Span::styled(visible_text, style)];
    if selected {
        let current_width = Line::from(spans.clone()).width();
        let padding = (width as usize).saturating_sub(current_width);
        if padding > 0 {
            spans.push(Span::styled(" ".repeat(padding), style));
        }
    }

    Line::from(spans)
}

fn render_weekly_history_lines(
    summaries: &[DayHistorySummary],
    palette: ThemePalette,
) -> Vec<Line<'static>> {
    if summaries.is_empty() {
        return vec![Line::from("No history recorded in the last 7 days.")];
    }

    let max_total = summaries
        .iter()
        .map(|summary| summary.completed_sessions + summary.voided_sessions)
        .max()
        .unwrap_or(1)
        .max(1);

    summaries
        .iter()
        .map(|summary| {
            let total = summary.completed_sessions + summary.voided_sessions;
            let completed_width = (summary.completed_sessions * 10).div_ceil(max_total);
            let voided_width = (summary.voided_sessions * 10).div_ceil(max_total);
            let bar = format!(
                "{}{}",
                "█".repeat(completed_width),
                "░".repeat(voided_width)
            );
            Line::from(vec![
                Span::styled(
                    summary.day.format("%a %d").to_string(),
                    Style::default().fg(palette.subtle_text),
                ),
                Span::raw("  "),
                Span::styled("C", Style::default().fg(palette.success)),
                Span::styled(
                    format!("{:>2}", summary.completed_sessions),
                    Style::default().fg(palette.text),
                ),
                Span::raw(" "),
                Span::styled("V", Style::default().fg(palette.error)),
                Span::styled(
                    format!("{:>2}", summary.voided_sessions),
                    Style::default().fg(palette.text),
                ),
                Span::raw("  "),
                Span::styled(bar, Style::default().fg(palette.accent)),
                Span::raw("  "),
                Span::styled(
                    format_duration_seconds(summary.focus_seconds),
                    Style::default().fg(palette.text),
                ),
                Span::raw(" / "),
                Span::styled(
                    format_duration_seconds(summary.break_seconds),
                    Style::default().fg(palette.subtle_text),
                ),
                Span::raw(if total == 0 { "  -" } else { "" }),
            ])
        })
        .collect()
}

fn history_title(active_tab: HistoryPanelTab, palette: ThemePalette) -> Line<'static> {
    let today_style = if active_tab == HistoryPanelTab::Today {
        Style::default().fg(palette.accent)
    } else {
        Style::default().fg(palette.subtle_text)
    };
    let weekly_style = if active_tab == HistoryPanelTab::Last7Days {
        Style::default().fg(palette.accent)
    } else {
        Style::default().fg(palette.subtle_text)
    };

    Line::from(vec![
        Span::raw("[2] "),
        Span::styled("Today", today_style),
        Span::raw(" - "),
        Span::styled("Last 7 Days", weekly_style),
    ])
}

fn selectable_line(
    label: &str,
    selected: bool,
    width: u16,
    palette: ThemePalette,
) -> Line<'static> {
    let style = if selected {
        Style::default()
            .fg(palette.text)
            .bg(palette.border)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(palette.text)
    };

    let text = ellipsize_end(&format!("  {label}"), width as usize);
    let mut spans = vec![Span::styled(text, style)];
    if selected {
        let current_width = Line::from(spans.clone()).width();
        let padding = (width as usize).saturating_sub(current_width);
        if padding > 0 {
            spans.push(Span::styled(" ".repeat(padding), style));
        }
    }

    Line::from(spans)
}

fn task_view_symbol(view: TaskView, symbols: Symbols) -> &'static str {
    match view {
        TaskView::All => symbols.tasks,
        TaskView::Inbox => symbols.inbox,
        TaskView::Today => symbols.today,
        TaskView::Soon => symbols.soon,
    }
}

fn navigation_title(symbols: Symbols, palette: ThemePalette) -> Line<'static> {
    Line::from(vec![
        Span::raw("[3] "),
        Span::styled(
            format!("{} Navigation", symbols.navigation),
            Style::default().fg(palette.accent),
        ),
        Span::raw(" - "),
        Span::styled("Filters & Tags", Style::default().fg(palette.subtle_text)),
        Span::raw(" - "),
        Span::styled("Projects", Style::default().fg(palette.subtle_text)),
    ])
}

fn right_panel_title(
    active_tab: RightPanelTab,
    symbols: Symbols,
    palette: ThemePalette,
) -> Line<'static> {
    let tasks_style = if active_tab == RightPanelTab::Tasks {
        Style::default().fg(palette.accent)
    } else {
        Style::default().fg(palette.subtle_text)
    };
    let stats_style = if active_tab == RightPanelTab::Statistics {
        Style::default().fg(palette.accent)
    } else {
        Style::default().fg(palette.subtle_text)
    };

    Line::from(vec![
        Span::raw("[5] "),
        Span::styled(format!("{} Tasks", symbols.tasks), tasks_style),
        Span::raw(" - "),
        Span::styled(format!("{} Stats", symbols.stats), stats_style),
    ])
}

fn panel_block(title: Line<'static>, focused: bool, palette: ThemePalette) -> Block<'static> {
    let border_style = if focused {
        Style::default()
            .fg(palette.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(palette.border)
    };

    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style)
}

fn format_duration(duration: chrono::Duration) -> String {
    let total_seconds = duration.num_seconds().max(0);
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}

fn format_duration_seconds(total_seconds: u32) -> String {
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn format_compact_duration(total_seconds: u32) -> String {
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    if total_seconds == 0 {
        "00m".to_string()
    } else if minutes > 0 && seconds == 0 {
        format!("{minutes:02}m")
    } else if minutes == 0 {
        format!("{seconds:02}s")
    } else {
        format!("{minutes:02}m{seconds:02}s")
    }
}

fn assigned_task_line(
    app: &App,
    _symbols: Symbols,
    palette: ThemePalette,
    width: u16,
) -> Line<'static> {
    match app.assigned_task() {
        Some(task) => Line::from(vec![
            Span::styled("Task: ", Style::default().fg(palette.subtle_text)),
            Span::styled(
                ellipsize_end(&task.title, width.saturating_sub(6) as usize),
                Style::default().fg(palette.text),
            ),
        ]),
        None => Line::from(vec![Span::styled(
            "Task: none",
            Style::default().fg(palette.subtle_text),
        )]),
    }
}

fn timer_color(phase: TimerPhase, palette: ThemePalette) -> Color {
    match phase {
        TimerPhase::Focus => palette.timer_work,
        TimerPhase::ShortBreak => palette.timer_short_break,
        TimerPhase::LongBreak => palette.timer_long_break,
    }
}

fn cycle_line(
    entries: &[CycleEntryState],
    symbols: Symbols,
    palette: ThemePalette,
) -> Line<'static> {
    let mut spans = vec![Span::styled(
        "Cycle ",
        Style::default()
            .fg(palette.text)
            .add_modifier(Modifier::BOLD),
    )];

    for (index, entry) in entries.iter().enumerate() {
        let (symbol, color) = match entry {
            CycleEntryState::NotStarted => (symbols.todo, palette.subtle_text),
            CycleEntryState::Running => (symbols.in_progress, palette.timer_work),
            CycleEntryState::Break => (symbols.breaking, palette.timer_short_break),
            CycleEntryState::Completed => (symbols.done, palette.success),
            CycleEntryState::Voided => (symbols.voided, palette.error),
        };
        spans.push(Span::styled(symbol.to_string(), Style::default().fg(color)));
        if index + 1 < entries.len() {
            spans.push(Span::raw(" "));
        }
    }

    Line::from(spans)
}

fn progress_bar(timer: &crate::app::TimerView, symbols: Symbols, width: u16) -> String {
    let width = width.saturating_sub(2).max(8) as usize;
    let filled = (timer.progress.clamp(0.0, 1.0) * width as f64).round() as usize;
    format!(
        "{}{}",
        symbols.bar_full.repeat(filled),
        symbols.bar_empty.repeat(width.saturating_sub(filled))
    )
}

fn ellipsize_end(text: &str, max_width: usize) -> String {
    const ELLIPSIS: &str = "…";

    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    if max_width == 1 {
        return ELLIPSIS.to_string();
    }

    let mut width = 0;
    let mut output = String::new();
    for character in text.chars() {
        let char_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if width + char_width > max_width - 1 {
            break;
        }
        width += char_width;
        output.push(character);
    }
    output.push_str(ELLIPSIS);
    output
}

fn tail_visible_text(text: &str, max_width: usize) -> String {
    const ELLIPSIS: &str = "…";

    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    if max_width == 1 {
        return ELLIPSIS.to_string();
    }

    let mut width = 1;
    let mut chars = Vec::new();
    for character in text.chars().rev() {
        let char_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if width + char_width > max_width {
            break;
        }
        width += char_width;
        chars.push(character);
    }
    chars.reverse();

    let mut output = String::from(ELLIPSIS);
    for character in chars {
        output.push(character);
    }
    output
}

fn input_window_text(text: &str, cursor: usize, max_width: usize) -> String {
    const ELLIPSIS: &str = "…";

    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }

    let safe_cursor = cursor.min(text.len());
    let before = &text[..safe_cursor];
    let after = &text[safe_cursor..];

    if UnicodeWidthStr::width(before) <= max_width / 2 {
        return ellipsize_end(text, max_width);
    }
    if UnicodeWidthStr::width(after) <= max_width / 2 {
        return tail_visible_text(text, max_width);
    }

    let left_budget = max_width.saturating_sub(2) / 2;
    let right_budget = max_width.saturating_sub(2) - left_budget;

    let mut left_width = 0;
    let mut left_chars = Vec::new();
    for character in before.chars().rev() {
        let char_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if left_width + char_width > left_budget {
            break;
        }
        left_width += char_width;
        left_chars.push(character);
    }
    left_chars.reverse();

    let mut right_width = 0;
    let mut right_chars = Vec::new();
    for character in after.chars() {
        let char_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if right_width + char_width > right_budget {
            break;
        }
        right_width += char_width;
        right_chars.push(character);
    }

    let mut output = String::from(ELLIPSIS);
    for character in left_chars {
        output.push(character);
    }
    for character in right_chars {
        output.push(character);
    }
    output.push_str(ELLIPSIS);
    output
}

fn progress_meta_line(
    timer: &crate::app::TimerView,
    width: u16,
    palette: ThemePalette,
) -> Line<'static> {
    let percent = format!(
        "{}%",
        (timer.progress.clamp(0.0, 1.0) * 100.0).round() as u32
    );
    let remaining = format_duration(timer.remaining);
    let available = width.saturating_sub(2) as usize;
    let spaces = available.saturating_sub(percent.len() + remaining.len());
    Line::from(vec![
        Span::styled(percent, Style::default().fg(palette.subtle_text)),
        Span::raw(" ".repeat(spaces)),
        Span::styled(remaining, Style::default().fg(palette.text)),
    ])
}

#[derive(Debug, Clone, Copy)]
struct Symbols {
    timer: &'static str,
    navigation: &'static str,
    favorite: &'static str,
    tasks: &'static str,
    inbox: &'static str,
    today: &'static str,
    soon: &'static str,
    details: &'static str,
    stats: &'static str,
    sort: &'static str,
    hidden: &'static str,
    visible: &'static str,
    recurring: &'static str,
    todo: &'static str,
    in_progress: &'static str,
    breaking: &'static str,
    done: &'static str,
    voided: &'static str,
    bar_full: &'static str,
    bar_empty: &'static str,
}

impl Symbols {
    fn new(mode: GlyphMode) -> Self {
        match mode {
            GlyphMode::Ascii => Self {
                timer: "*",
                navigation: ">",
                favorite: "*",
                tasks: "#",
                inbox: "I",
                today: "T",
                soon: "S",
                details: ">",
                stats: "%",
                sort: "~",
                hidden: "x",
                visible: "o",
                recurring: "~",
                todo: ".",
                in_progress: ">",
                breaking: "~",
                done: "x",
                voided: "!",
                bar_full: "=",
                bar_empty: "-",
            },
            GlyphMode::NerdFonts => Self {
                timer: "󰔛",
                navigation: "󰆍",
                favorite: "󰓎",
                tasks: "󰄱",
                inbox: "󰏆",
                today: "󰃰",
                soon: "󰸘",
                details: "󰋼",
                stats: "󰕾",
                sort: "󰒺",
                hidden: "󰈉",
                visible: "󰈈",
                recurring: "󰑖",
                todo: "󰄱",
                in_progress: "󰧞",
                breaking: "󰒲",
                done: "󰄵",
                voided: "󰅖",
                bar_full: "█",
                bar_empty: "░",
            },
        }
    }
}

fn task_row_style(
    task: &Task,
    palette: ThemePalette,
    selected: bool,
    now: chrono::DateTime<Local>,
) -> Style {
    let base = if selected {
        Style::default()
            .bg(palette.border)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    match task.status {
        TaskStatus::Done => base.fg(palette.subtle_text).add_modifier(Modifier::DIM),
        TaskStatus::Todo if task_is_overdue(task, now) => base.fg(palette.error),
        TaskStatus::Todo => base.fg(palette.text),
    }
}

fn task_due_style(
    task: &Task,
    palette: ThemePalette,
    selected: bool,
    now: chrono::DateTime<Local>,
) -> Style {
    let base = if selected {
        Style::default()
            .bg(palette.border)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    if task_is_overdue(task, now) {
        base.fg(palette.error)
    } else if task.status == TaskStatus::Done {
        base.fg(palette.subtle_text).add_modifier(Modifier::DIM)
    } else {
        base.fg(palette.accent)
    }
}

fn task_recurring_style(
    task: &Task,
    palette: ThemePalette,
    selected: bool,
    now: chrono::DateTime<Local>,
) -> Style {
    let base = if selected {
        Style::default()
            .bg(palette.border)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    if task_is_overdue(task, now) {
        base.fg(palette.error)
    } else if task.status == TaskStatus::Done {
        base.fg(palette.subtle_text).add_modifier(Modifier::DIM)
    } else {
        base.fg(palette.timer_short_break)
    }
}

fn task_is_overdue(task: &Task, now: chrono::DateTime<Local>) -> bool {
    let Some(due) = &task.due else {
        return false;
    };
    if task.status == TaskStatus::Done {
        return false;
    }

    if let Some(datetime) = due.datetime {
        datetime < now.naive_local()
    } else {
        due.date < now.date_naive()
    }
}

fn format_due_label(due: &crate::domain::TaskDue, today: NaiveDate) -> String {
    let day_label = match (due.date - today).num_days() {
        0 => "today".to_string(),
        1 => "tomorrow".to_string(),
        -1 => "yesterday".to_string(),
        2..=6 => format!("in {} days", (due.date - today).num_days()),
        -6..=-2 => format!("{} days ago", (today - due.date).num_days()),
        7..=13 => "next week".to_string(),
        -13..=-7 => "last week".to_string(),
        _ => due.date.format("%Y-%m-%d").to_string(),
    };

    if let Some(datetime) = due.datetime {
        format!("{day_label} {}", datetime.format("%H:%M"))
    } else {
        day_label
    }
}

fn format_recurring_rule(value: &str) -> String {
    value
        .split_whitespace()
        .map(capitalize_word)
        .collect::<Vec<_>>()
        .join(" ")
}

fn capitalize_word(word: &str) -> String {
    let mut characters = word.chars();
    let Some(first) = characters.next() else {
        return String::new();
    };

    let mut output = String::new();
    output.extend(first.to_uppercase());
    output.push_str(characters.as_str());
    output
}

fn time_date(date: NaiveDate) -> TimeDate {
    TimeDate::from_calendar_date(date.year(), time_month(date.month()), date.day() as u8)
        .expect("valid date")
}

fn time_month(month: u32) -> TimeMonth {
    match month {
        1 => TimeMonth::January,
        2 => TimeMonth::February,
        3 => TimeMonth::March,
        4 => TimeMonth::April,
        5 => TimeMonth::May,
        6 => TimeMonth::June,
        7 => TimeMonth::July,
        8 => TimeMonth::August,
        9 => TimeMonth::September,
        10 => TimeMonth::October,
        11 => TimeMonth::November,
        12 => TimeMonth::December,
        _ => TimeMonth::January,
    }
}
