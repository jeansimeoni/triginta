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
        App, CalendarPickerView, CycleEntryState, DeleteConfirmationView, FormPreviewPanelView,
        HistoryPanelTab, PanelFocus, PreviewLineView, ProjectDeleteConfirmationView,
        ProjectEditorView, ProjectSortPopupView, ProjectTreeRowView, RightPanelTab, ScreenData,
        ShortcutSection, ShortcutTip, SidebarTab, TagDeleteConfirmationView, TagEditorView,
        TagListRowView, TagSortPopupView, TaskEditorView, TaskInputView, TaskSearchView,
        TaskSortPopupView, TaskView, TimerPhase,
    },
    config::GlyphMode,
    domain::{
        DayHistorySummary, SessionEntry, SessionKind, SessionOutcome, TagColor, Task, TaskPriority,
        TaskStatus,
    },
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
        Line::from(format!("[1] {} Pomodoro", symbols.timer)),
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
    let (summary, lines, right_indicator): (
        Line<'static>,
        Vec<Line<'static>>,
        Option<(usize, usize)>,
    ) = match app.active_history_panel_tab() {
        HistoryPanelTab::Today => {
            let rows = history_rows(data.history_entries.as_slice(), data.tasks.as_slice());
            let selected = today_selected.min(rows.len().saturating_sub(1));
            let visible_height = area.height.saturating_sub(2) as usize;
            let start = selected.saturating_sub(visible_height.saturating_sub(1));
            let summary = Line::from(format!(
                "{} {}  |  {} {}  |  {} {}",
                symbols.timer,
                format_duration_seconds(data.today_stats.total_work_seconds),
                symbols.breaking,
                format_duration_seconds(data.today_stats.total_break_seconds),
                symbols.stats,
                data.today_stats.total_sessions,
            ))
            .right_aligned();
            let lines = if rows.is_empty() {
                vec![Line::from("No pomodoros recorded today.")]
            } else {
                let show_selection = app.focused_panel() == PanelFocus::History;
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
                Some((rows.len(), start))
            } else {
                None
            };
            (summary, lines, indicator)
        }
        HistoryPanelTab::Last7Days => (
            Line::from(format!(
                "{} {}  |  {} {}  |  {} {}",
                symbols.timer,
                format_duration_seconds(data.weekly_stats.total_work_seconds),
                symbols.breaking,
                format_duration_seconds(data.weekly_stats.total_break_seconds),
                symbols.stats,
                data.weekly_stats.total_sessions,
            ))
            .right_aligned(),
            render_weekly_history_lines(
                data.weekly_summaries.as_slice(),
                area.width.saturating_sub(4),
                palette,
            ),
            None,
        ),
    };
    let block = panel_block(
        history_title(app.active_history_panel_tab(), symbols, palette),
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

    if let Some((content_length, scroll_offset)) = right_indicator {
        let viewport = inner.height as usize;
        let position = scrollbar_position_from_offset(scroll_offset, content_length, viewport);
        let mut scrollbar_state = ScrollbarState::default()
            .content_length(content_length)
            .viewport_content_length(viewport)
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
    let (mut lines, selected_index) = match app.active_sidebar_tab() {
        SidebarTab::Navigation => (
            app.navigation_task_views()
                .into_iter()
                .map(|view| {
                    let selected = app.active_task_view() == view;
                    selectable_count_line(
                        &format!("{} {}", task_view_symbol(view, symbols), view.label()),
                        app.task_count_for_view(view),
                        selected,
                        content_width,
                        palette,
                    )
                })
                .collect::<Vec<_>>(),
            app.navigation_task_views()
                .iter()
                .position(|view| app.active_task_view() == *view),
        ),
        SidebarTab::FiltersTags => {
            let rows = app.tags_rows();
            let selected_index = rows.iter().position(|row| row.is_selected);
            if rows.is_empty() {
                (vec![Line::from("No matching tags.")], selected_index)
            } else if rows.len() == 1 && !app.has_user_tags() {
                (vec![Line::from("No tags yet.")], selected_index)
            } else {
                (
                    rows.into_iter()
                        .map(|row| tag_list_line(row, symbols, content_width, palette))
                        .collect::<Vec<_>>(),
                    selected_index,
                )
            }
        }
        SidebarTab::Projects => {
            let rows = app.project_tree_rows();
            let selected_index = rows.iter().position(|row| row.is_selected);
            if rows.is_empty() {
                (vec![Line::from("No matching projects.")], selected_index)
            } else if rows.len() == 1 && !app.has_user_projects() {
                (vec![Line::from("No projects yet.")], selected_index)
            } else {
                (
                    rows.into_iter()
                        .map(|row| project_tree_line(row, symbols, content_width, palette))
                        .collect::<Vec<_>>(),
                    selected_index,
                )
            }
        }
    };
    if lines.is_empty() {
        lines = vec![Line::from("No matching results.")];
    }

    let footer = match app.active_sidebar_tab() {
        SidebarTab::FiltersTags => tags_sort_footer(app, symbols, palette),
        SidebarTab::Projects => projects_sort_footer(app, symbols, palette),
        SidebarTab::Navigation => Line::from(""),
    };
    let footer_hints = match app.active_sidebar_tab() {
        SidebarTab::Navigation => navigation_footer_hint(symbols, palette),
        SidebarTab::FiltersTags => {
            tags_footer_hints(symbols, focused_panel == PanelFocus::Navigation, palette)
        }
        SidebarTab::Projects => {
            projects_footer_hints(symbols, focused_panel == PanelFocus::Navigation, palette)
        }
    };

    let block = panel_block(
        navigation_title(app.active_sidebar_tab(), symbols, palette),
        focused_panel == PanelFocus::Navigation,
        palette,
    )
    .title_bottom(footer)
    .title_bottom(footer_hints);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let viewport_lines = inner.height as usize;
    let scroll = panel_scroll_offset(lines.len(), viewport_lines, selected_index);
    let visible_lines = lines
        .iter()
        .skip(scroll)
        .take(viewport_lines)
        .cloned()
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(visible_lines).wrap(Wrap { trim: false }),
        inner,
    );

    if lines.len() > viewport_lines {
        let selected_position = scrollbar_position_from_offset(scroll, lines.len(), viewport_lines);
        let mut scrollbar_state = ScrollbarState::default()
            .content_length(lines.len())
            .viewport_content_length(viewport_lines)
            .position(selected_position);
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

fn panel_scroll_offset(
    total_lines: usize,
    viewport_lines: usize,
    selected_index: Option<usize>,
) -> usize {
    if total_lines <= viewport_lines || viewport_lines == 0 {
        return 0;
    }

    let max_scroll = total_lines.saturating_sub(viewport_lines);
    let selected = selected_index
        .unwrap_or(0)
        .min(total_lines.saturating_sub(1));
    selected.saturating_sub(viewport_lines / 2).min(max_scroll)
}

fn scrollbar_position_from_offset(
    scroll_offset: usize,
    total_lines: usize,
    viewport_lines: usize,
) -> usize {
    if total_lines == 0 {
        return 0;
    }
    if total_lines <= viewport_lines || viewport_lines == 0 {
        return scroll_offset.min(total_lines.saturating_sub(1));
    }

    let max_start = total_lines.saturating_sub(viewport_lines);
    let clamped_start = scroll_offset.min(max_start);
    clamped_start.saturating_mul(total_lines.saturating_sub(1)) / max_start.max(1)
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
            Line::from(format!("[6] {} Favorites", symbols.favorite)),
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
    let search_query = app.task_list_search_query().unwrap_or("");
    let footer = task_list_footer(app, symbols, palette);
    let footer_hints = task_list_footer_hints(
        app,
        symbols,
        focused_panel == PanelFocus::RightPane,
        palette,
    );
    let block = panel_block(
        right_panel_title(RightPanelTab::Tasks, symbols, palette),
        focused_panel == PanelFocus::RightPane,
        palette,
    )
    .title_bottom(footer)
    .title_bottom(footer_hints);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if visible_tasks.is_empty() {
        let lines = if search_query.is_empty() {
            match app.active_task_view() {
                TaskView::Today => vec![
                    Line::from("No tasks in Today yet."),
                    Line::from("Tasks due today will appear here."),
                ],
                TaskView::Soon => vec![
                    Line::from("No tasks in Soon yet."),
                    Line::from("Upcoming tasks will appear here."),
                ],
                TaskView::All | TaskView::Inbox => vec![
                    Line::from("No tasks yet."),
                    Line::from("Press c to create your first task."),
                ],
            }
        } else {
            vec![Line::from("No matching tasks.")]
        };
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        return;
    }

    let viewport_task_rows = (inner.height as usize / 2).max(1);
    let has_scrollbar = visible_tasks.len() > viewport_task_rows;
    let content_width = inner
        .width
        .saturating_sub(if has_scrollbar { 1 } else { 0 });
    let selected_index = app
        .selected_task()
        .and_then(|selected| visible_tasks.iter().position(|task| task.id == selected.id));
    let task_scroll = panel_scroll_offset(visible_tasks.len(), viewport_task_rows, selected_index);

    let mut lines = Vec::with_capacity(viewport_task_rows.saturating_mul(2));
    let show_selection = focused_panel == PanelFocus::RightPane;
    for task in visible_tasks
        .iter()
        .skip(task_scroll)
        .take(viewport_task_rows)
    {
        let selected =
            show_selection && app.selected_task().map(|selected| selected.id) == Some(task.id);
        lines.push(task_summary_line(
            task,
            symbols,
            palette,
            selected,
            content_width,
        ));
        lines.push(task_project_line(
            app.screen_data(),
            task,
            symbols,
            palette,
            selected,
            content_width,
        ));
    }

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);

    if has_scrollbar {
        let position =
            scrollbar_position_from_offset(task_scroll, visible_tasks.len(), viewport_task_rows);
        let mut scrollbar_state = ScrollbarState::default()
            .content_length(visible_tasks.len())
            .viewport_content_length(viewport_task_rows)
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
        if let Some(indicator) = task_priority_indicator(task.priority, symbols) {
            lines.push(Line::from(vec![
                Span::raw("Priority: "),
                Span::styled(
                    indicator,
                    Style::default().fg(priority_color(task.priority, palette)),
                ),
            ]));
        }
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
        if let Some(project_name) = project_name_for_task(app.screen_data(), task) {
            lines.push(Line::from(format!("Project: {project_name}")));
        }
        let tags = task_tags_for_task(app.screen_data(), task.id);
        if !tags.is_empty() {
            lines.push(Line::from(format!(
                "Tags: {}",
                tags.into_iter()
                    .map(|(tag, _)| format!("@{tag}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            )));
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
        symbols.bar_full.repeat(filled),
        symbols
            .bar_empty
            .repeat(completed_width.saturating_sub(filled))
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
    if let Some(search) = app.focused_panel_search_status() {
        if search.is_editing {
            let query_width = center_width.saturating_sub(1) as usize;
            let query_window = input_window_view(&search.query, search.cursor, query_width.max(1));
            let text = format!("/{}", query_window.text);
            frame.render_widget(
                Paragraph::new(Line::from(ellipsize_end(&text, center_width as usize)))
                    .style(Style::default().fg(palette.subtle_text)),
                center_area,
            );
            let cursor_col = 1usize.saturating_add(query_window.cursor_col);
            let x = center_area
                .x
                .saturating_add((cursor_col as u16).min(center_area.width.saturating_sub(1)));
            frame.set_cursor_position((x, center_area.y));
            return;
        }
        let locked_text = format!("SEARCH /{}  Esc clear", search.query);
        frame.render_widget(
            Paragraph::new(
                Line::from(ellipsize_end(&locked_text, center_width as usize)).centered(),
            )
            .style(Style::default().fg(palette.subtle_text)),
            center_area,
        );
        return;
    }

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

fn task_project_line(
    data: &ScreenData,
    task: &Task,
    symbols: Symbols,
    palette: ThemePalette,
    selected: bool,
    width: u16,
) -> Line<'static> {
    let (project_name, project_color) = project_meta_for_task(data, task)
        .map(|(name, color)| (name, palette.project_color(color)))
        .unwrap_or(("Inbox", palette.subtle_text));
    let priority_indicator = task_priority_indicator(task.priority, symbols);
    let priority_meta_width = priority_indicator
        .as_ref()
        .map(|value| value.width())
        .unwrap_or(0);
    let tags =
        format_task_tags_for_row(task_tags_for_task(data, task.id).as_slice(), width, symbols);
    let tags_width = task_tag_segments_width(tags.as_slice(), symbols);
    let status_marker = task_status_symbol(task.status, symbols);
    let leading_padding = 2usize
        .saturating_add(status_marker.width())
        .saturating_add(1);
    let has_right_meta = priority_indicator.is_some() || !tags.is_empty();
    let right_meta_width = priority_meta_width
        .saturating_add(tags_width)
        .saturating_add(if priority_indicator.is_some() && !tags.is_empty() {
            1
        } else {
            0
        });
    let base_style = if selected {
        Style::default()
            .fg(palette.text)
            .bg(palette.border)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let glyph_style = base_style.patch(if selected {
        Style::default().fg(palette.text)
    } else {
        Style::default().fg(project_color)
    });
    let name_style = base_style.patch(if selected {
        Style::default().fg(palette.text)
    } else {
        Style::default().fg(project_color)
    });
    let mut spans = vec![
        Span::styled(" ".repeat(leading_padding), base_style),
        Span::styled(symbols.project, glyph_style),
        Span::styled(
            " ",
            base_style.patch(Style::default().fg(palette.subtle_text)),
        ),
        Span::styled(
            ellipsize_end(
                project_name,
                width
                    .saturating_sub(leading_padding as u16)
                    .saturating_sub(symbols.project.width() as u16)
                    .saturating_sub(1)
                    .saturating_sub(right_meta_width as u16)
                    .saturating_sub(if has_right_meta { 2 } else { 0 }) as usize,
            ),
            name_style,
        ),
    ];
    if has_right_meta {
        let current_width = Line::from(spans.clone()).width();
        let min_gap = 2usize;
        let available_for_gap = (width as usize)
            .saturating_sub(current_width)
            .saturating_sub(right_meta_width);
        let gap = available_for_gap.max(min_gap);
        spans.push(Span::styled(" ".repeat(gap), base_style));
    }

    if let Some(priority_indicator) = priority_indicator.as_ref() {
        let priority_style = if selected {
            base_style.patch(
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Style::default()
                .fg(priority_color(task.priority, palette))
                .add_modifier(Modifier::BOLD)
        };
        spans.push(Span::styled(priority_indicator.clone(), priority_style));
        if !tags.is_empty() {
            spans.push(Span::styled(" ".to_string(), base_style));
        }
    }
    if !tags.is_empty() {
        spans.extend(task_tag_segments_spans(
            tags.as_slice(),
            base_style,
            selected,
            palette,
            symbols,
        ));
    }
    let current_width = Line::from(spans.clone()).width();
    let padding = (width as usize).saturating_sub(current_width);
    if padding > 0 {
        spans.push(Span::styled(" ".repeat(padding), base_style));
    }
    Line::from(spans)
}

fn project_name_for_task<'a>(data: &'a ScreenData, task: &Task) -> Option<&'a str> {
    data.projects
        .iter()
        .find(|project| project.id == task.project_id)
        .map(|project| project.name.as_str())
}

fn project_meta_for_task<'a>(
    data: &'a ScreenData,
    task: &Task,
) -> Option<(&'a str, crate::domain::ProjectColor)> {
    data.projects
        .iter()
        .find(|project| project.id == task.project_id)
        .map(|project| (project.name.as_str(), project.color))
}

fn task_tags_for_task<'a>(
    data: &'a ScreenData,
    task_id: crate::domain::TaskId,
) -> Vec<(&'a str, TagColor)> {
    let mut tags = data
        .task_tag_links
        .iter()
        .filter_map(|(linked_task_id, tag_id)| {
            if *linked_task_id != task_id {
                return None;
            }
            data.tags
                .iter()
                .find(|tag| tag.id == *tag_id && tag.deleted_at.is_none())
                .map(|tag| (tag.name.as_str(), tag.color))
        })
        .collect::<Vec<_>>();
    tags.sort_by_key(|(name, _)| name.to_lowercase());
    tags
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TaskTagRowSegment {
    text: String,
    color: Option<TagColor>,
}

fn format_task_tags_for_row(
    tags: &[(&str, TagColor)],
    width: u16,
    symbols: Symbols,
) -> Vec<TaskTagRowSegment> {
    if tags.is_empty() {
        return Vec::new();
    }
    let max_width = (width as usize / 2).max(8);
    let mut segments = Vec::new();
    let mut used = 0usize;
    for (tag, color) in tags {
        let chunk = format!("@{tag}");
        let next = if segments.is_empty() {
            task_tag_segment_content_width(chunk.as_str(), true, symbols)
        } else {
            task_tag_segment_content_width(chunk.as_str(), true, symbols) + 1
        };
        if used + next > max_width {
            break;
        }
        used += next;
        segments.push(TaskTagRowSegment {
            text: chunk,
            color: Some(*color),
        });
    }
    let remaining = tags.len().saturating_sub(segments.len());
    if remaining > 0 {
        let suffix = format!("+{remaining}");
        if !segments.is_empty()
            && used + task_tag_segment_content_width(suffix.as_str(), false, symbols) + 1
                <= max_width
        {
            segments.push(TaskTagRowSegment {
                text: suffix,
                color: None,
            });
        } else if segments.is_empty()
            && task_tag_segment_content_width(suffix.as_str(), false, symbols) <= max_width
        {
            segments.push(TaskTagRowSegment {
                text: suffix,
                color: None,
            });
        }
    }
    segments
}

fn task_tag_segments_width(segments: &[TaskTagRowSegment], symbols: Symbols) -> usize {
    segments
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            let width = task_tag_segment_content_width(
                segment.text.as_str(),
                segment.color.is_some(),
                symbols,
            );
            if index == 0 { width } else { width + 1 }
        })
        .sum()
}

fn task_tag_segments_spans(
    segments: &[TaskTagRowSegment],
    base_style: Style,
    selected: bool,
    palette: ThemePalette,
    symbols: Symbols,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (index, segment) in segments.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" ", base_style));
        }
        let style = if segment.color.is_none() {
            base_style.patch(Style::default().fg(palette.subtle_text))
        } else if selected {
            base_style
        } else if let Some(color) = segment.color {
            base_style
                .patch(Style::default().fg(palette.project_color(project_color_for_tag(color))))
        } else {
            base_style
        };
        if segment.color.is_none() {
            spans.push(Span::styled(segment.text.clone(), style));
            continue;
        }
        let Some(color) = segment.color else {
            continue;
        };
        let chip_color = palette.project_color(project_color_for_tag(color));
        if symbols.tag_chip_uses_background {
            let chip_bg = if selected { Color::White } else { chip_color };
            let chip_fg = if selected {
                Color::Black
            } else {
                contrasting_text_color(chip_bg)
            };
            spans.push(Span::styled(
                symbols.tag_chip_left,
                base_style.patch(Style::default().fg(chip_bg)),
            ));
            spans.push(Span::styled(
                segment.text.clone(),
                base_style.patch(Style::default().bg(chip_bg).fg(chip_fg)),
            ));
            spans.push(Span::styled(
                symbols.tag_chip_right,
                base_style.patch(Style::default().fg(chip_bg)),
            ));
        } else {
            let chip = format!(
                "{}{}{}",
                symbols.tag_chip_left, segment.text, symbols.tag_chip_right
            );
            spans.push(Span::styled(chip, style));
        }
    }
    spans
}

fn task_tag_segment_content_width(content: &str, is_tag: bool, symbols: Symbols) -> usize {
    if !is_tag {
        return content.width();
    }
    symbols.tag_chip_left.width() + content.width() + symbols.tag_chip_right.width()
}

fn contrasting_text_color(color: Color) -> Color {
    match color {
        Color::Rgb(r, g, b) => {
            let to_linear = |channel: u8| -> f64 {
                let value = f64::from(channel) / 255.0;
                if value <= 0.03928 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            let l = 0.2126 * to_linear(r) + 0.7152 * to_linear(g) + 0.0722 * to_linear(b);
            let contrast_white = (1.0 + 0.05) / (l + 0.05);
            let contrast_black = (l + 0.05) / 0.05;
            if contrast_white >= contrast_black {
                Color::White
            } else {
                Color::Black
            }
        }
        _ => Color::White,
    }
}

fn project_tree_line(
    row: ProjectTreeRowView,
    symbols: Symbols,
    width: u16,
    palette: ThemePalette,
) -> Line<'static> {
    let mut label = row.tree_prefix.clone();
    if row.is_favorite {
        label.push_str(symbols.favorite);
        label.push(' ');
    }
    let color = row
        .color
        .map(|color| palette.project_color(color))
        .unwrap_or(palette.text);
    let selection_style = if row.is_selected {
        Style::default()
            .fg(palette.text)
            .bg(palette.border)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let count_text = format!(" {}", row.task_count);
    let name_width = ((width as usize)
        .saturating_sub(label.width())
        .saturating_sub(symbols.project.width())
        .saturating_sub(1)
        .saturating_sub(count_text.width()))
    .max(1);
    let mut spans = vec![
        Span::styled(
            label,
            if row.is_selected {
                selection_style
            } else {
                selection_style.patch(Style::default().fg(palette.subtle_text))
            },
        ),
        Span::styled(
            format!("{} ", symbols.project),
            if row.is_selected {
                selection_style
            } else {
                selection_style.patch(Style::default().fg(color))
            },
        ),
        Span::styled(
            ellipsize_end(row.name.as_str(), name_width),
            if row.is_selected {
                selection_style
            } else {
                selection_style.patch(Style::default().fg(color))
            },
        ),
        Span::styled(
            count_text,
            if row.is_selected {
                Style::default().fg(palette.subtle_text).bg(palette.border)
            } else {
                Style::default().fg(palette.subtle_text)
            },
        ),
    ];
    let current_width = Line::from(spans.clone()).width();
    let padding = (width as usize).saturating_sub(current_width);
    if padding > 0 {
        spans.push(Span::styled(" ".repeat(padding), selection_style));
    }
    Line::from(spans)
}

fn tag_list_line(
    row: TagListRowView,
    symbols: Symbols,
    width: u16,
    palette: ThemePalette,
) -> Line<'static> {
    let mut label = String::new();
    if row.is_favorite {
        label.push_str(symbols.favorite);
        label.push(' ');
    }
    let color = row
        .color
        .map(|color| palette.project_color(project_color_for_tag(color)))
        .unwrap_or(palette.text);
    let selection_style = if row.is_selected {
        Style::default()
            .fg(palette.text)
            .bg(palette.border)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let count_text = format!(" {}", row.task_count);
    let name_width = (width as usize)
        .saturating_sub(label.width())
        .saturating_sub(symbols.tag.width())
        .saturating_sub(3)
        .saturating_sub(count_text.width())
        .max(1);
    let mut spans = vec![
        Span::styled(
            label,
            if row.is_selected {
                selection_style
            } else {
                selection_style.patch(Style::default().fg(palette.subtle_text))
            },
        ),
        Span::styled(
            format!("{} ", symbols.tag),
            if row.is_selected {
                selection_style
            } else {
                selection_style.patch(Style::default().fg(color))
            },
        ),
        Span::styled(
            ellipsize_end(row.name.as_str(), name_width),
            if row.is_selected {
                selection_style
            } else {
                selection_style.patch(Style::default().fg(color))
            },
        ),
        Span::styled(
            count_text,
            if row.is_selected {
                Style::default().fg(palette.subtle_text).bg(palette.border)
            } else {
                Style::default().fg(palette.subtle_text)
            },
        ),
    ];
    let current_width = Line::from(spans.clone()).width();
    let padding = (width as usize).saturating_sub(current_width);
    if padding > 0 {
        spans.push(Span::styled(" ".repeat(padding), selection_style));
    }
    Line::from(spans)
}

fn render_task_overlay(frame: &mut Frame<'_>, app: &App, symbols: Symbols, palette: ThemePalette) {
    if app.is_help_open() {
        render_help_dialog(frame, app, palette);
        return;
    }

    if let Some(sort_popup) = app.project_sort_popup_view() {
        let anchor = project_sort_popup_anchor(frame.area());
        render_project_sort_popup(frame, &sort_popup, anchor, symbols, palette);
        return;
    }

    if let Some(sort_popup) = app.tag_sort_popup_view() {
        let anchor = project_sort_popup_anchor(frame.area());
        render_tag_sort_popup(frame, &sort_popup, anchor, symbols, palette);
        return;
    }

    if let Some(sort_popup) = app.task_sort_popup_view() {
        let anchor = task_sort_popup_anchor(frame.area());
        render_task_sort_popup(frame, &sort_popup, anchor, symbols, palette);
        return;
    }

    if let Some(search) = app.task_search_view() {
        render_task_search_popup(frame, &search, symbols, palette);
        return;
    }

    if let Some(editor) = app.task_editor_view() {
        render_task_editor_popup(frame, &editor, symbols, palette);
        return;
    }

    if let Some(editor) = app.project_editor_view() {
        render_project_editor_popup(frame, &editor, symbols, palette);
        return;
    }

    if let Some(editor) = app.tag_editor_view() {
        render_tag_editor_popup(frame, &editor, symbols, palette);
        return;
    }

    if let Some(input) = app.task_input_view() {
        render_task_input_popup(frame, &input, symbols, palette);
        return;
    }

    if let Some(confirmation) = app.project_delete_confirmation_view() {
        render_project_delete_confirmation(frame, &confirmation, symbols, palette);
        return;
    }

    if let Some(confirmation) = app.tag_delete_confirmation_view() {
        render_tag_delete_confirmation(frame, &confirmation, symbols, palette);
        return;
    }

    if let Some(confirmation) = app.delete_confirmation_view() {
        render_delete_confirmation(frame, &confirmation, symbols, palette);
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

fn project_sort_popup_anchor(area: Rect) -> Rect {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);
    let left_sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(26),
            Constraint::Percentage(22),
            Constraint::Percentage(30),
            Constraint::Percentage(22),
        ])
        .split(columns[0]);
    let navigation_area = left_sections[2];

    Rect::new(
        navigation_area
            .x
            .saturating_add(navigation_area.width.saturating_sub(20)),
        navigation_area
            .y
            .saturating_add(navigation_area.height.saturating_sub(2)),
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
        let position = scrollbar_position_from_offset(start, lines.len(), visible_height);
        let mut scrollbar_state = ScrollbarState::default()
            .content_length(lines.len())
            .viewport_content_length(visible_height)
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
    symbols: Symbols,
    palette: ThemePalette,
) {
    let base_total_height = 11;
    let input_height = 3;
    let preview_height = preview_panel_required_height(&input.preview_panel, 3);
    let total_height = input_height + preview_height;
    let area = anchored_form_rect(frame.area(), 72, base_total_height, total_height);
    frame.render_widget(Clear, area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(input_height),
            Constraint::Length(preview_height),
        ])
        .split(area);
    render_task_input_box(frame, sections[0], input, symbols, palette);
    render_form_preview_panel(frame, sections[1], &input.preview_panel, palette);
    let input_area = sections[0];

    if !input.tag_suggestions.is_empty() {
        let dropdown_height = input.tag_suggestions.len().min(4) as u16 + 2;
        let visible_width = input_area.width.saturating_sub(4) as usize;
        let cursor_col = editor_cursor_display_column(&input.value, input.cursor, visible_width);
        let dropdown_area = project_parent_dropdown_rect(
            frame.area(),
            input_area,
            cursor_col as u16,
            project_parent_dropdown_width(input.tag_suggestions.as_slice()),
            dropdown_height,
        );
        render_task_tag_suggestions(frame, dropdown_area, input, palette);
    } else if !input.project_suggestions.is_empty() {
        let dropdown_height = input.project_suggestions.len().min(4) as u16 + 2;
        let visible_width = input_area.width.saturating_sub(4) as usize;
        let cursor_col = editor_cursor_display_column(&input.value, input.cursor, visible_width);
        let dropdown_area = project_parent_dropdown_rect(
            frame.area(),
            input_area,
            cursor_col as u16,
            project_parent_dropdown_width(input.project_suggestions.as_slice()),
            dropdown_height,
        );
        render_task_project_suggestions(frame, dropdown_area, input, palette);
    } else if !input.priority_suggestions.is_empty() {
        let dropdown_height = input.priority_suggestions.len().min(4) as u16 + 2;
        let visible_width = input_area.width.saturating_sub(4) as usize;
        let cursor_col = editor_cursor_display_column(&input.value, input.cursor, visible_width);
        let dropdown_area = project_parent_dropdown_rect(
            frame.area(),
            input_area,
            cursor_col as u16,
            project_parent_dropdown_width(input.priority_suggestions.as_slice()),
            dropdown_height,
        );
        render_task_priority_suggestions(frame, dropdown_area, input, palette);
    }
}

fn render_task_input_box(
    frame: &mut Frame<'_>,
    area: Rect,
    input: &TaskInputView,
    symbols: Symbols,
    palette: ThemePalette,
) {
    let visible_width = area.width.saturating_sub(4) as usize;
    let window = input_window_view(&input.value, input.cursor, visible_width);
    let lines = vec![Line::from(window.text)];
    let popup = Paragraph::new(lines).block(
        Block::default()
            .title(Span::styled(
                input.title,
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ))
            .title_bottom(task_input_shortcuts_line(symbols, palette))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.accent)),
    );

    frame.render_widget(popup, area);
    set_single_line_input_cursor(frame, area, window.cursor_col);
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

fn task_list_footer_hints(
    app: &App,
    symbols: Symbols,
    focused: bool,
    palette: ThemePalette,
) -> Line<'static> {
    if !focused {
        return Line::from("").right_aligned();
    }

    let done_filter_hint = if app.hides_completed_tasks() {
        " f hidden  "
    } else {
        " f shown  "
    };

    Line::from(vec![
        Span::styled(symbols.sort, Style::default().fg(palette.accent)),
        Span::styled(" o sort  ", Style::default().fg(palette.subtle_text)),
        Span::styled(symbols.visible, Style::default().fg(palette.accent)),
        Span::styled(done_filter_hint, Style::default().fg(palette.subtle_text)),
        Span::styled(symbols.done, Style::default().fg(palette.accent)),
        Span::styled(" space done", Style::default().fg(palette.subtle_text)),
    ])
    .right_aligned()
}

fn render_task_editor_popup(
    frame: &mut Frame<'_>,
    editor: &TaskEditorView,
    symbols: Symbols,
    palette: ThemePalette,
) {
    let base_total_height = 24;
    let form_height = 17;
    let preview_height = preview_panel_required_height(&editor.preview_panel, 3);
    let area = anchored_form_rect(
        frame.area(),
        96,
        base_total_height,
        form_height + preview_height,
    );
    frame.render_widget(Clear, area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(form_height),
            Constraint::Length(preview_height),
        ])
        .split(area);
    let form_block = Block::default()
        .title(Span::styled(
            editor.title,
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(editor_shortcuts_line(symbols, palette))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.accent));
    let form_inner = form_block.inner(sections[0]);
    frame.render_widget(form_block, sections[0]);
    let form_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(form_inner);

    render_editor_field(
        frame,
        form_rows[0],
        "Title [F1]",
        &editor.title_value,
        editor.title_cursor,
        editor.focus.title,
        None,
        palette,
    );
    render_editor_field(
        frame,
        form_rows[1],
        "Project [F2]",
        &editor.project_value,
        editor.project_cursor,
        editor.focus.project,
        Some("type to fuzzy-match a project"),
        palette,
    );

    render_editor_field(
        frame,
        form_rows[2],
        "Tags [F3]",
        &editor.tags_value,
        editor.tags_cursor,
        editor.focus.tags,
        Some("@work @deep"),
        palette,
    );

    let due_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(48),
            Constraint::Percentage(20),
            Constraint::Percentage(32),
        ])
        .split(form_rows[3]);
    render_editor_field(
        frame,
        due_row[0],
        "Due Date [F4]",
        &editor.due_date_value,
        editor.due_date_cursor,
        editor.focus.due_date,
        Some("YYYY-MM-DD"),
        palette,
    );
    render_editor_field(
        frame,
        due_row[1],
        "Priority [F5]",
        &editor.priority_value,
        editor.priority_cursor,
        editor.focus.priority,
        Some("p1..p4"),
        palette,
    );
    render_editor_field(
        frame,
        due_row[2],
        "Due Time [F6]",
        &editor.due_time_value,
        editor.due_time_cursor,
        editor.focus.due_time,
        Some("HH:MM"),
        palette,
    );
    render_editor_field(
        frame,
        form_rows[4],
        "Recurrence [F7]",
        &editor.recurrence_value,
        editor.recurrence_cursor,
        editor.focus.recurrence,
        Some("every monday at 9am"),
        palette,
    );
    render_form_preview_panel(frame, sections[1], &editor.preview_panel, palette);

    if (editor.focus.project || editor.focus.title) && !editor.project_suggestions.is_empty() {
        let dropdown_height = editor.project_suggestions.len().min(4) as u16 + 2;
        let project_anchor = if editor.focus.title {
            form_rows[0]
        } else {
            form_rows[1]
        };
        let visible_width = project_anchor.width.saturating_sub(4) as usize;
        let cursor_col = editor_cursor_display_column(
            if editor.focus.title {
                &editor.title_value
            } else {
                &editor.project_value
            },
            if editor.focus.title {
                editor.title_cursor
            } else {
                editor.project_cursor
            },
            visible_width,
        );
        let dropdown_area = project_parent_dropdown_rect(
            frame.area(),
            project_anchor,
            cursor_col as u16,
            project_parent_dropdown_width(editor.project_suggestions.as_slice()),
            dropdown_height,
        );
        render_editor_project_suggestions(frame, dropdown_area, editor, palette);
    }

    if (editor.focus.tags || editor.focus.title) && !editor.tag_suggestions.is_empty() {
        let dropdown_height = editor.tag_suggestions.len().min(4) as u16 + 2;
        let tags_anchor = if editor.focus.title {
            form_rows[0]
        } else {
            form_rows[2]
        };
        let visible_width = tags_anchor.width.saturating_sub(4) as usize;
        let cursor_col = editor_cursor_display_column(
            if editor.focus.title {
                &editor.title_value
            } else {
                &editor.tags_value
            },
            if editor.focus.title {
                editor.title_cursor
            } else {
                editor.tags_cursor
            },
            visible_width,
        );
        let dropdown_area = project_parent_dropdown_rect(
            frame.area(),
            tags_anchor,
            cursor_col as u16,
            project_parent_dropdown_width(editor.tag_suggestions.as_slice()),
            dropdown_height,
        );
        render_editor_tag_suggestions(frame, dropdown_area, editor, palette);
    }

    if (editor.focus.priority || editor.focus.title) && !editor.priority_suggestions.is_empty() {
        let dropdown_height = editor.priority_suggestions.len().min(4) as u16 + 2;
        let priority_anchor = if editor.focus.title {
            form_rows[0]
        } else {
            due_row[1]
        };
        let visible_width = priority_anchor.width.saturating_sub(4) as usize;
        let cursor_col = editor_cursor_display_column(
            if editor.focus.title {
                &editor.title_value
            } else {
                &editor.priority_value
            },
            if editor.focus.title {
                editor.title_cursor
            } else {
                editor.priority_cursor
            },
            visible_width,
        );
        let dropdown_area = project_parent_dropdown_rect(
            frame.area(),
            priority_anchor,
            cursor_col as u16,
            project_parent_dropdown_width(editor.priority_suggestions.as_slice()),
            dropdown_height,
        );
        render_editor_priority_suggestions(frame, dropdown_area, editor, palette);
    }

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
    let visible_width = area.width.saturating_sub(4) as usize;
    let window = input_window_view(value, cursor, visible_width);
    let text = if focused {
        vec![Line::from(Span::styled(
            window.text.clone(),
            Style::default().fg(palette.text),
        ))]
    } else if value.is_empty() {
        vec![Line::from(Span::styled(
            placeholder.unwrap_or(""),
            Style::default()
                .fg(palette.subtle_text)
                .add_modifier(Modifier::DIM),
        ))]
    } else if area.height > 3 {
        vec![Line::from(ellipsize_end(value, visible_width))]
    } else {
        vec![Line::from(window.text)]
    };
    let widget = Paragraph::new(text).wrap(Wrap { trim: false }).block(
        Block::default()
            .title(Span::styled(label, border_style))
            .borders(Borders::ALL)
            .border_style(border_style),
    );
    frame.render_widget(widget, area);
    if focused {
        set_single_line_input_cursor(frame, area, window.cursor_col);
    }
}

fn render_editor_project_suggestions(
    frame: &mut Frame<'_>,
    area: Rect,
    editor: &TaskEditorView,
    palette: ThemePalette,
) {
    let content_width = area.width.saturating_sub(2) as usize;
    let lines = editor
        .project_suggestions
        .iter()
        .enumerate()
        .map(|(index, suggestion)| {
            let style = if index
                == editor
                    .selected_project_suggestion
                    .min(editor.project_suggestions.len().saturating_sub(1))
            {
                Style::default()
                    .fg(palette.text)
                    .bg(palette.border)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.text)
            };
            let value = ellipsize_end(suggestion, content_width);
            let padding = " ".repeat(content_width.saturating_sub(value.width()));
            Line::from(vec![Span::styled(format!("{value}{padding}"), style)])
        })
        .collect::<Vec<_>>();

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(Span::styled(
                        "Project",
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::ALL)
                    .border_style(
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .style(Style::default().bg(Color::Rgb(4, 4, 8))),
        area,
    );
}

fn render_editor_tag_suggestions(
    frame: &mut Frame<'_>,
    area: Rect,
    editor: &TaskEditorView,
    palette: ThemePalette,
) {
    let content_width = area.width.saturating_sub(2) as usize;
    let lines = editor
        .tag_suggestions
        .iter()
        .enumerate()
        .map(|(index, suggestion)| {
            let style = if index
                == editor
                    .selected_tag_suggestion
                    .min(editor.tag_suggestions.len().saturating_sub(1))
            {
                Style::default()
                    .fg(palette.text)
                    .bg(palette.border)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.text)
            };
            let value = ellipsize_end(suggestion, content_width);
            let padding = " ".repeat(content_width.saturating_sub(value.width()));
            Line::from(vec![Span::styled(format!("{value}{padding}"), style)])
        })
        .collect::<Vec<_>>();

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(Span::styled(
                        "Tag",
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::ALL)
                    .border_style(
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .style(Style::default().bg(Color::Rgb(4, 4, 8))),
        area,
    );
}

fn render_editor_priority_suggestions(
    frame: &mut Frame<'_>,
    area: Rect,
    editor: &TaskEditorView,
    palette: ThemePalette,
) {
    let content_width = area.width.saturating_sub(2) as usize;
    let lines = editor
        .priority_suggestions
        .iter()
        .enumerate()
        .map(|(index, suggestion)| {
            let style = if index
                == editor
                    .selected_priority_suggestion
                    .min(editor.priority_suggestions.len().saturating_sub(1))
            {
                Style::default()
                    .fg(palette.text)
                    .bg(palette.border)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.text)
            };
            let value = ellipsize_end(suggestion, content_width);
            let padding = " ".repeat(content_width.saturating_sub(value.width()));
            Line::from(vec![Span::styled(format!("{value}{padding}"), style)])
        })
        .collect::<Vec<_>>();

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(Span::styled(
                        "Priority",
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::ALL)
                    .border_style(
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .style(Style::default().bg(Color::Rgb(4, 4, 8))),
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
            "F1-F7 fields  ↑/↓ suggestions  F8 calendar  F9 clear due",
            Style::default().fg(palette.subtle_text),
        )])
        .right_aligned();
    }

    Line::from(vec![
        Span::styled("F1-F7", Style::default().fg(palette.subtle_text)),
        Span::raw(" fields  "),
        Span::styled("F8", Style::default().fg(palette.subtle_text)),
        Span::raw(" 󰃭  "),
        Span::styled("F9", Style::default().fg(palette.subtle_text)),
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
    symbols: Symbols,
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
    ];
    let popup = Paragraph::new(lines).block(
        Block::default()
            .title(Span::styled(
                "Remove Task",
                Style::default()
                    .fg(palette.error)
                    .add_modifier(Modifier::BOLD),
            ))
            .title_bottom(confirm_shortcuts_line(symbols, palette))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.error)),
    );

    frame.render_widget(popup, area);
}

fn render_project_delete_confirmation(
    frame: &mut Frame<'_>,
    confirmation: &ProjectDeleteConfirmationView,
    symbols: Symbols,
    palette: ThemePalette,
) {
    let area = centered_rect(frame.area(), 64, 7);
    frame.render_widget(Clear, area);

    let lines = vec![
        Line::from("Remove this project and its subtree?"),
        Line::from(""),
        Line::from(Span::styled(
            format!("\"{}\"", confirmation.project_name),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("Tasks in this subtree will be soft-deleted."),
        Line::from("History links will be preserved."),
    ];
    let popup = Paragraph::new(lines).block(
        Block::default()
            .title(Span::styled(
                "Remove Project",
                Style::default()
                    .fg(palette.error)
                    .add_modifier(Modifier::BOLD),
            ))
            .title_bottom(confirm_shortcuts_line(symbols, palette))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.error)),
    );

    frame.render_widget(popup, area);
}

fn render_project_editor_popup(
    frame: &mut Frame<'_>,
    editor: &ProjectEditorView,
    symbols: Symbols,
    palette: ThemePalette,
) {
    let show_parent_dropdown =
        (editor.focus.name || editor.focus.parent) && !editor.parent_suggestions.is_empty();
    let base_total_height = 17;
    let form_height = 11;
    let preview_height = preview_panel_required_height(&editor.preview_panel, 3);
    let area = anchored_form_rect(
        frame.area(),
        72,
        base_total_height,
        form_height + preview_height,
    );
    frame.render_widget(Clear, area);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(form_height),
            Constraint::Length(preview_height),
        ])
        .split(area);

    let form_block = Block::default()
        .title(Span::styled(
            editor.title,
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(project_editor_shortcuts_line(symbols, palette))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.accent));
    let form_inner = form_block.inner(sections[0]);
    frame.render_widget(form_block, sections[0]);
    let form_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(form_inner);

    render_editor_field(
        frame,
        form_rows[0],
        "Name [F1]",
        &editor.name_value,
        editor.name_cursor,
        editor.focus.name,
        None,
        palette,
    );
    render_editor_field(
        frame,
        form_rows[1],
        "Parent [F2]",
        &editor.parent_value,
        editor.parent_cursor,
        editor.focus.parent,
        Some("Type to fuzzy-match a parent project"),
        palette,
    );
    let meta_row = form_rows[2];
    let meta_columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(meta_row);
    render_project_value_field(
        frame,
        meta_columns[0],
        "Color [F3]",
        &editor.color_label,
        editor.focus.color,
        Some(Style::default().fg(palette.project_color(editor.color_value))),
        palette,
    );
    render_project_value_field(
        frame,
        meta_columns[1],
        "Favorite [F4]",
        if editor.is_favorite { "yes" } else { "no" },
        editor.focus.favorite,
        None,
        palette,
    );

    render_form_preview_panel(frame, sections[1], &editor.preview_panel, palette);

    if show_parent_dropdown {
        let dropdown_height = editor.parent_suggestions.len().min(4) as u16 + 2;
        let (anchor, value, cursor) = if editor.focus.parent {
            (form_rows[1], &editor.parent_value, editor.parent_cursor)
        } else {
            (form_rows[0], &editor.name_value, editor.name_cursor)
        };
        let visible_width = anchor.width.saturating_sub(4) as usize;
        let cursor_col = editor_cursor_display_column(value, cursor, visible_width);
        let dropdown_area = project_parent_dropdown_rect(
            frame.area(),
            anchor,
            cursor_col as u16,
            project_parent_dropdown_width(editor.parent_suggestions.as_slice()),
            dropdown_height,
        );
        render_project_parent_suggestions(frame, dropdown_area, editor, palette);
    }
}

fn render_tag_editor_popup(
    frame: &mut Frame<'_>,
    editor: &TagEditorView,
    symbols: Symbols,
    palette: ThemePalette,
) {
    let base_total_height = 14;
    let form_height = 8;
    let preview_height = preview_panel_required_height(&editor.preview_panel, 3);
    let area = anchored_form_rect(
        frame.area(),
        64,
        base_total_height,
        form_height + preview_height,
    );
    frame.render_widget(Clear, area);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(form_height),
            Constraint::Length(preview_height),
        ])
        .split(area);

    let form_block = Block::default()
        .title(Span::styled(
            editor.title,
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(project_editor_shortcuts_line(symbols, palette))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.accent));
    let form_inner = form_block.inner(sections[0]);
    frame.render_widget(form_block, sections[0]);
    let form_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(3)])
        .split(form_inner);

    render_editor_field(
        frame,
        form_rows[0],
        "Name [F1]",
        &editor.name_value,
        editor.name_cursor,
        editor.focus.name,
        Some("Use @ in task title to assign"),
        palette,
    );
    let meta_columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(form_rows[1]);
    render_project_value_field(
        frame,
        meta_columns[0],
        "Color [F2]",
        &editor.color_label,
        editor.focus.color,
        Some(Style::default().fg(palette.project_color(project_color_for_tag(editor.color_value)))),
        palette,
    );
    render_project_value_field(
        frame,
        meta_columns[1],
        "Favorite [F3]",
        if editor.is_favorite { "yes" } else { "no" },
        editor.focus.favorite,
        None,
        palette,
    );

    render_form_preview_panel(frame, sections[1], &editor.preview_panel, palette);
}

fn render_tag_delete_confirmation(
    frame: &mut Frame<'_>,
    confirmation: &TagDeleteConfirmationView,
    symbols: Symbols,
    palette: ThemePalette,
) {
    let area = centered_rect(frame.area(), 64, 7);
    frame.render_widget(Clear, area);

    let lines = vec![
        Line::from("Remove this tag?"),
        Line::from(""),
        Line::from(Span::styled(
            format!("\"{}\"", confirmation.tag_name),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("Tag assignments will be detached from tasks."),
        Line::from("Tasks themselves are kept."),
    ];
    let popup = Paragraph::new(lines).block(
        Block::default()
            .title(Span::styled(
                "Remove Tag",
                Style::default()
                    .fg(palette.error)
                    .add_modifier(Modifier::BOLD),
            ))
            .title_bottom(confirm_shortcuts_line(symbols, palette))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.error)),
    );

    frame.render_widget(popup, area);
}

fn render_form_preview_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    preview_panel: &FormPreviewPanelView,
    palette: ThemePalette,
) {
    let content_width = area.width.saturating_sub(4) as usize;
    let inner_height = area.height.saturating_sub(2) as usize;
    let lines = preview_panel_lines(preview_panel, content_width, inner_height, palette);

    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(palette.border)),
        ),
        area,
    );
}

fn preview_panel_required_height(
    preview_panel: &FormPreviewPanelView,
    min_inner_height: u16,
) -> u16 {
    let info_lines = preview_panel.preview_lines.len() as u16;
    let tips_lines = preview_panel.tips.len() as u16;
    let required_inner = if tips_lines > 0 {
        info_lines.saturating_add(1).saturating_add(tips_lines)
    } else {
        info_lines.max(1)
    };
    required_inner.max(min_inner_height).saturating_add(2)
}

fn preview_panel_lines(
    preview_panel: &FormPreviewPanelView,
    content_width: usize,
    inner_height: usize,
    palette: ThemePalette,
) -> Vec<Line<'static>> {
    if inner_height == 0 {
        return Vec::new();
    }

    let info_lines = preview_panel
        .preview_lines
        .iter()
        .map(|preview_line| render_preview_line(preview_line, content_width, palette))
        .collect::<Vec<_>>();
    let tip_lines = preview_panel
        .tips
        .iter()
        .map(|tip| {
            Line::from(Span::styled(
                ellipsize_end(tip, content_width),
                Style::default()
                    .fg(palette.subtle_text)
                    .add_modifier(Modifier::DIM),
            ))
        })
        .collect::<Vec<_>>();

    if tip_lines.is_empty() {
        return info_lines.into_iter().take(inner_height).collect();
    }

    let shown_tip_count = tip_lines.len().min(inner_height.saturating_sub(1));
    let reserve_separator = inner_height > shown_tip_count;
    let info_capacity = inner_height
        .saturating_sub(shown_tip_count)
        .saturating_sub(if reserve_separator { 1 } else { 0 });
    let mut lines = info_lines
        .into_iter()
        .take(info_capacity)
        .collect::<Vec<_>>();

    if reserve_separator {
        lines.push(Line::from(""));
    }
    let spacer_lines = inner_height
        .saturating_sub(lines.len())
        .saturating_sub(shown_tip_count);
    lines.extend((0..spacer_lines).map(|_| Line::from("")));
    lines.extend(tip_lines.into_iter().take(shown_tip_count));
    lines
}

fn render_preview_line(
    preview_line: &PreviewLineView,
    content_width: usize,
    palette: ThemePalette,
) -> Line<'static> {
    match preview_line {
        PreviewLineView::KeyValue {
            label,
            value,
            emphasized,
            dimmed,
        } => {
            let mut value_style = Style::default().fg(palette.text);
            if *emphasized {
                value_style = value_style.add_modifier(Modifier::BOLD);
            }
            if *dimmed {
                value_style = value_style
                    .fg(palette.subtle_text)
                    .add_modifier(Modifier::DIM);
            }
            let plain = format!("{label}: {value}");
            let clipped = ellipsize_end(plain.as_str(), content_width);
            let prefix = format!("{label}: ");
            if clipped.starts_with(prefix.as_str()) {
                let suffix = clipped[prefix.len()..].to_string();
                Line::from(vec![
                    Span::styled(prefix, Style::default().fg(palette.subtle_text)),
                    Span::styled(suffix, value_style),
                ])
            } else {
                Line::from(Span::styled(clipped, value_style))
            }
        }
        PreviewLineView::Text { text, dimmed } => {
            let mut style = Style::default().fg(palette.text);
            if *dimmed {
                style = style.fg(palette.subtle_text).add_modifier(Modifier::DIM);
            }
            Line::from(Span::styled(
                ellipsize_end(text.as_str(), content_width),
                style,
            ))
        }
    }
}

fn render_project_parent_suggestions(
    frame: &mut Frame<'_>,
    area: Rect,
    editor: &ProjectEditorView,
    palette: ThemePalette,
) {
    let content_width = area.width.saturating_sub(2) as usize;
    let lines = editor
        .parent_suggestions
        .iter()
        .enumerate()
        .map(|(index, suggestion)| {
            let style = if index
                == editor
                    .selected_parent_suggestion
                    .min(editor.parent_suggestions.len().saturating_sub(1))
            {
                Style::default()
                    .fg(palette.text)
                    .bg(palette.border)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.text)
            };
            let value = ellipsize_end(suggestion, content_width);
            let padding = " ".repeat(content_width.saturating_sub(value.width()));
            Line::from(vec![Span::styled(format!("{value}{padding}"), style)])
        })
        .collect::<Vec<_>>();

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(Span::styled(
                        "Parent Project",
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::ALL)
                    .border_style(
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .style(Style::default().bg(Color::Rgb(4, 4, 8))),
        area,
    );
}

fn render_task_project_suggestions(
    frame: &mut Frame<'_>,
    area: Rect,
    input: &TaskInputView,
    palette: ThemePalette,
) {
    let content_width = area.width.saturating_sub(2) as usize;
    let lines = input
        .project_suggestions
        .iter()
        .enumerate()
        .map(|(index, suggestion)| {
            let style = if index
                == input
                    .selected_project_suggestion
                    .min(input.project_suggestions.len().saturating_sub(1))
            {
                Style::default()
                    .fg(palette.text)
                    .bg(palette.border)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.text)
            };
            let value = ellipsize_end(suggestion, content_width);
            let padding = " ".repeat(content_width.saturating_sub(value.width()));
            Line::from(vec![Span::styled(format!("{value}{padding}"), style)])
        })
        .collect::<Vec<_>>();

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(Span::styled(
                        "Project",
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::ALL)
                    .border_style(
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .style(Style::default().bg(Color::Rgb(4, 4, 8))),
        area,
    );
}

fn render_task_tag_suggestions(
    frame: &mut Frame<'_>,
    area: Rect,
    input: &TaskInputView,
    palette: ThemePalette,
) {
    let content_width = area.width.saturating_sub(2) as usize;
    let lines = input
        .tag_suggestions
        .iter()
        .enumerate()
        .map(|(index, suggestion)| {
            let style = if index
                == input
                    .selected_tag_suggestion
                    .min(input.tag_suggestions.len().saturating_sub(1))
            {
                Style::default()
                    .fg(palette.text)
                    .bg(palette.border)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.text)
            };
            let value = ellipsize_end(suggestion, content_width);
            let padding = " ".repeat(content_width.saturating_sub(value.width()));
            Line::from(vec![Span::styled(format!("{value}{padding}"), style)])
        })
        .collect::<Vec<_>>();

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(Span::styled(
                        "Tag",
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::ALL)
                    .border_style(
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .style(Style::default().bg(Color::Rgb(4, 4, 8))),
        area,
    );
}

fn render_task_priority_suggestions(
    frame: &mut Frame<'_>,
    area: Rect,
    input: &TaskInputView,
    palette: ThemePalette,
) {
    let content_width = area.width.saturating_sub(2) as usize;
    let lines = input
        .priority_suggestions
        .iter()
        .enumerate()
        .map(|(index, suggestion)| {
            let style = if index
                == input
                    .selected_priority_suggestion
                    .min(input.priority_suggestions.len().saturating_sub(1))
            {
                Style::default()
                    .fg(palette.text)
                    .bg(palette.border)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.text)
            };
            let value = ellipsize_end(suggestion, content_width);
            let padding = " ".repeat(content_width.saturating_sub(value.width()));
            Line::from(vec![Span::styled(format!("{value}{padding}"), style)])
        })
        .collect::<Vec<_>>();

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(Span::styled(
                        "Priority",
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::ALL)
                    .border_style(
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .style(Style::default().bg(Color::Rgb(4, 4, 8))),
        area,
    );
}

fn render_project_value_field(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    value: &str,
    focused: bool,
    value_style: Option<Style>,
    palette: ThemePalette,
) {
    let border_style = if focused {
        Style::default()
            .fg(palette.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(palette.subtle_text)
    };

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            ellipsize_end(value, area.width.saturating_sub(4) as usize),
            value_style.unwrap_or_else(|| Style::default().fg(palette.text)),
        )))
        .block(
            Block::default()
                .title(Span::styled(label, border_style))
                .borders(Borders::ALL)
                .border_style(border_style),
        ),
        area,
    );
}

fn render_task_search_popup(
    frame: &mut Frame<'_>,
    search: &TaskSearchView,
    symbols: Symbols,
    palette: ThemePalette,
) {
    let area = centered_rect(frame.area(), 72, 8);
    frame.render_widget(Clear, area);

    let visible_width = area.width.saturating_sub(4) as usize;
    let query_window = input_window_view(&search.query, search.cursor, visible_width);
    let mut lines = vec![Line::from(query_window.text)];
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
            .title_bottom(search_shortcuts_line(symbols, palette))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.accent)),
    );

    frame.render_widget(popup, area);
    set_single_line_input_cursor(frame, area, query_window.cursor_col);
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

fn render_project_sort_popup(
    frame: &mut Frame<'_>,
    popup: &ProjectSortPopupView,
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

fn render_tag_sort_popup(
    frame: &mut Frame<'_>,
    popup: &TagSortPopupView,
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

fn task_input_shortcuts_line(symbols: Symbols, palette: ThemePalette) -> Line<'static> {
    if symbols.tasks == "#" {
        return Line::from(vec![Span::styled(
            "↑/↓ move  Tab accept #/@  Enter save  Esc cancel",
            Style::default().fg(palette.subtle_text),
        )])
        .right_aligned();
    }

    Line::from(vec![
        Span::styled("↑/↓", Style::default().fg(palette.subtle_text)),
        Span::raw(" move  "),
        Span::styled("⇥", Style::default().fg(palette.subtle_text)),
        Span::raw(" #/@  "),
        Span::styled("󰌑", Style::default().fg(palette.subtle_text)),
        Span::raw(" save  "),
        Span::styled(symbols.voided, Style::default().fg(palette.subtle_text)),
        Span::raw(" esc"),
    ])
    .right_aligned()
}

fn project_editor_shortcuts_line(symbols: Symbols, palette: ThemePalette) -> Line<'static> {
    if symbols.tasks == "#" {
        return Line::from(vec![Span::styled(
            "Tab accept parent/next  F1-F4 field  h/l change  Enter save",
            Style::default().fg(palette.subtle_text),
        )])
        .right_aligned();
    }

    Line::from(vec![
        Span::styled("⇥", Style::default().fg(palette.subtle_text)),
        Span::raw(" parent/next  "),
        Span::styled("F1-F4", Style::default().fg(palette.subtle_text)),
        Span::raw(" field  "),
        Span::styled("←/→", Style::default().fg(palette.subtle_text)),
        Span::raw(" h/l  "),
        Span::styled("󰌑", Style::default().fg(palette.subtle_text)),
        Span::raw(" save"),
    ])
    .right_aligned()
}

fn project_parent_dropdown_rect(
    frame: Rect,
    name_field: Rect,
    cursor_col: u16,
    width: u16,
    height: u16,
) -> Rect {
    let min_x = frame.x.saturating_add(1);
    let max_x = frame
        .x
        .saturating_add(frame.width.saturating_sub(width).saturating_sub(1));
    let cursor_x = name_field.x.saturating_add(2).saturating_add(cursor_col);
    let x = cursor_x.clamp(min_x, max_x.max(min_x));

    let frame_bottom = frame.y.saturating_add(frame.height);
    let below_y = name_field
        .y
        .saturating_add(name_field.height.saturating_sub(1));
    let above_y = name_field.y.saturating_sub(height.saturating_sub(1));
    let can_place_below = below_y.saturating_add(height) <= frame_bottom;

    let y = if can_place_below {
        below_y
    } else {
        above_y.clamp(
            frame.y.saturating_add(1),
            frame_bottom.saturating_sub(height),
        )
    };

    Rect::new(x, y, width, height)
}

fn project_parent_dropdown_width(suggestions: &[String]) -> u16 {
    let content = suggestions
        .iter()
        .map(|suggestion| suggestion.width())
        .max()
        .unwrap_or(16)
        .saturating_add(2);
    (content as u16).clamp(22, 56)
}

fn editor_cursor_display_column(value: &str, cursor: usize, max_width: usize) -> usize {
    input_window_view(value, cursor, max_width).cursor_col
}

fn search_shortcuts_line(symbols: Symbols, palette: ThemePalette) -> Line<'static> {
    if symbols.tasks == "#" {
        return Line::from(vec![Span::styled(
            "j/k move  Enter assign  Esc cancel",
            Style::default().fg(palette.subtle_text),
        )])
        .right_aligned();
    }

    Line::from(vec![
        Span::styled("󰌑", Style::default().fg(palette.subtle_text)),
        Span::raw(" assign  "),
        Span::styled(symbols.voided, Style::default().fg(palette.subtle_text)),
        Span::raw(" esc  "),
        Span::styled("󰄾", Style::default().fg(palette.subtle_text)),
        Span::raw(" j/k"),
    ])
    .right_aligned()
}

fn confirm_shortcuts_line(symbols: Symbols, palette: ThemePalette) -> Line<'static> {
    if symbols.tasks == "#" {
        return Line::from(vec![Span::styled(
            "Enter/y confirm  Esc/n cancel",
            Style::default().fg(palette.subtle_text),
        )])
        .right_aligned();
    }

    Line::from(vec![
        Span::styled("󰄵", Style::default().fg(palette.subtle_text)),
        Span::raw(" enter/y  "),
        Span::styled(symbols.voided, Style::default().fg(palette.subtle_text)),
        Span::raw(" esc/n"),
    ])
    .right_aligned()
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let popup_width = width.min(area.width.saturating_sub(2)).max(1);
    let popup_height = height.min(area.height.saturating_sub(2)).max(1);
    let x = area.x + area.width.saturating_sub(popup_width) / 2;
    let y = area.y + area.height.saturating_sub(popup_height) / 2;
    Rect::new(x, y, popup_width, popup_height)
}

fn anchored_form_rect(area: Rect, width: u16, base_height: u16, actual_height: u16) -> Rect {
    let popup_width = width.min(area.width.saturating_sub(2)).max(1);
    let centered_base = centered_rect(area, width, base_height);
    let popup_height = actual_height.min(area.height.saturating_sub(2)).max(1);
    let max_y = area.y + area.height.saturating_sub(popup_height);
    let y = centered_base.y.min(max_y);
    Rect::new(centered_base.x, y, popup_width, popup_height)
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
    width: u16,
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
            let left_text = format!(
                "{}  C{:>2} V{:>2}  {}",
                summary.day.format("%a %d"),
                summary.completed_sessions,
                summary.voided_sessions,
                bar,
            );
            let right_text = if total == 0 {
                format!(
                    "{} / {}  -",
                    format_duration_seconds(summary.focus_seconds),
                    format_duration_seconds(summary.break_seconds)
                )
            } else {
                format!(
                    "{} / {}",
                    format_duration_seconds(summary.focus_seconds),
                    format_duration_seconds(summary.break_seconds)
                )
            };
            let left_width = UnicodeWidthStr::width(left_text.as_str());
            let right_width = UnicodeWidthStr::width(right_text.as_str());
            let spacing = (width as usize)
                .saturating_sub(left_width)
                .saturating_sub(right_width)
                .max(2);

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
                Span::raw(" ".repeat(spacing)),
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

fn history_title(
    active_tab: HistoryPanelTab,
    symbols: Symbols,
    palette: ThemePalette,
) -> Line<'static> {
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
        Span::styled(format!("{} ", symbols.today), today_style),
        Span::styled("Today", today_style),
        Span::raw(" - "),
        Span::styled(format!("{} ", symbols.stats), weekly_style),
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

fn selectable_count_line(
    label: &str,
    count: usize,
    selected: bool,
    width: u16,
    palette: ThemePalette,
) -> Line<'static> {
    let base_style = if selected {
        Style::default()
            .fg(palette.text)
            .bg(palette.border)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(palette.text)
    };
    let count_style = if selected {
        Style::default().fg(palette.subtle_text).bg(palette.border)
    } else {
        Style::default().fg(palette.subtle_text)
    };
    let count_text = format!("{count}");
    let count_width = count_text.width() + 2;
    let label_width = (width as usize).saturating_sub(count_width + 2);
    let mut spans = vec![
        Span::styled(
            ellipsize_end(&format!("  {label}"), label_width),
            base_style,
        ),
        Span::styled(" ".to_string(), base_style),
        Span::styled(count_text, count_style),
    ];
    let current_width = Line::from(spans.clone()).width();
    let padding = (width as usize).saturating_sub(current_width);
    if padding > 0 {
        spans.push(Span::styled(" ".repeat(padding), base_style));
    }
    Line::from(spans)
}

fn navigation_footer_hint(symbols: Symbols, palette: ThemePalette) -> Line<'static> {
    let jump_label = if symbols.tasks == "#" {
        "Home/End"
    } else {
        "↤/↦"
    };
    Line::from(vec![
        Span::styled("j/k", Style::default().fg(palette.accent)),
        Span::styled(" move  ", Style::default().fg(palette.subtle_text)),
        Span::styled(jump_label, Style::default().fg(palette.accent)),
        Span::styled(" jump ", Style::default().fg(palette.subtle_text)),
    ])
    .right_aligned()
}

fn projects_sort_footer(app: &App, symbols: Symbols, palette: ThemePalette) -> Line<'static> {
    let sort_prefix = if symbols.tasks == "#" {
        "sort"
    } else {
        symbols.sort
    };
    Line::from(vec![Span::styled(
        format!(
            " {} {} ",
            sort_prefix,
            app.project_sort_order().short_label()
        ),
        Style::default().fg(palette.subtle_text),
    )])
}

fn tags_sort_footer(app: &App, symbols: Symbols, palette: ThemePalette) -> Line<'static> {
    let sort_prefix = if symbols.tasks == "#" {
        "sort"
    } else {
        symbols.sort
    };
    Line::from(vec![Span::styled(
        format!(" {} {} ", sort_prefix, app.tag_sort_order().short_label()),
        Style::default().fg(palette.subtle_text),
    )])
}

fn projects_footer_hints(symbols: Symbols, focused: bool, palette: ThemePalette) -> Line<'static> {
    if !focused {
        return Line::from("").right_aligned();
    }

    let project_new_icon = if symbols.tasks == "#" { "+" } else { "✚" };

    Line::from(vec![
        Span::styled(symbols.sort, Style::default().fg(palette.accent)),
        Span::styled(" o sort  ", Style::default().fg(palette.subtle_text)),
        Span::styled(project_new_icon, Style::default().fg(palette.accent)),
        Span::styled(" C new  ", Style::default().fg(palette.subtle_text)),
        Span::styled(symbols.tasks, Style::default().fg(palette.accent)),
        Span::styled(" c ", Style::default().fg(palette.subtle_text)),
    ])
    .right_aligned()
}

fn tags_footer_hints(symbols: Symbols, focused: bool, palette: ThemePalette) -> Line<'static> {
    if !focused {
        return Line::from("").right_aligned();
    }
    let tag_new_icon = if symbols.tasks == "#" { "+" } else { "✚" };
    Line::from(vec![
        Span::styled(symbols.sort, Style::default().fg(palette.accent)),
        Span::styled(" o sort  ", Style::default().fg(palette.subtle_text)),
        Span::styled(tag_new_icon, Style::default().fg(palette.accent)),
        Span::styled(" C new  ", Style::default().fg(palette.subtle_text)),
        Span::styled(symbols.tasks, Style::default().fg(palette.accent)),
        Span::styled(" c ", Style::default().fg(palette.subtle_text)),
    ])
    .right_aligned()
}

fn task_view_symbol(view: TaskView, symbols: Symbols) -> &'static str {
    match view {
        TaskView::All => symbols.tasks,
        TaskView::Inbox => symbols.inbox,
        TaskView::Today => symbols.today,
        TaskView::Soon => symbols.soon,
    }
}

fn navigation_title(
    active_tab: SidebarTab,
    symbols: Symbols,
    palette: ThemePalette,
) -> Line<'static> {
    let nav_style = if active_tab == SidebarTab::Navigation {
        Style::default().fg(palette.accent)
    } else {
        Style::default().fg(palette.subtle_text)
    };
    let filters_style = if active_tab == SidebarTab::FiltersTags {
        Style::default().fg(palette.accent)
    } else {
        Style::default().fg(palette.subtle_text)
    };
    let projects_style = if active_tab == SidebarTab::Projects {
        Style::default().fg(palette.accent)
    } else {
        Style::default().fg(palette.subtle_text)
    };

    Line::from(vec![
        Span::raw("[3] "),
        Span::styled(format!("{} ", symbols.tasks), nav_style),
        Span::styled("Navigation", nav_style),
        Span::raw(" - "),
        Span::styled("[4] Filters & Tags", filters_style),
        Span::raw(" - "),
        Span::styled(format!("{} ", symbols.project), projects_style),
        Span::styled("[5] Projects", projects_style),
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
        Span::raw("[7] "),
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

#[derive(Debug, Clone)]
struct InputWindowView {
    text: String,
    cursor_col: usize,
}

fn input_window_view(text: &str, cursor: usize, max_width: usize) -> InputWindowView {
    const ELLIPSIS: &str = "…";

    if max_width == 0 {
        return InputWindowView {
            text: String::new(),
            cursor_col: 0,
        };
    }
    let safe_cursor = cursor.min(text.len());
    let before = &text[..safe_cursor];

    if UnicodeWidthStr::width(text) <= max_width {
        return InputWindowView {
            text: text.to_string(),
            cursor_col: UnicodeWidthStr::width(before).min(max_width.saturating_sub(1)),
        };
    }

    let after = &text[safe_cursor..];

    if UnicodeWidthStr::width(before) <= max_width / 2 {
        let rendered = ellipsize_end(text, max_width);
        let cursor_col = UnicodeWidthStr::width(before).min(max_width.saturating_sub(1));
        return InputWindowView {
            text: rendered,
            cursor_col,
        };
    }
    if UnicodeWidthStr::width(after) <= max_width / 2 {
        let mut suffix_start = text.len();
        let mut suffix_width = 1;
        for (index, character) in text.char_indices().rev() {
            let char_width = UnicodeWidthChar::width(character).unwrap_or(0);
            if suffix_width + char_width > max_width {
                break;
            }
            suffix_width += char_width;
            suffix_start = index;
        }
        let suffix = &text[suffix_start..];
        let rendered = format!("{ELLIPSIS}{suffix}");
        let before_suffix = if safe_cursor <= suffix_start {
            ""
        } else {
            &text[suffix_start..safe_cursor]
        };
        let cursor_col = 1 + UnicodeWidthStr::width(before_suffix);
        return InputWindowView {
            text: rendered,
            cursor_col: cursor_col.min(max_width.saturating_sub(1)),
        };
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

    InputWindowView {
        text: output,
        cursor_col: (1 + left_width).min(max_width.saturating_sub(1)),
    }
}

fn set_single_line_input_cursor(frame: &mut Frame<'_>, area: Rect, cursor_col: usize) {
    if area.width <= 2 || area.height <= 2 {
        return;
    }
    let content_width = area.width.saturating_sub(2);
    let x = area
        .x
        .saturating_add(1)
        .saturating_add((cursor_col as u16).min(content_width.saturating_sub(1)));
    let y = area.y.saturating_add(1);
    frame.set_cursor_position((x, y));
}

#[cfg(test)]
mod tests {
    use super::{
        FormPreviewPanelView, PreviewLineView, Symbols, TaskTagRowSegment,
        format_task_tags_for_row, input_window_view, preview_panel_lines,
        preview_panel_required_height,
    };
    use crate::config::GlyphMode;
    use crate::domain::TagColor;
    use crate::theme::{ProjectColorPalette, ThemePalette};
    use ratatui::style::Color;

    fn test_palette() -> ThemePalette {
        ThemePalette {
            text: Color::White,
            subtle_text: Color::Gray,
            border: Color::DarkGray,
            accent: Color::Cyan,
            timer_work: Color::Green,
            timer_short_break: Color::Blue,
            timer_long_break: Color::Magenta,
            success: Color::Green,
            error: Color::Red,
            priority_1: Color::Red,
            priority_2: Color::Yellow,
            priority_3: Color::Blue,
            project_colors: ProjectColorPalette {
                berry_red: Color::Rgb(178, 67, 79),
                red: Color::Red,
                orange: Color::Rgb(255, 165, 0),
                yellow: Color::Yellow,
                olive_green: Color::Rgb(128, 128, 0),
                lime_green: Color::Rgb(50, 205, 50),
                green: Color::Green,
                mint_green: Color::Rgb(152, 255, 152),
                teal: Color::Cyan,
                sky_blue: Color::Rgb(135, 206, 235),
                light_blue: Color::Rgb(173, 216, 230),
                blue: Color::Blue,
                grape: Color::Rgb(111, 45, 168),
                violet: Color::Rgb(138, 43, 226),
                lavender: Color::Rgb(230, 230, 250),
                magenta: Color::Magenta,
                salmon: Color::Rgb(250, 128, 114),
                charcoal: Color::Rgb(54, 69, 79),
                grey: Color::Gray,
                taupe: Color::Rgb(72, 60, 50),
            },
        }
    }

    #[test]
    fn input_window_view_keeps_full_text_when_it_fits() {
        let view = input_window_view("hello", 2, 10);
        assert_eq!(view.text, "hello");
        assert_eq!(view.cursor_col, 2);
    }

    #[test]
    fn input_window_view_ellipsizes_end_when_cursor_is_near_start() {
        let view = input_window_view("abcdefghijklmnop", 3, 8);
        assert_eq!(view.text, "abcdefg…");
        assert_eq!(view.cursor_col, 3);
    }

    #[test]
    fn input_window_view_ellipsizes_start_when_cursor_is_near_end() {
        let view = input_window_view("abcdefghijklmnop", 15, 8);
        assert_eq!(view.text, "…jklmnop");
        assert_eq!(view.cursor_col, 7);
    }

    #[test]
    fn input_window_view_centers_cursor_when_text_is_long_on_both_sides() {
        let view = input_window_view("abcdefghijklmnop", 8, 8);
        assert_eq!(view.text, "…fghijk…");
        assert_eq!(view.cursor_col, 4);
    }

    #[test]
    fn input_window_view_clamps_cursor_for_out_of_bounds_index() {
        let view = input_window_view("abc", 10, 5);
        assert_eq!(view.text, "abc");
        assert_eq!(view.cursor_col, 3);
    }

    #[test]
    fn format_task_tags_for_row_keeps_tag_colors_and_adds_overflow_suffix() {
        let tags = vec![
            ("deepwork", TagColor::Blue),
            ("focus", TagColor::Red),
            ("planning", TagColor::Teal),
        ];
        let rendered =
            format_task_tags_for_row(tags.as_slice(), 30, Symbols::new(GlyphMode::Ascii));
        assert_eq!(
            rendered,
            vec![
                TaskTagRowSegment {
                    text: "@deepwork".to_string(),
                    color: Some(TagColor::Blue),
                },
                TaskTagRowSegment {
                    text: "+2".to_string(),
                    color: None,
                },
            ]
        );
    }

    #[test]
    fn preview_panel_required_height_grows_with_content() {
        let compact = FormPreviewPanelView {
            preview_lines: vec![PreviewLineView::Text {
                text: "Only one line".to_string(),
                dimmed: false,
            }],
            tips: vec!["Tip".to_string()],
        };
        let expanded = FormPreviewPanelView {
            preview_lines: vec![
                PreviewLineView::KeyValue {
                    label: "Project".to_string(),
                    value: "Inbox".to_string(),
                    emphasized: false,
                    dimmed: false,
                },
                PreviewLineView::KeyValue {
                    label: "Tags".to_string(),
                    value: "@work @deep".to_string(),
                    emphasized: false,
                    dimmed: false,
                },
                PreviewLineView::KeyValue {
                    label: "Priority".to_string(),
                    value: "P2".to_string(),
                    emphasized: false,
                    dimmed: false,
                },
            ],
            tips: vec![
                "Type # for projects".to_string(),
                "Type @ for tags".to_string(),
            ],
        };

        assert!(
            preview_panel_required_height(&expanded, 3)
                > preview_panel_required_height(&compact, 3)
        );
    }

    #[test]
    fn preview_panel_lines_keep_tips_bottom_aligned_with_separator() {
        let preview = FormPreviewPanelView {
            preview_lines: vec![PreviewLineView::KeyValue {
                label: "Project".to_string(),
                value: "Inbox".to_string(),
                emphasized: false,
                dimmed: false,
            }],
            tips: vec!["Tip A".to_string(), "Tip B".to_string()],
        };
        let lines = preview_panel_lines(&preview, 30, 6, test_palette());

        assert_eq!(lines.len(), 6);
        assert_eq!(lines[0].to_string(), "Project: Inbox");
        assert_eq!(lines[1].to_string(), "");
        assert_eq!(lines[4].to_string(), "Tip A");
        assert_eq!(lines[5].to_string(), "Tip B");
    }

    #[test]
    fn anchored_form_rect_keeps_base_top_when_expanding() {
        let area = super::Rect::new(0, 0, 120, 50);
        let base = super::centered_rect(area, 72, 11);
        let expanded = super::anchored_form_rect(area, 72, 11, 16);
        assert_eq!(expanded.y, base.y);
        assert_eq!(expanded.x, base.x);
        assert_eq!(expanded.height, 16);
    }

    #[test]
    fn anchored_form_rect_shifts_up_only_when_needed_to_fit() {
        let area = super::Rect::new(0, 0, 80, 20);
        let expanded = super::anchored_form_rect(area, 72, 11, 30);
        assert_eq!(expanded.height, 18);
        assert_eq!(expanded.y, 2);
    }
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
    favorite: &'static str,
    project: &'static str,
    tag: &'static str,
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
    priority: &'static str,
    todo: &'static str,
    in_progress: &'static str,
    breaking: &'static str,
    done: &'static str,
    voided: &'static str,
    bar_full: &'static str,
    bar_empty: &'static str,
    tag_chip_left: &'static str,
    tag_chip_right: &'static str,
    tag_chip_uses_background: bool,
    ascii_mode: bool,
}

impl Symbols {
    fn new(mode: GlyphMode) -> Self {
        match mode {
            GlyphMode::Ascii => Self {
                timer: "*",
                favorite: "*",
                project: "P",
                tag: "@",
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
                priority: "!",
                todo: ".",
                in_progress: ">",
                breaking: "~",
                done: "x",
                voided: "!",
                bar_full: "=",
                bar_empty: "-",
                tag_chip_left: "[",
                tag_chip_right: "]",
                tag_chip_uses_background: false,
                ascii_mode: true,
            },
            GlyphMode::NerdFonts => Self {
                timer: "󰔛",
                favorite: "󰓎",
                project: "󰉋",
                tag: "󰓹",
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
                priority: "⚑",
                todo: "󰄱",
                in_progress: "󰧞",
                breaking: "󰒲",
                done: "󰄵",
                voided: "󰅖",
                bar_full: "█",
                bar_empty: "░",
                tag_chip_left: "",
                tag_chip_right: "",
                tag_chip_uses_background: true,
                ascii_mode: false,
            },
        }
    }
}

fn project_color_for_tag(color: TagColor) -> crate::domain::ProjectColor {
    match color {
        TagColor::BerryRed => crate::domain::ProjectColor::BerryRed,
        TagColor::Red => crate::domain::ProjectColor::Red,
        TagColor::Orange => crate::domain::ProjectColor::Orange,
        TagColor::Yellow => crate::domain::ProjectColor::Yellow,
        TagColor::OliveGreen => crate::domain::ProjectColor::OliveGreen,
        TagColor::LimeGreen => crate::domain::ProjectColor::LimeGreen,
        TagColor::Green => crate::domain::ProjectColor::Green,
        TagColor::MintGreen => crate::domain::ProjectColor::MintGreen,
        TagColor::Teal => crate::domain::ProjectColor::Teal,
        TagColor::SkyBlue => crate::domain::ProjectColor::SkyBlue,
        TagColor::LightBlue => crate::domain::ProjectColor::LightBlue,
        TagColor::Blue => crate::domain::ProjectColor::Blue,
        TagColor::Grape => crate::domain::ProjectColor::Grape,
        TagColor::Violet => crate::domain::ProjectColor::Violet,
        TagColor::Lavender => crate::domain::ProjectColor::Lavender,
        TagColor::Magenta => crate::domain::ProjectColor::Magenta,
        TagColor::Salmon => crate::domain::ProjectColor::Salmon,
        TagColor::Charcoal => crate::domain::ProjectColor::Charcoal,
        TagColor::Grey => crate::domain::ProjectColor::Grey,
        TagColor::Taupe => crate::domain::ProjectColor::Taupe,
    }
}

fn task_row_style(
    task: &Task,
    palette: ThemePalette,
    selected: bool,
    now: chrono::DateTime<Local>,
) -> Style {
    if selected {
        return Style::default()
            .fg(palette.text)
            .bg(palette.border)
            .add_modifier(Modifier::BOLD);
    }
    let base = Style::default();

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
    if selected {
        return Style::default()
            .fg(palette.text)
            .bg(palette.border)
            .add_modifier(Modifier::BOLD);
    }
    let base = Style::default();

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
    if selected {
        return Style::default()
            .fg(palette.text)
            .bg(palette.border)
            .add_modifier(Modifier::BOLD);
    }
    let base = Style::default();

    if task_is_overdue(task, now) {
        base.fg(palette.error)
    } else if task.status == TaskStatus::Done {
        base.fg(palette.subtle_text).add_modifier(Modifier::DIM)
    } else {
        base.fg(palette.timer_short_break)
    }
}

fn task_priority_indicator(priority: TaskPriority, symbols: Symbols) -> Option<String> {
    match priority {
        TaskPriority::P1 | TaskPriority::P2 | TaskPriority::P3 => {
            if symbols.ascii_mode {
                Some(priority.label().to_string())
            } else {
                Some(format!("{}{}", symbols.priority, priority.level()))
            }
        }
        TaskPriority::P4 => None,
    }
}

fn priority_color(priority: TaskPriority, palette: ThemePalette) -> Color {
    match priority {
        TaskPriority::P1 => palette.priority_1,
        TaskPriority::P2 => palette.priority_2,
        TaskPriority::P3 => palette.priority_3,
        TaskPriority::P4 => palette.text,
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
