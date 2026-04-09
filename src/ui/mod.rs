use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::{
    app::{App, CycleEntryState, PanelFocus, RightPanelTab, ScreenData, TimerPhase},
    config::GlyphMode,
    domain::{PomodoroSession, Task, TaskStatus},
};

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let symbols = Symbols::new(app.glyph_mode());
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(2)])
        .split(frame.area());

    render_body(frame, app, layout[0], symbols);
    render_status(frame, app, layout[1]);
}

fn render_body(frame: &mut Frame<'_>, app: &App, area: Rect, symbols: Symbols) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    render_left_column(frame, app, columns[0], symbols);
    render_right_panel(frame, app, columns[1], symbols);
}

fn render_left_column(frame: &mut Frame<'_>, app: &App, area: Rect, symbols: Symbols) {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(24),
            Constraint::Percentage(34),
            Constraint::Percentage(22),
        ])
        .split(area);

    render_timer_panel(frame, app, sections[0], symbols);
    render_history_panel(
        frame,
        app.screen_data(),
        sections[1],
        symbols,
        app.focused_panel(),
    );
    render_navigation_panel(frame, sections[2], symbols, app.focused_panel());
    render_favorites_panel(
        frame,
        app.screen_data(),
        sections[3],
        symbols,
        app.focused_panel(),
    );
}

fn render_right_panel(frame: &mut Frame<'_>, app: &App, area: Rect, symbols: Symbols) {
    match app.active_right_panel_tab() {
        RightPanelTab::Tasks => {
            render_tasks_workspace(frame, app.screen_data(), area, symbols, app.focused_panel())
        }
        RightPanelTab::Statistics => {
            render_statistics_panel(frame, app.screen_data(), area, symbols, app.focused_panel())
        }
    }
}

fn render_tasks_workspace(
    frame: &mut Frame<'_>,
    data: &ScreenData,
    area: Rect,
    symbols: Symbols,
    focused_panel: PanelFocus,
) {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    render_task_list_panel(frame, data, sections[0], symbols, focused_panel);
    render_task_details_panel(frame, data, sections[1], symbols);
}

fn render_timer_panel(frame: &mut Frame<'_>, app: &App, area: Rect, symbols: Symbols) {
    let timer = app.timer_view();
    let block = panel_block(
        Line::from(format!("[1] Pomodoro")),
        app.focused_panel() == PanelFocus::Timer,
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
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(content);

    let headline = Paragraph::new(vec![Line::from(vec![Span::styled(
        format!("{} {}", symbols.timer, timer.run_state.label()),
        Style::default()
            .fg(timer_color(timer.phase))
            .add_modifier(Modifier::BOLD),
    )])]);

    let progress = Paragraph::new(Line::from(progress_bar(&timer, symbols, content.width)))
        .style(Style::default().fg(timer_color(timer.phase)))
        .wrap(Wrap { trim: true });

    let progress_meta = Paragraph::new(progress_meta_line(&timer, content.width));
    let cycle = Paragraph::new(cycle_line(timer.cycle_entries.as_slice(), symbols));

    frame.render_widget(headline, sections[0]);
    frame.render_widget(progress, sections[1]);
    frame.render_widget(progress_meta, sections[2]);
    frame.render_widget(Paragraph::new(""), sections[3]);
    frame.render_widget(cycle, sections[4]);
}

fn render_history_panel(
    frame: &mut Frame<'_>,
    data: &ScreenData,
    area: Rect,
    symbols: Symbols,
    focused_panel: PanelFocus,
) {
    let mut lines = vec![
        Line::from(vec![Span::styled(
            format!(
                "{} {} sessions  |  {} min",
                symbols.focus, data.stats.total_sessions, data.stats.total_minutes
            ),
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
    ];

    if data.recent_sessions.is_empty() {
        lines.push(Line::from("No pomodoros recorded today."));
    } else {
        for session in data.recent_sessions.iter().take(5) {
            lines.push(Line::from(format_session_summary(session, symbols)));
        }
    }

    let history = Paragraph::new(lines)
        .block(panel_block(
            Line::from("[2] Daily History"),
            focused_panel == PanelFocus::History,
        ))
        .wrap(Wrap { trim: true });

    frame.render_widget(history, area);
}

fn render_navigation_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    symbols: Symbols,
    focused_panel: PanelFocus,
) {
    let content = Paragraph::new(vec![
        navigation_line(&format!("{} Inbox", symbols.selected), true),
        navigation_line(&format!("{} Today", symbols.unselected), false),
        navigation_line(&format!("{} Soon", symbols.unselected), false),
        Line::from(""),
        Line::from("Branch-style tab switching can be wired next."),
    ])
    .block(panel_block(
        navigation_title(symbols),
        focused_panel == PanelFocus::Navigation,
    ))
    .wrap(Wrap { trim: true });

    frame.render_widget(content, area);
}

fn render_favorites_panel(
    frame: &mut Frame<'_>,
    data: &ScreenData,
    area: Rect,
    symbols: Symbols,
    focused_panel: PanelFocus,
) {
    let favorites = favorite_tasks(data.tasks.as_slice());
    let mut lines = vec![];

    if favorites.is_empty() {
        lines.push(Line::from("No favorites yet."));
        lines.push(Line::from("Pinned tasks or saved searches can live here."));
    } else {
        for task in favorites {
            lines.push(Line::from(format!("{} {}", symbols.favorite, task.title)));
        }
    }

    let panel = Paragraph::new(lines)
        .block(panel_block(
            Line::from("[4] Favorites"),
            focused_panel == PanelFocus::Favorites,
        ))
        .wrap(Wrap { trim: true });

    frame.render_widget(panel, area);
}

fn render_task_list_panel(
    frame: &mut Frame<'_>,
    data: &ScreenData,
    area: Rect,
    symbols: Symbols,
    focused_panel: PanelFocus,
) {
    let mut lines = vec![
        Line::from(vec![Span::styled(
            format!("{} All Tasks", symbols.tasks),
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
    ];

    if data.tasks.is_empty() {
        lines.push(Line::from("No tasks yet."));
        lines.push(Line::from(
            "All tasks will show here when nothing is selected.",
        ));
    } else {
        for task in data.tasks.iter().take(12) {
            lines.push(Line::from(format_task_summary(task, symbols)));
        }
    }

    let tasks = Paragraph::new(lines)
        .block(panel_block(
            right_panel_title(RightPanelTab::Tasks, symbols),
            focused_panel == PanelFocus::RightPane,
        ))
        .wrap(Wrap { trim: true });

    frame.render_widget(tasks, area);
}

fn render_task_details_panel(
    frame: &mut Frame<'_>,
    data: &ScreenData,
    area: Rect,
    symbols: Symbols,
) {
    let lines = if let Some(task) = first_active_task(data.tasks.as_slice()) {
        vec![
            Line::from(vec![Span::styled(
                &task.title,
                Style::default().add_modifier(Modifier::BOLD),
            )]),
            Line::from(""),
            Line::from(format!("Status: {}", task.status.as_str())),
            Line::from(format!(
                "Created: {}",
                task.created_at.format("%Y-%m-%d %H:%M")
            )),
            Line::from(""),
            Line::from("Description, comments, labels,"),
            Line::from("and scheduling metadata will render here."),
        ]
    } else {
        vec![
            Line::from("No task selected."),
            Line::from(""),
            Line::from("Task details will fill this pane once"),
            Line::from("task selection is wired."),
        ]
    };

    let details = Paragraph::new(lines)
        .block(
            Block::default()
                .title(format!("{} Task Details", symbols.details))
                .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM),
        )
        .wrap(Wrap { trim: true });

    frame.render_widget(details, area);
}

fn render_statistics_panel(
    frame: &mut Frame<'_>,
    data: &ScreenData,
    area: Rect,
    symbols: Symbols,
    focused_panel: PanelFocus,
) {
    let completed_width = 24usize;
    let total_minutes = data.stats.total_minutes;
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
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(format!("Sessions today: {}", data.stats.total_sessions)),
        Line::from(format!("Focused minutes: {}", total_minutes)),
        Line::from(format!("Completed tasks: {}", data.stats.completed_tasks)),
        Line::from(""),
        Line::from(format!("Daily goal      {}", graph)),
        Line::from(format!("{total_minutes} / {goal_minutes} minutes")),
        Line::from(""),
        Line::from("This tab is reserved for charts, streaks,"),
        Line::from("distributions, and longer-term summaries."),
    ];

    let stats = Paragraph::new(lines)
        .block(panel_block(
            right_panel_title(RightPanelTab::Statistics, symbols),
            focused_panel == PanelFocus::RightPane,
        ))
        .wrap(Wrap { trim: true });

    frame.render_widget(stats, area);
}

fn render_status(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let message = format!(
        "{}  |  1-5: focus panel  tab: cycle focus  s/space: start  p: pause  x: void  q: quit",
        app.status_message()
    );

    let status = Paragraph::new(message)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));

    frame.render_widget(status, area);
}

fn first_active_task(tasks: &[Task]) -> Option<&Task> {
    tasks.iter().find(|task| task.status != TaskStatus::Done)
}

fn favorite_tasks(tasks: &[Task]) -> Vec<&Task> {
    tasks
        .iter()
        .filter(|task| task.status != TaskStatus::Done)
        .take(3)
        .collect()
}

fn format_session_summary(session: &PomodoroSession, symbols: Symbols) -> String {
    let task_suffix = session
        .task_id
        .map(|task_id| format!("  {} task #{}", symbols.tasks, task_id.0))
        .unwrap_or_default();

    format!(
        "{}  {} {} min{}",
        session.started_at.format("%H:%M"),
        symbols.timer,
        session.duration_minutes,
        task_suffix
    )
}

fn format_task_summary(task: &Task, symbols: Symbols) -> String {
    let marker = match task.status {
        TaskStatus::Todo => symbols.todo,
        TaskStatus::InProgress => symbols.in_progress,
        TaskStatus::Done => symbols.done,
    };

    format!("{marker} {}", task.title)
}

fn navigation_line(label: &str, selected: bool) -> Line<'static> {
    if selected {
        Line::from(vec![Span::styled(
            label.to_string(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )])
    } else {
        Line::from(label.to_string())
    }
}

fn navigation_title(symbols: Symbols) -> Line<'static> {
    Line::from(vec![
        Span::raw("[3] "),
        Span::styled(
            format!("{} Navigation", symbols.navigation),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw(" - "),
        Span::styled("Filters & Tags", Style::default().fg(Color::DarkGray)),
        Span::raw(" - "),
        Span::styled("Projects", Style::default().fg(Color::DarkGray)),
    ])
}

fn right_panel_title(active_tab: RightPanelTab, symbols: Symbols) -> Line<'static> {
    let tasks_style = if active_tab == RightPanelTab::Tasks {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let stats_style = if active_tab == RightPanelTab::Statistics {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    Line::from(vec![
        Span::raw("[5] "),
        Span::styled(format!("{} Tasks", symbols.tasks), tasks_style),
        Span::raw(" - "),
        Span::styled(format!("{} Stats", symbols.stats), stats_style),
    ])
}

fn panel_block(title: Line<'static>, focused: bool) -> Block<'static> {
    let border_style = if focused {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
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

fn timer_color(phase: TimerPhase) -> Color {
    match phase {
        TimerPhase::Focus => Color::Green,
        TimerPhase::ShortBreak => Color::LightBlue,
        TimerPhase::LongBreak => Color::Cyan,
    }
}

fn cycle_line(entries: &[CycleEntryState], symbols: Symbols) -> Line<'static> {
    let mut spans = vec![Span::styled(
        "Cycle ",
        Style::default().add_modifier(Modifier::BOLD),
    )];

    for (index, entry) in entries.iter().enumerate() {
        let symbol = match entry {
            CycleEntryState::NotStarted => symbols.todo,
            CycleEntryState::Running => symbols.in_progress,
            CycleEntryState::Break => symbols.breaking,
            CycleEntryState::Completed => symbols.done,
            CycleEntryState::Voided => symbols.voided,
        };
        spans.push(Span::raw(symbol.to_string()));
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

fn progress_meta_line(timer: &crate::app::TimerView, width: u16) -> Line<'static> {
    let percent = format!(
        "{}%",
        (timer.progress.clamp(0.0, 1.0) * 100.0).round() as u32
    );
    let remaining = format_duration(timer.remaining);
    let available = width.saturating_sub(2) as usize;
    let spaces = available.saturating_sub(percent.len() + remaining.len());
    Line::from(format!("{percent}{}{remaining}", " ".repeat(spaces)))
}

#[derive(Debug, Clone, Copy)]
struct Symbols {
    timer: &'static str,
    focus: &'static str,
    navigation: &'static str,
    favorite: &'static str,
    tasks: &'static str,
    details: &'static str,
    stats: &'static str,
    selected: &'static str,
    unselected: &'static str,
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
                focus: "*",
                navigation: ">",
                favorite: "*",
                tasks: "#",
                details: ">",
                stats: "%",
                selected: ">",
                unselected: "-",
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
                focus: "󱎫",
                navigation: "󰆍",
                favorite: "󰓎",
                tasks: "󰄱",
                details: "󰋼",
                stats: "󰕾",
                selected: "󰁔",
                unselected: "󰘍",
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
