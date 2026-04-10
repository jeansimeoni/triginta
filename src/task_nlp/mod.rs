use chrono::{Datelike, Days, Months, NaiveDate, NaiveDateTime, NaiveTime, Weekday};

use crate::domain::TaskDue;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTaskInput {
    pub cleaned_title: String,
    pub due: Option<TaskDue>,
}

pub fn parse_task_input(input: &str, reference_date: NaiveDate) -> ParsedTaskInput {
    let trimmed = input.trim();
    let Some((span, due)) = extract_due(trimmed, reference_date) else {
        return ParsedTaskInput {
            cleaned_title: trimmed.to_string(),
            due: None,
        };
    };

    let cleaned_title = remove_due_span(trimmed, span);
    ParsedTaskInput {
        cleaned_title,
        due: Some(due),
    }
}

pub fn parse_due_input(input: &str, reference_date: NaiveDate) -> Option<TaskDue> {
    parse_task_input(format!("Placeholder {input}").as_str(), reference_date).due
}

pub fn parse_due_time_input(input: &str) -> Option<NaiveTime> {
    parse_time_token(input.trim())
}

fn extract_due(input: &str, reference_date: NaiveDate) -> Option<((usize, usize), TaskDue)> {
    let lower = input.to_ascii_lowercase();
    let mut best: Option<((usize, usize), TaskDue)> = None;

    for candidate in phrase_candidates(reference_date) {
        update_best_match(input, lower.as_str(), &candidate, &mut best);
    }

    for candidate in relative_candidates(input, reference_date, lower.as_str()) {
        if is_better_match(best.as_ref(), &candidate) {
            best = Some(candidate);
        }
    }

    for candidate in iso_date_candidates(input) {
        if is_better_match(best.as_ref(), &candidate) {
            best = Some(candidate);
        }
    }

    for candidate in month_day_candidates(input, reference_date) {
        if is_better_match(best.as_ref(), &candidate) {
            best = Some(candidate);
        }
    }

    best
}

fn update_best_match(
    input: &str,
    lower: &str,
    candidate: &(String, TaskDue),
    best: &mut Option<((usize, usize), TaskDue)>,
) {
    let phrase = candidate.0.as_str();
    let mut search_from = 0;
    while let Some(relative_start) = lower[search_from..].find(phrase) {
        let start = search_from + relative_start;
        let end = start + phrase.len();
        if !is_phrase_boundary(input, start, end) {
            search_from = end;
            continue;
        }

        let matched = with_time_suffix(
            input,
            start,
            end,
            TaskDue {
                date: candidate.1.date,
                datetime: None,
                string: input[start..end].trim().to_string(),
                is_recurring: candidate.1.is_recurring,
            },
        );
        if is_better_match(best.as_ref(), &matched) {
            *best = Some(matched);
        }

        search_from = end;
    }
}

fn phrase_candidates(reference_date: NaiveDate) -> Vec<(String, TaskDue)> {
    let mut candidates = vec![
        simple_due("today", reference_date, false),
        simple_due(
            "tomorrow",
            reference_date
                .checked_add_days(Days::new(1))
                .unwrap_or(reference_date),
            false,
        ),
        simple_due(
            "next week",
            reference_date
                .checked_add_days(Days::new(7))
                .unwrap_or(reference_date),
            false,
        ),
        simple_due(
            "next month",
            reference_date
                .checked_add_months(Months::new(1))
                .unwrap_or(reference_date),
            false,
        ),
        simple_due("every day", reference_date, true),
        simple_due("daily", reference_date, true),
        simple_due("every week", reference_date, true),
        simple_due("weekly", reference_date, true),
        simple_due("every month", reference_date, true),
        simple_due("monthly", reference_date, true),
        simple_due("every year", reference_date, true),
        simple_due("yearly", reference_date, true),
        simple_due(
            "every weekday",
            next_weekday_or_same(reference_date, Weekday::Mon),
            true,
        ),
        simple_due(
            "every weekend",
            next_weekday_or_same(reference_date, Weekday::Sat),
            true,
        ),
    ];

    for (name, weekday) in weekday_candidates() {
        let next = next_weekday(reference_date, weekday);
        let next_or_same = next_weekday_or_same(reference_date, weekday);

        candidates.push(simple_due(name, next, false));
        candidates.push(simple_due(format!("next {name}"), next, false));
        candidates.push(simple_due(format!("every {name}"), next_or_same, true));
    }

    candidates
}

fn relative_candidates(
    input: &str,
    reference_date: NaiveDate,
    lower: &str,
) -> Vec<((usize, usize), TaskDue)> {
    let mut matches = Vec::new();
    let words = words_with_positions(lower);
    let mut index = 0;

    while index < words.len() {
        if words[index].0 == "in"
            && index + 2 < words.len()
            && let Ok(amount) = words[index + 1].0.parse::<u64>()
            && let Some(date) = relative_date(reference_date, amount, words[index + 2].0)
        {
            matches.push(with_time_suffix(
                input,
                words[index].1,
                words[index + 2].2,
                TaskDue {
                    date,
                    datetime: None,
                    string: input[words[index].1..words[index + 2].2].trim().to_string(),
                    is_recurring: false,
                },
            ));
            index += 3;
            continue;
        }

        if words[index].0 == "every"
            && index + 1 < words.len()
            && let Ok(amount) = words[index + 1].0.parse::<u64>()
            && index + 2 < words.len()
            && let Some(date) = recurring_relative_date(reference_date, amount, words[index + 2].0)
        {
            matches.push(with_time_suffix(
                input,
                words[index].1,
                words[index + 2].2,
                TaskDue {
                    date,
                    datetime: None,
                    string: input[words[index].1..words[index + 2].2].trim().to_string(),
                    is_recurring: true,
                },
            ));
            index += 3;
            continue;
        }

        index += 1;
    }

    matches
}

fn iso_date_candidates(input: &str) -> Vec<((usize, usize), TaskDue)> {
    let mut matches = Vec::new();
    let mut index = 0;

    while index + 10 <= input.len() {
        let candidate = &input[index..index + 10];
        if looks_like_iso_date(candidate) && is_phrase_boundary(input, index, index + 10) {
            if let Ok(date) = NaiveDate::parse_from_str(candidate, "%Y-%m-%d") {
                matches.push(with_time_suffix(
                    input,
                    index,
                    index + 10,
                    TaskDue {
                        date,
                        datetime: None,
                        string: candidate.to_string(),
                        is_recurring: false,
                    },
                ));
            }
        }
        index += 1;
    }

    matches
}

fn month_day_candidates(input: &str, reference_date: NaiveDate) -> Vec<((usize, usize), TaskDue)> {
    let lower = input.to_ascii_lowercase();
    let words = words_with_positions(lower.as_str());
    let mut matches = Vec::new();

    for index in 0..words.len().saturating_sub(1) {
        let month = parse_month(words[index].0);
        let day = words[index + 1].0.parse::<u32>().ok();
        let (Some(month), Some(day)) = (month, day) else {
            continue;
        };

        let start = words[index].1;
        let end = words[index + 1].2;
        if !is_phrase_boundary(input, start, end) {
            continue;
        }

        let date = month_day_date(reference_date, month, day);
        let Some(date) = date else {
            continue;
        };

        matches.push(with_time_suffix(
            input,
            start,
            end,
            TaskDue {
                date,
                datetime: None,
                string: input[start..end].trim().to_string(),
                is_recurring: false,
            },
        ));
    }

    matches
}

fn simple_due(phrase: impl Into<String>, date: NaiveDate, is_recurring: bool) -> (String, TaskDue) {
    let string = phrase.into();
    (
        string.clone(),
        TaskDue {
            date,
            datetime: None,
            string,
            is_recurring,
        },
    )
}

fn with_time_suffix(
    input: &str,
    start: usize,
    end: usize,
    mut due: TaskDue,
) -> ((usize, usize), TaskDue) {
    if let Some((time_end, time)) = parse_time_suffix(input, end) {
        due.datetime = Some(NaiveDateTime::new(due.date, time));
        due.string = input[start..time_end].trim().to_string();
        (extend_removable_span(input, start, time_end), due)
    } else {
        (extend_removable_span(input, start, end), due)
    }
}

fn parse_time_suffix(input: &str, start: usize) -> Option<(usize, NaiveTime)> {
    let slice = &input[start..];
    let trimmed = slice.trim_start();
    if trimmed.is_empty() {
        return None;
    }

    let leading_ws = slice.len() - trimmed.len();
    let mut offset = start + leading_ws;
    let lower = trimmed.to_ascii_lowercase();
    let mut candidate = lower.as_str();

    if let Some(rest) = candidate.strip_prefix("at ") {
        offset += 3;
        candidate = rest;
    }

    let token_end = candidate
        .char_indices()
        .find(|(_, character)| character.is_whitespace())
        .map(|(index, _)| index)
        .unwrap_or(candidate.len());
    let token = &candidate[..token_end];
    let time = parse_time_token(token)?;
    Some((offset + token_end, time))
}

fn parse_time_token(token: &str) -> Option<NaiveTime> {
    let token = token
        .trim_end_matches(|character: char| !character.is_ascii_alphanumeric() && character != ':');
    if token.is_empty() {
        return None;
    }

    match token {
        "noon" => return NaiveTime::from_hms_opt(12, 0, 0),
        "midnight" => return NaiveTime::from_hms_opt(0, 0, 0),
        _ => {}
    }

    if let Some(value) = token.strip_suffix("am") {
        return parse_meridiem_time(value, false);
    }
    if let Some(value) = token.strip_suffix("pm") {
        return parse_meridiem_time(value, true);
    }

    if let Some((hour, minute)) = token.split_once(':') {
        let hour = hour.parse::<u32>().ok()?;
        let minute = minute.parse::<u32>().ok()?;
        return NaiveTime::from_hms_opt(hour, minute, 0);
    }

    None
}

fn parse_meridiem_time(value: &str, is_pm: bool) -> Option<NaiveTime> {
    let (hour, minute) = if let Some((hour, minute)) = value.split_once(':') {
        (hour.parse::<u32>().ok()?, minute.parse::<u32>().ok()?)
    } else {
        (value.parse::<u32>().ok()?, 0)
    };

    if hour == 0 || hour > 12 || minute > 59 {
        return None;
    }

    let normalized_hour = match (hour, is_pm) {
        (12, false) => 0,
        (12, true) => 12,
        (_, true) => hour + 12,
        (_, false) => hour,
    };

    NaiveTime::from_hms_opt(normalized_hour, minute, 0)
}

fn weekday_candidates() -> [(&'static str, Weekday); 17] {
    [
        ("monday", Weekday::Mon),
        ("mon", Weekday::Mon),
        ("tuesday", Weekday::Tue),
        ("tue", Weekday::Tue),
        ("tues", Weekday::Tue),
        ("wednesday", Weekday::Wed),
        ("wed", Weekday::Wed),
        ("thursday", Weekday::Thu),
        ("thu", Weekday::Thu),
        ("thur", Weekday::Thu),
        ("thurs", Weekday::Thu),
        ("friday", Weekday::Fri),
        ("fri", Weekday::Fri),
        ("saturday", Weekday::Sat),
        ("sat", Weekday::Sat),
        ("sunday", Weekday::Sun),
        ("sun", Weekday::Sun),
    ]
}

fn words_with_positions(input: &str) -> Vec<(&str, usize, usize)> {
    let mut words = Vec::new();
    let mut current_start = None;

    for (index, character) in input.char_indices() {
        if character.is_ascii_alphanumeric() {
            if current_start.is_none() {
                current_start = Some(index);
            }
        } else if let Some(start) = current_start.take() {
            words.push((&input[start..index], start, index));
        }
    }

    if let Some(start) = current_start {
        words.push((&input[start..], start, input.len()));
    }

    words
}

fn relative_date(reference_date: NaiveDate, amount: u64, unit: &str) -> Option<NaiveDate> {
    match singular_unit(unit) {
        "day" => reference_date.checked_add_days(Days::new(amount)),
        "week" => reference_date.checked_add_days(Days::new(amount.saturating_mul(7))),
        "month" => reference_date.checked_add_months(Months::new(amount as u32)),
        "year" => {
            reference_date.checked_add_months(Months::new((amount as u32).saturating_mul(12)))
        }
        _ => None,
    }
}

fn recurring_relative_date(
    reference_date: NaiveDate,
    amount: u64,
    unit: &str,
) -> Option<NaiveDate> {
    match singular_unit(unit) {
        "day" | "week" | "month" | "year" => relative_date(reference_date, amount, unit),
        _ => None,
    }
}

fn singular_unit(unit: &str) -> &str {
    match unit {
        "days" => "day",
        "weeks" => "week",
        "months" => "month",
        "years" => "year",
        _ => unit,
    }
}

fn parse_month(word: &str) -> Option<u32> {
    match word {
        "jan" | "january" => Some(1),
        "feb" | "february" => Some(2),
        "mar" | "march" => Some(3),
        "apr" | "april" => Some(4),
        "may" => Some(5),
        "jun" | "june" => Some(6),
        "jul" | "july" => Some(7),
        "aug" | "august" => Some(8),
        "sep" | "sept" | "september" => Some(9),
        "oct" | "october" => Some(10),
        "nov" | "november" => Some(11),
        "dec" | "december" => Some(12),
        _ => None,
    }
}

fn month_day_date(reference_date: NaiveDate, month: u32, day: u32) -> Option<NaiveDate> {
    let this_year = NaiveDate::from_ymd_opt(reference_date.year(), month, day)?;
    if this_year >= reference_date {
        return Some(this_year);
    }
    NaiveDate::from_ymd_opt(reference_date.year() + 1, month, day)
}

fn next_weekday(reference_date: NaiveDate, weekday: Weekday) -> NaiveDate {
    let current = reference_date.weekday().num_days_from_monday() as i64;
    let target = weekday.num_days_from_monday() as i64;
    let mut days = (target - current).rem_euclid(7);
    if days == 0 {
        days = 7;
    }
    reference_date
        .checked_add_days(Days::new(days as u64))
        .unwrap_or(reference_date)
}

fn next_weekday_or_same(reference_date: NaiveDate, weekday: Weekday) -> NaiveDate {
    let current = reference_date.weekday().num_days_from_monday() as i64;
    let target = weekday.num_days_from_monday() as i64;
    let days = (target - current).rem_euclid(7);
    reference_date
        .checked_add_days(Days::new(days as u64))
        .unwrap_or(reference_date)
}

fn looks_like_iso_date(candidate: &str) -> bool {
    candidate.len() == 10
        && candidate.as_bytes()[4] == b'-'
        && candidate.as_bytes()[7] == b'-'
        && candidate
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}

fn is_phrase_boundary(input: &str, start: usize, end: usize) -> bool {
    let left_ok = start == 0
        || input[..start]
            .chars()
            .last()
            .map(|character| !character.is_ascii_alphanumeric())
            .unwrap_or(true);
    let right_ok = end >= input.len()
        || input[end..]
            .chars()
            .next()
            .map(|character| !character.is_ascii_alphanumeric())
            .unwrap_or(true);
    left_ok && right_ok
}

fn extend_removable_span(input: &str, start: usize, end: usize) -> (usize, usize) {
    let mut connector_end = start;
    while connector_end > 0 {
        let previous = input[..connector_end]
            .chars()
            .last()
            .expect("connector_end > 0 implies char exists");
        if previous.is_whitespace() {
            connector_end -= previous.len_utf8();
        } else {
            break;
        }
    }

    if connector_end == start {
        return (start, end);
    }

    let mut connector_start = connector_end;
    while connector_start > 0 {
        let previous = input[..connector_start]
            .chars()
            .last()
            .expect("connector_start > 0 implies char exists");
        if previous.is_ascii_alphabetic() {
            connector_start -= previous.len_utf8();
        } else {
            break;
        }
    }

    if connector_start == connector_end {
        return (start, end);
    }

    let connector = input[connector_start..connector_end].to_ascii_lowercase();
    if matches!(connector.as_str(), "by" | "on" | "for" | "due") {
        (connector_start, end)
    } else {
        (start, end)
    }
}

fn is_better_match(
    current: Option<&((usize, usize), TaskDue)>,
    candidate: &((usize, usize), TaskDue),
) -> bool {
    let Some(current) = current else {
        return true;
    };
    let current_span = current.0;
    let candidate_span = candidate.0;
    candidate_span.1 > current_span.1
        || (candidate_span.1 == current_span.1 && candidate_span.0 < current_span.0)
        || (candidate_span.1 == current_span.1
            && candidate_span.0 == current_span.0
            && (candidate_span.1 - candidate_span.0) > (current_span.1 - current_span.0))
}

fn remove_due_span(input: &str, span: (usize, usize)) -> String {
    let mut left = input[..span.0].trim_end().to_string();
    let right = input[span.1..].trim_start();

    if !left.is_empty() && !right.is_empty() {
        left.push(' ');
    }
    left.push_str(right);
    left.trim().to_string()
}

#[cfg(test)]
mod tests {
    use chrono::{Days, NaiveDate};

    use crate::domain::TaskDue;

    use super::parse_task_input;

    #[test]
    fn parses_tomorrow_due_phrase_and_cleans_title() {
        let parsed = parse_task_input(
            "Ship report tomorrow",
            NaiveDate::from_ymd_opt(2026, 4, 9).expect("valid date"),
        );

        assert_eq!(parsed.cleaned_title, "Ship report");
        assert_eq!(
            parsed.due.expect("due should parse").date,
            NaiveDate::from_ymd_opt(2026, 4, 10).expect("valid date")
        );
    }

    #[test]
    fn parses_relative_days_phrase() {
        let parsed = parse_task_input(
            "Renew certificate in 3 days",
            NaiveDate::from_ymd_opt(2026, 4, 9).expect("valid date"),
        );

        assert_eq!(parsed.cleaned_title, "Renew certificate");
        assert_eq!(
            parsed.due,
            Some(TaskDue {
                date: NaiveDate::from_ymd_opt(2026, 4, 12).expect("valid date"),
                datetime: None,
                string: "in 3 days".to_string(),
                is_recurring: false,
            })
        );
    }

    #[test]
    fn parses_recurring_weekday_phrase() {
        let reference_date = NaiveDate::from_ymd_opt(2026, 4, 9).expect("valid date");
        let parsed = parse_task_input("Team sync every monday", reference_date);

        assert_eq!(parsed.cleaned_title, "Team sync");
        assert_eq!(
            parsed.due,
            Some(TaskDue {
                date: NaiveDate::from_ymd_opt(2026, 4, 13).expect("valid date"),
                datetime: None,
                string: "every monday".to_string(),
                is_recurring: true,
            })
        );
    }

    #[test]
    fn recurring_weekday_can_resolve_to_same_day() {
        let reference_date = NaiveDate::from_ymd_opt(2026, 4, 13).expect("valid date");
        let parsed = parse_task_input("Review every monday", reference_date);

        assert_eq!(
            parsed.due,
            Some(TaskDue {
                date: reference_date,
                datetime: None,
                string: "every monday".to_string(),
                is_recurring: true,
            })
        );
    }

    #[test]
    fn parses_month_day_without_year() {
        let parsed = parse_task_input(
            "Tax prep apr 10",
            NaiveDate::from_ymd_opt(2026, 4, 9).expect("valid date"),
        );

        assert_eq!(parsed.cleaned_title, "Tax prep");
        assert_eq!(
            parsed.due,
            Some(TaskDue {
                date: NaiveDate::from_ymd_opt(2026, 4, 10).expect("valid date"),
                datetime: None,
                string: "apr 10".to_string(),
                is_recurring: false,
            })
        );
    }

    #[test]
    fn month_day_rolls_into_next_year_if_needed() {
        let parsed = parse_task_input(
            "Tax prep apr 10",
            NaiveDate::from_ymd_opt(2026, 4, 11).expect("valid date"),
        );

        assert_eq!(
            parsed.due.expect("due should parse").date,
            NaiveDate::from_ymd_opt(2027, 4, 10).expect("valid date")
        );
    }

    #[test]
    fn rightmost_due_phrase_wins() {
        let reference_date = NaiveDate::from_ymd_opt(2026, 4, 9).expect("valid date");
        let parsed = parse_task_input("Draft tomorrow then review monday", reference_date);

        assert_eq!(parsed.cleaned_title, "Draft tomorrow then review");
        assert_eq!(
            parsed.due.expect("due should parse").date,
            NaiveDate::from_ymd_opt(2026, 4, 13).expect("valid date")
        );
    }

    #[test]
    fn parses_every_day_as_recurring_today() {
        let reference_date = NaiveDate::from_ymd_opt(2026, 4, 9).expect("valid date");
        let parsed = parse_task_input("Journal every day", reference_date);

        assert_eq!(
            parsed.due,
            Some(TaskDue {
                date: reference_date,
                datetime: None,
                string: "every day".to_string(),
                is_recurring: true,
            })
        );
    }

    #[test]
    fn keeps_title_when_no_due_phrase_is_found() {
        let parsed = parse_task_input(
            "Ship report",
            NaiveDate::from_ymd_opt(2026, 4, 9).expect("valid date"),
        );

        assert_eq!(parsed.cleaned_title, "Ship report");
        assert!(parsed.due.is_none());
    }

    #[test]
    fn strips_by_connector_from_title() {
        let reference_date = NaiveDate::from_ymd_opt(2026, 4, 9).expect("valid date");
        let parsed = parse_task_input("Submit report by tomorrow", reference_date);

        assert_eq!(parsed.cleaned_title, "Submit report");
        assert_eq!(
            parsed.due,
            Some(TaskDue {
                date: NaiveDate::from_ymd_opt(2026, 4, 10).expect("valid date"),
                datetime: None,
                string: "tomorrow".to_string(),
                is_recurring: false,
            })
        );
    }

    #[test]
    fn strips_on_connector_from_title() {
        let reference_date = NaiveDate::from_ymd_opt(2026, 4, 9).expect("valid date");
        let parsed = parse_task_input("Check in on friday", reference_date);

        assert_eq!(parsed.cleaned_title, "Check in");
        assert_eq!(
            parsed.due.expect("due should parse").date,
            NaiveDate::from_ymd_opt(2026, 4, 10).expect("valid date")
        );
    }

    #[test]
    fn strips_due_connector_from_title() {
        let reference_date = NaiveDate::from_ymd_opt(2026, 4, 9).expect("valid date");
        let parsed = parse_task_input("Invoice client due next week", reference_date);

        assert_eq!(parsed.cleaned_title, "Invoice client");
        assert_eq!(
            parsed.due,
            Some(TaskDue {
                date: NaiveDate::from_ymd_opt(2026, 4, 16).expect("valid date"),
                datetime: None,
                string: "next week".to_string(),
                is_recurring: false,
            })
        );
    }

    #[test]
    fn parses_due_time_with_meridiem() {
        let reference_date = NaiveDate::from_ymd_opt(2026, 4, 9).expect("valid date");
        let parsed = parse_task_input("Submit report tomorrow at 3pm", reference_date);

        assert_eq!(parsed.cleaned_title, "Submit report");
        assert_eq!(
            parsed.due,
            Some(TaskDue {
                date: NaiveDate::from_ymd_opt(2026, 4, 10).expect("valid date"),
                datetime: Some(
                    NaiveDate::from_ymd_opt(2026, 4, 10)
                        .expect("valid date")
                        .and_hms_opt(15, 0, 0)
                        .expect("valid time"),
                ),
                string: "tomorrow at 3pm".to_string(),
                is_recurring: false,
            })
        );
    }

    #[test]
    fn parses_due_time_with_24_hour_clock() {
        let reference_date = NaiveDate::from_ymd_opt(2026, 4, 9).expect("valid date");
        let parsed = parse_task_input("Check in fri 14:00", reference_date);

        assert_eq!(parsed.cleaned_title, "Check in");
        assert_eq!(
            parsed.due,
            Some(TaskDue {
                date: NaiveDate::from_ymd_opt(2026, 4, 10).expect("valid date"),
                datetime: Some(
                    NaiveDate::from_ymd_opt(2026, 4, 10)
                        .expect("valid date")
                        .and_hms_opt(14, 0, 0)
                        .expect("valid time"),
                ),
                string: "fri 14:00".to_string(),
                is_recurring: false,
            })
        );
    }

    #[test]
    fn parses_recurring_due_time() {
        let reference_date = NaiveDate::from_ymd_opt(2026, 4, 9).expect("valid date");
        let parsed = parse_task_input("Journal every day at 9am", reference_date);

        assert_eq!(parsed.cleaned_title, "Journal");
        assert_eq!(
            parsed.due,
            Some(TaskDue {
                date: reference_date,
                datetime: Some(reference_date.and_hms_opt(9, 0, 0).expect("valid time"),),
                string: "every day at 9am".to_string(),
                is_recurring: true,
            })
        );
    }

    #[test]
    fn parses_due_time_at_noon() {
        let reference_date = NaiveDate::from_ymd_opt(2026, 4, 9).expect("valid date");
        let parsed = parse_task_input("Submit report tomorrow at noon", reference_date);

        assert_eq!(parsed.cleaned_title, "Submit report");
        assert_eq!(
            parsed.due,
            Some(TaskDue {
                date: NaiveDate::from_ymd_opt(2026, 4, 10).expect("valid date"),
                datetime: Some(
                    NaiveDate::from_ymd_opt(2026, 4, 10)
                        .expect("valid date")
                        .and_hms_opt(12, 0, 0)
                        .expect("valid time"),
                ),
                string: "tomorrow at noon".to_string(),
                is_recurring: false,
            })
        );
    }

    #[test]
    fn parses_due_time_at_midnight() {
        let reference_date = NaiveDate::from_ymd_opt(2026, 4, 9).expect("valid date");
        let parsed = parse_task_input("Reset counters tomorrow at midnight", reference_date);

        assert_eq!(parsed.cleaned_title, "Reset counters");
        assert_eq!(
            parsed.due,
            Some(TaskDue {
                date: NaiveDate::from_ymd_opt(2026, 4, 10).expect("valid date"),
                datetime: Some(
                    NaiveDate::from_ymd_opt(2026, 4, 10)
                        .expect("valid date")
                        .and_hms_opt(0, 0, 0)
                        .expect("valid time"),
                ),
                string: "tomorrow at midnight".to_string(),
                is_recurring: false,
            })
        );
    }

    #[test]
    fn supports_weekday_abbreviations() {
        let reference_date = NaiveDate::from_ymd_opt(2026, 4, 9).expect("valid date");
        let parsed = parse_task_input("Check in fri", reference_date);

        assert_eq!(
            parsed.due.expect("due should parse").date,
            reference_date + Days::new(1)
        );
    }
}
