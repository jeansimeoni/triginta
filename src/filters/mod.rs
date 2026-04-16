use chrono::NaiveDate;

use crate::domain::{Task, TaskPriority, TaskStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterExpr {
    Term(FilterTerm),
    Not(Box<FilterExpr>),
    And(Box<FilterExpr>, Box<FilterExpr>),
    Or(Box<FilterExpr>, Box<FilterExpr>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterTerm {
    Tag(String),
    Project(String),
    Priority(TaskPriority),
    DueToday,
    Overdue,
    NoDue,
    StatusDone,
    StatusTodo,
    Search(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterValidationError {
    pub message: String,
}

impl FilterValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    And,
    Or,
    Not,
    LParen,
    RParen,
    Term(String),
}

pub fn parse_and_validate(query: &str) -> Result<FilterExpr, FilterValidationError> {
    let tokens = tokenize(query)?;
    if tokens.is_empty() {
        return Err(FilterValidationError::new("query cannot be empty"));
    }
    let mut parser = Parser::new(tokens);
    let expr = parser.parse_expression()?;
    if parser.has_remaining() {
        return Err(FilterValidationError::new(
            "unexpected trailing tokens in query",
        ));
    }
    Ok(expr)
}

pub fn evaluate(
    expr: &FilterExpr,
    task: &Task,
    today: NaiveDate,
    project_name: Option<&str>,
    tag_names: &[&str],
) -> bool {
    match expr {
        FilterExpr::Term(term) => evaluate_term(term, task, today, project_name, tag_names),
        FilterExpr::Not(inner) => !evaluate(inner, task, today, project_name, tag_names),
        FilterExpr::And(left, right) => {
            evaluate(left, task, today, project_name, tag_names)
                && evaluate(right, task, today, project_name, tag_names)
        }
        FilterExpr::Or(left, right) => {
            evaluate(left, task, today, project_name, tag_names)
                || evaluate(right, task, today, project_name, tag_names)
        }
    }
}

fn evaluate_term(
    term: &FilterTerm,
    task: &Task,
    today: NaiveDate,
    project_name: Option<&str>,
    tag_names: &[&str],
) -> bool {
    match term {
        FilterTerm::Tag(name) => tag_names.iter().any(|tag| tag.eq_ignore_ascii_case(name)),
        FilterTerm::Project(name) => {
            project_name.is_some_and(|project| project.eq_ignore_ascii_case(name))
        }
        FilterTerm::Priority(priority) => task.priority == *priority,
        FilterTerm::DueToday => task.due.as_ref().is_some_and(|due| due.date == today),
        FilterTerm::Overdue => task.due.as_ref().is_some_and(|due| due.date < today),
        FilterTerm::NoDue => task.due.is_none(),
        FilterTerm::StatusDone => task.status == TaskStatus::Done,
        FilterTerm::StatusTodo => task.status == TaskStatus::Todo,
        FilterTerm::Search(query) => task
            .title
            .to_lowercase()
            .contains(query.to_lowercase().as_str()),
    }
}

fn tokenize(query: &str) -> Result<Vec<Token>, FilterValidationError> {
    let mut tokens = Vec::new();
    let chars = query.chars().collect::<Vec<_>>();
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '&' => {
                tokens.push(Token::And);
                i += 1;
            }
            ',' | '|' => {
                tokens.push(Token::Or);
                i += 1;
            }
            '!' => {
                tokens.push(Token::Not);
                i += 1;
            }
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            '"' => {
                let start = i + 1;
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    i += 1;
                }
                if i >= chars.len() {
                    return Err(FilterValidationError::new("unterminated string in query"));
                }
                let value = chars[start..i].iter().collect::<String>();
                if value.trim().is_empty() {
                    return Err(FilterValidationError::new("empty quoted term in query"));
                }
                tokens.push(token_from_term(value)?);
                i += 1;
            }
            _ => {
                let start = i;
                while i < chars.len()
                    && !chars[i].is_whitespace()
                    && !matches!(chars[i], '&' | ',' | '|' | '!' | '(' | ')')
                {
                    i += 1;
                }
                let value = chars[start..i].iter().collect::<String>();
                tokens.push(token_from_term(value)?);
            }
        }
    }
    Ok(tokens)
}

fn token_from_term(value: String) -> Result<Token, FilterValidationError> {
    if value.eq_ignore_ascii_case("and") {
        return Ok(Token::And);
    }
    if value.eq_ignore_ascii_case("or") {
        return Ok(Token::Or);
    }
    if value.trim().is_empty() {
        return Err(FilterValidationError::new("empty term in query"));
    }
    Ok(Token::Term(value))
}

fn parse_term(raw: &str) -> Result<FilterTerm, FilterValidationError> {
    let normalized = raw.trim();
    if normalized.is_empty() {
        return Err(FilterValidationError::new("empty term in query"));
    }

    if let Some(tag) = normalized.strip_prefix('@') {
        if tag.trim().is_empty() {
            return Err(FilterValidationError::new("tag term cannot be empty"));
        }
        return Ok(FilterTerm::Tag(tag.trim().to_string()));
    }
    if let Some(project) = normalized.strip_prefix('#') {
        if project.trim().is_empty() {
            return Err(FilterValidationError::new("project term cannot be empty"));
        }
        return Ok(FilterTerm::Project(project.trim().to_string()));
    }

    if normalized.eq_ignore_ascii_case("today") {
        return Ok(FilterTerm::DueToday);
    }
    if normalized.eq_ignore_ascii_case("overdue") {
        return Ok(FilterTerm::Overdue);
    }
    if normalized.eq_ignore_ascii_case("no_date")
        || normalized.eq_ignore_ascii_case("no-date")
        || normalized.eq_ignore_ascii_case("nodate")
    {
        return Ok(FilterTerm::NoDue);
    }
    if normalized.eq_ignore_ascii_case("completed") {
        return Ok(FilterTerm::StatusDone);
    }
    if normalized.eq_ignore_ascii_case("active") {
        return Ok(FilterTerm::StatusTodo);
    }

    if let Some(priority) = normalized
        .to_ascii_lowercase()
        .strip_prefix('p')
        .and_then(|value| value.parse::<u8>().ok())
    {
        return match priority {
            1 => Ok(FilterTerm::Priority(TaskPriority::P1)),
            2 => Ok(FilterTerm::Priority(TaskPriority::P2)),
            3 => Ok(FilterTerm::Priority(TaskPriority::P3)),
            4 => Ok(FilterTerm::Priority(TaskPriority::P4)),
            _ => Err(FilterValidationError::new(format!(
                "unsupported priority term: {normalized}"
            ))),
        };
    }

    if let Some(value) = normalized.strip_prefix("search:") {
        if value.trim().is_empty() {
            return Err(FilterValidationError::new("search term cannot be empty"));
        }
        return Ok(FilterTerm::Search(value.trim().to_string()));
    }

    if let Some((key, value)) = normalized.split_once(':') {
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        if value.is_empty() {
            return Err(FilterValidationError::new(format!(
                "missing value for term: {normalized}"
            )));
        }
        return match key.as_str() {
            "tag" => Ok(FilterTerm::Tag(value.to_string())),
            "project" => Ok(FilterTerm::Project(value.to_string())),
            "status" => {
                if value.eq_ignore_ascii_case("done") || value.eq_ignore_ascii_case("completed") {
                    Ok(FilterTerm::StatusDone)
                } else if value.eq_ignore_ascii_case("todo") || value.eq_ignore_ascii_case("active")
                {
                    Ok(FilterTerm::StatusTodo)
                } else {
                    Err(FilterValidationError::new(format!(
                        "unsupported status value: {value}"
                    )))
                }
            }
            "due" => {
                if value.eq_ignore_ascii_case("today") {
                    Ok(FilterTerm::DueToday)
                } else if value.eq_ignore_ascii_case("overdue") {
                    Ok(FilterTerm::Overdue)
                } else if value.eq_ignore_ascii_case("none")
                    || value.eq_ignore_ascii_case("no_date")
                    || value.eq_ignore_ascii_case("no-date")
                {
                    Ok(FilterTerm::NoDue)
                } else {
                    Err(FilterValidationError::new(format!(
                        "unsupported due value: {value}"
                    )))
                }
            }
            "priority" => match value.parse::<u8>().ok() {
                Some(1) => Ok(FilterTerm::Priority(TaskPriority::P1)),
                Some(2) => Ok(FilterTerm::Priority(TaskPriority::P2)),
                Some(3) => Ok(FilterTerm::Priority(TaskPriority::P3)),
                Some(4) => Ok(FilterTerm::Priority(TaskPriority::P4)),
                _ => Err(FilterValidationError::new(format!(
                    "unsupported priority value: {value}"
                ))),
            },
            _ => Err(FilterValidationError::new(format!(
                "unsupported term: {normalized}"
            ))),
        };
    }

    Err(FilterValidationError::new(format!(
        "unsupported term: {normalized}"
    )))
}

struct Parser {
    tokens: Vec<Token>,
    index: usize,
}

#[cfg(test)]
mod tests {
    use chrono::{Local, NaiveDate};

    use crate::domain::{ProjectId, Task, TaskDue, TaskId, TaskPriority, TaskStatus};

    use super::{evaluate, parse_and_validate};

    fn sample_task(
        title: &str,
        status: TaskStatus,
        priority: TaskPriority,
        due: Option<TaskDue>,
    ) -> Task {
        Task {
            id: TaskId(1),
            project_id: ProjectId(1),
            parent_task_id: None,
            child_order: 1,
            title: title.to_string(),
            description: String::new(),
            status,
            priority,
            created_at: Local::now(),
            completed_at: None,
            deleted_at: None,
            due,
        }
    }

    #[test]
    fn parser_rejects_unsupported_terms() {
        let error = parse_and_validate("assignee:me").expect_err("term should be rejected");
        assert!(error.message.contains("unsupported"));
    }

    #[test]
    fn evaluator_supports_boolean_expressions() {
        let expr = parse_and_validate("@Work & (today | p1)").expect("query should parse");
        let today = NaiveDate::from_ymd_opt(2026, 4, 14).expect("valid date");
        let task = sample_task(
            "Write release notes",
            TaskStatus::Todo,
            TaskPriority::P4,
            Some(TaskDue {
                date: today,
                datetime: None,
                timezone: None,
                string: "today".to_string(),
                is_recurring: false,
            }),
        );
        assert!(evaluate(
            &expr,
            &task,
            today,
            Some("Inbox"),
            &["Work", "Focus"],
        ));
    }

    #[test]
    fn evaluator_applies_negation() {
        let expr = parse_and_validate("!completed & @Work").expect("query should parse");
        let today = NaiveDate::from_ymd_opt(2026, 4, 14).expect("valid date");
        let task = sample_task("Task", TaskStatus::Todo, TaskPriority::P4, None);
        assert!(evaluate(&expr, &task, today, Some("Inbox"), &["Work"]));
    }
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, index: 0 }
    }

    fn has_remaining(&self) -> bool {
        self.index < self.tokens.len()
    }

    fn parse_expression(&mut self) -> Result<FilterExpr, FilterValidationError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<FilterExpr, FilterValidationError> {
        let mut expr = self.parse_and()?;
        while self.matches_or() {
            self.index += 1;
            let right = self.parse_and()?;
            expr = FilterExpr::Or(Box::new(expr), Box::new(right));
        }
        Ok(expr)
    }

    fn parse_and(&mut self) -> Result<FilterExpr, FilterValidationError> {
        let mut expr = self.parse_unary()?;
        while self.matches_and_or_implicit() {
            if self.matches_and() {
                self.index += 1;
            }
            let right = self.parse_unary()?;
            expr = FilterExpr::And(Box::new(expr), Box::new(right));
        }
        Ok(expr)
    }

    fn parse_unary(&mut self) -> Result<FilterExpr, FilterValidationError> {
        if matches!(self.peek(), Some(Token::Not)) {
            self.index += 1;
            return Ok(FilterExpr::Not(Box::new(self.parse_unary()?)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<FilterExpr, FilterValidationError> {
        match self.peek().cloned() {
            Some(Token::Term(value)) => {
                self.index += 1;
                Ok(FilterExpr::Term(parse_term(value.as_str())?))
            }
            Some(Token::LParen) => {
                self.index += 1;
                let expr = self.parse_expression()?;
                if !matches!(self.peek(), Some(Token::RParen)) {
                    return Err(FilterValidationError::new("missing closing ')' in query"));
                }
                self.index += 1;
                Ok(expr)
            }
            Some(Token::RParen) => Err(FilterValidationError::new("unexpected ')' in query")),
            Some(Token::And) | Some(Token::Or) => {
                Err(FilterValidationError::new("operator without left term"))
            }
            Some(Token::Not) => Err(FilterValidationError::new("invalid negation in query")),
            None => Err(FilterValidationError::new("query ended unexpectedly")),
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.index)
    }

    fn matches_and(&self) -> bool {
        matches!(self.peek(), Some(Token::And))
    }

    fn matches_or(&self) -> bool {
        matches!(self.peek(), Some(Token::Or))
    }

    fn matches_and_or_implicit(&self) -> bool {
        matches!(self.peek(), Some(Token::And))
            || matches!(self.peek(), Some(Token::Term(_)))
            || matches!(self.peek(), Some(Token::Not))
            || matches!(self.peek(), Some(Token::LParen))
    }
}
