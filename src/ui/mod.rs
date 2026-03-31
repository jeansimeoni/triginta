use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Tabs, Wrap},
};

use crate::app::{App, ScreenData};

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(frame.area());

    render_tabs(frame, app, layout[0]);
    render_body(frame, app, layout[1]);
    render_status(frame, app, layout[2]);
}

fn render_tabs(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let titles = ["Timer", "Tasks", "History"]
        .into_iter()
        .map(Line::from)
        .collect::<Vec<_>>();

    let tabs = Tabs::new(titles)
        .block(Block::default().title("Triginta").borders(Borders::ALL))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .select(app.current_screen().index());

    frame.render_widget(tabs, area);
}

fn render_body(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let body = match app.current_screen() {
        crate::app::AppScreen::Timer => render_timer(app.screen_data()),
        crate::app::AppScreen::Tasks => render_tasks(app.screen_data()),
        crate::app::AppScreen::History => render_history(app.screen_data()),
    };

    frame.render_widget(body, area);
}

fn render_timer(data: &ScreenData) -> Paragraph<'static> {
    let lines = vec![
        Line::from(vec![Span::styled(
            "Pomodoro timer",
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from("Timer engine not implemented yet."),
        Line::from("This boilerplate proves the TUI shell, event loop, and local storage setup."),
        Line::from(""),
        Line::from(format!("Tracked tasks in local DB: {}", data.tasks.len())),
        Line::from(format!(
            "Recorded pomodoros: {}",
            data.recent_sessions.len()
        )),
    ];

    Paragraph::new(lines)
        .block(Block::default().title("Timer").borders(Borders::ALL))
        .wrap(Wrap { trim: true })
}

fn render_tasks(data: &ScreenData) -> Paragraph<'static> {
    let mut lines = vec![
        Line::from(vec![Span::styled(
            "Tasks",
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
    ];

    if data.tasks.is_empty() {
        lines.push(Line::from("No tasks yet."));
        lines.push(Line::from(
            "Tasks will live locally in SQLite and remain offline-first.",
        ));
    } else {
        for task in &data.tasks {
            lines.push(Line::from(format!(
                "#{} [{}] {}",
                task.id.0,
                task.status.as_str(),
                task.title
            )));
        }
    }

    Paragraph::new(lines)
        .block(Block::default().title("Tasks").borders(Borders::ALL))
        .wrap(Wrap { trim: true })
}

fn render_history(data: &ScreenData) -> Paragraph<'static> {
    let mut lines = vec![
        Line::from(vec![Span::styled(
            "History",
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(format!("Total sessions: {}", data.stats.total_sessions)),
        Line::from(format!(
            "Total focused minutes: {}",
            data.stats.total_minutes
        )),
        Line::from(format!("Completed tasks: {}", data.stats.completed_tasks)),
        Line::from(""),
    ];

    if data.recent_sessions.is_empty() {
        lines.push(Line::from("No pomodoros recorded yet."));
    } else {
        for session in &data.recent_sessions {
            lines.push(Line::from(format!(
                "#{} {} minutes",
                session.id.0, session.duration_minutes
            )));
        }
    }

    Paragraph::new(lines)
        .block(Block::default().title("History").borders(Borders::ALL))
        .wrap(Wrap { trim: true })
}

fn render_status(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let message = format!(
        "{}  |  tab/l: next  h: previous  q: quit",
        app.status_message()
    );

    let paragraph = Paragraph::new(message)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));

    frame.render_widget(paragraph, area);
}
