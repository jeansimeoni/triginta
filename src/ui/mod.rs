use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Wrap},
};

use crate::{
    app::{App, RightPanelTab, ScreenData},
    domain::{PomodoroSession, Task, TaskStatus},
};

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(frame.area());

    render_header(frame, layout[0]);
    render_body(frame, app, layout[1]);
    render_status(frame, app, layout[2]);
}

fn render_header(frame: &mut Frame<'_>, area: Rect) {
    let header = Paragraph::new("Triginta  |  Pomodoro Dashboard")
        .block(Block::default().borders(Borders::ALL))
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_widget(header, area);
}

fn render_body(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    render_left_column(frame, app.screen_data(), columns[0]);
    render_right_panel(frame, app, columns[1]);
}

fn render_left_column(frame: &mut Frame<'_>, data: &ScreenData, area: Rect) {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(24),
            Constraint::Percentage(34),
            Constraint::Percentage(22),
        ])
        .split(area);

    render_timer_panel(frame, data, sections[0]);
    render_history_panel(frame, data, sections[1]);
    render_navigation_panel(frame, sections[2]);
    render_favorites_panel(frame, data, sections[3]);
}

fn render_right_panel(frame: &mut Frame<'_>, app: &App, area: Rect) {
    match app.active_right_panel_tab() {
        RightPanelTab::Tasks => render_tasks_workspace(frame, app.screen_data(), area),
        RightPanelTab::Statistics => render_statistics_panel(frame, app.screen_data(), area),
    }
}

fn render_tasks_workspace(frame: &mut Frame<'_>, data: &ScreenData, area: Rect) {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    render_task_list_panel(frame, data, sections[0]);
    render_task_details_panel(frame, data, sections[1]);
}

fn render_timer_panel(frame: &mut Frame<'_>, data: &ScreenData, area: Rect) {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(2)])
        .split(area);

    let current_task = first_active_task(data.tasks.as_slice())
        .map(|task| task.title.as_str())
        .unwrap_or("No active task");

    let info = Paragraph::new(vec![
        Line::from(vec![Span::styled(
            "Current Pomodoro",
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::from("00:00 / 25:00"),
        Line::from(format!("Task: {current_task}")),
    ])
    .block(Block::default().title("[1] Timer").borders(Borders::ALL))
    .wrap(Wrap { trim: true });

    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM))
        .gauge_style(
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
        .percent(0)
        .label("0%");

    frame.render_widget(info, sections[0]);
    frame.render_widget(gauge, sections[1]);
}

fn render_history_panel(frame: &mut Frame<'_>, data: &ScreenData, area: Rect) {
    let mut lines = vec![
        Line::from(vec![Span::styled(
            format!(
                "{} sessions  |  {} min",
                data.stats.total_sessions, data.stats.total_minutes
            ),
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
    ];

    if data.recent_sessions.is_empty() {
        lines.push(Line::from("No pomodoros recorded today."));
    } else {
        for session in data.recent_sessions.iter().take(5) {
            lines.push(Line::from(format_session_summary(session)));
        }
    }

    let history = Paragraph::new(lines)
        .block(
            Block::default()
                .title("[2] Daily History")
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: true });

    frame.render_widget(history, area);
}

fn render_navigation_panel(frame: &mut Frame<'_>, area: Rect) {
    let content = Paragraph::new(vec![
        navigation_line("> Inbox", true),
        navigation_line("  Today", false),
        navigation_line("  Soon", false),
        Line::from(""),
        Line::from("Branch-style tab switching can be wired next."),
    ])
    .block(
        Block::default()
            .title(navigation_title())
            .borders(Borders::ALL),
    )
    .wrap(Wrap { trim: true });

    frame.render_widget(content, area);
}

fn render_favorites_panel(frame: &mut Frame<'_>, data: &ScreenData, area: Rect) {
    let favorites = favorite_tasks(data.tasks.as_slice());
    let mut lines = vec![];

    if favorites.is_empty() {
        lines.push(Line::from("No favorites yet."));
        lines.push(Line::from("Pinned tasks or saved searches can live here."));
    } else {
        for task in favorites {
            lines.push(Line::from(format!("* {}", task.title)));
        }
    }

    let panel = Paragraph::new(lines)
        .block(
            Block::default()
                .title("[4] Favorites")
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: true });

    frame.render_widget(panel, area);
}

fn render_task_list_panel(frame: &mut Frame<'_>, data: &ScreenData, area: Rect) {
    let mut lines = vec![
        Line::from(vec![Span::styled(
            "All Tasks",
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
            lines.push(Line::from(format_task_summary(task)));
        }
    }

    let tasks = Paragraph::new(lines)
        .block(
            Block::default()
                .title(right_panel_title(RightPanelTab::Tasks))
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: true });

    frame.render_widget(tasks, area);
}

fn render_task_details_panel(frame: &mut Frame<'_>, data: &ScreenData, area: Rect) {
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
                .title("Task Details")
                .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM),
        )
        .wrap(Wrap { trim: true });

    frame.render_widget(details, area);
}

fn render_statistics_panel(frame: &mut Frame<'_>, data: &ScreenData, area: Rect) {
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
            "Pomodoro Statistics",
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
        .block(
            Block::default()
                .title(right_panel_title(RightPanelTab::Statistics))
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: true });

    frame.render_widget(stats, area);
}

fn render_status(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let message = format!(
        "{}  |  tab/l: next right tab  h: previous right tab  q: quit",
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

fn format_session_summary(session: &PomodoroSession) -> String {
    let task_suffix = session
        .task_id
        .map(|task_id| format!("  task #{}", task_id.0))
        .unwrap_or_default();

    format!(
        "{}  {} min{}",
        session.started_at.format("%H:%M"),
        session.duration_minutes,
        task_suffix
    )
}

fn format_task_summary(task: &Task) -> String {
    let marker = match task.status {
        TaskStatus::Todo => "[ ]",
        TaskStatus::InProgress => "[~]",
        TaskStatus::Done => "[x]",
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

fn navigation_title() -> Line<'static> {
    Line::from(vec![
        Span::raw("[3] "),
        Span::styled("Navigation", Style::default().fg(Color::Yellow)),
        Span::raw(" - "),
        Span::styled("Filters & Tags", Style::default().fg(Color::DarkGray)),
        Span::raw(" - "),
        Span::styled("Projects", Style::default().fg(Color::DarkGray)),
    ])
}

fn right_panel_title(active_tab: RightPanelTab) -> Line<'static> {
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
        Span::styled("Tasks", tasks_style),
        Span::raw(" - "),
        Span::styled("Stats", stats_style),
    ])
}
