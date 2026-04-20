// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jean Simeoni

use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NlpLocale {
    En,
    PtBr,
    Es,
}

impl NlpLocale {
    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::PtBr => "pt-BR",
            Self::Es => "es",
        }
    }

    pub fn from_language_hint(hint: &str) -> Option<Self> {
        let normalized = fold_diacritics(hint.trim().to_lowercase().as_str());
        if normalized.is_empty() {
            return None;
        }
        if normalized == "en" || normalized.starts_with("en-") || normalized == "english" {
            return Some(Self::En);
        }
        if normalized == "pt"
            || normalized == "pt-br"
            || normalized == "pt_br"
            || normalized.starts_with("pt-")
            || normalized.starts_with("pt_")
            || normalized.contains("portuguese")
            || normalized.contains("portugues")
        {
            return Some(Self::PtBr);
        }
        if normalized == "es"
            || normalized.starts_with("es-")
            || normalized.starts_with("es_")
            || normalized.contains("spanish")
            || normalized.contains("espanol")
            || normalized.contains("espanhol")
        {
            return Some(Self::Es);
        }
        None
    }
}

pub fn default_locale_priority() -> Vec<NlpLocale> {
    vec![NlpLocale::En, NlpLocale::PtBr, NlpLocale::Es]
}

pub fn locale_priority_with_hint(language_hint: Option<&str>) -> Vec<NlpLocale> {
    let mut locales = Vec::new();
    if let Some(hint) = language_hint
        && let Some(locale) = NlpLocale::from_language_hint(hint)
    {
        locales.push(locale);
    }
    locales.push(NlpLocale::En);
    locales.push(NlpLocale::PtBr);
    locales.push(NlpLocale::Es);

    let mut seen = HashSet::new();
    locales
        .into_iter()
        .filter(|locale| seen.insert(*locale))
        .collect()
}

pub fn normalize_due_input_for_locale(input: &str, locale: NlpLocale) -> String {
    let folded = fold_diacritics(input.to_lowercase().as_str());
    let compact = collapse_whitespace(folded.as_str());
    let compact = match locale {
        NlpLocale::En => compact,
        NlpLocale::PtBr => normalize_pt_br(compact.as_str()),
        NlpLocale::Es => normalize_es(compact.as_str()),
    };
    collapse_whitespace(compact.as_str())
}

fn normalize_pt_br(input: &str) -> String {
    let mut text = input.to_string();
    text = normalize_pt_br_weekly_weekday_phrases(text.as_str());
    let replacements = [
        ("ultimo dia util do mes", "last weekday every month"),
        ("primeiro dia util do mes", "first weekday every month"),
        ("ultimo dia util todo mes", "last weekday every month"),
        ("primeiro dia util todo mes", "first weekday every month"),
        ("ultimo dia do mes", "last day every month"),
        ("primeiro dia do mes", "first day every month"),
        ("no ultimo dia util do mes", "last weekday every month"),
        ("no primeiro dia util do mes", "first weekday every month"),
        ("no ultimo dia do mes", "last day every month"),
        ("no primeiro dia do mes", "first day every month"),
        ("todo mes no ultimo dia util", "last weekday every month"),
        ("todo mes no primeiro dia util", "first weekday every month"),
        ("todo mes no ultimo dia", "last day every month"),
        ("todo mes no primeiro dia", "first day every month"),
        ("todo mes no quinto dia util", "fifth weekday every month"),
        ("todo quinto dia util", "fifth weekday every month"),
        ("todo mes no quarto dia util", "fourth weekday every month"),
        ("todo mes no terceiro dia util", "third weekday every month"),
        ("todo mes no segundo dia util", "second weekday every month"),
        ("quinto dia util todo mes", "fifth weekday every month"),
        ("quarto dia util todo mes", "fourth weekday every month"),
        ("terceiro dia util todo mes", "third weekday every month"),
        ("segundo dia util todo mes", "second weekday every month"),
        ("primeiro dia util todo mes", "first weekday every month"),
        ("quinto", "fifth"),
        ("quarto", "fourth"),
        ("terceiro", "third"),
        ("segundo", "second"),
        ("primeiro", "first"),
        ("ultimo", "last"),
        ("dias uteis", "weekdays"),
        ("todos os dias uteis", "every weekday"),
        ("todo dia util", "every weekday"),
        ("dia util", "weekday"),
        ("todo o dia", "every day"),
        ("fim de semana", "weekend"),
        ("todos os dias", "every day"),
        ("todo dia", "every day"),
        ("diariamente", "daily"),
        ("todas as semanas", "every week"),
        ("toda semana", "every week"),
        ("semanalmente", "weekly"),
        ("todos os meses", "every month"),
        ("todo mes", "every month"),
        ("mensalmente", "monthly"),
        ("todos os anos", "every year"),
        ("todo ano", "every year"),
        ("anualmente", "yearly"),
        ("proxima semana", "next week"),
        ("proximo mes", "next month"),
        ("depois de amanha", "in 2 days"),
        ("amanha", "tomorrow"),
        ("hoje", "today"),
        ("segunda-feira", "monday"),
        ("terca-feira", "tuesday"),
        ("quarta-feira", "wednesday"),
        ("quinta-feira", "thursday"),
        ("sexta-feira", "friday"),
        ("sabado", "saturday"),
        ("domingo", "sunday"),
        ("segunda", "monday"),
        ("terca", "tuesday"),
        ("quarta", "wednesday"),
        ("quinta", "thursday"),
        ("sexta", "friday"),
        ("sab", "sat"),
        ("dom", "sun"),
        ("a cada", "every"),
        ("cada", "every"),
        ("dias", "days"),
        ("semanas", "weeks"),
        ("meses", "months"),
        ("anos", "years"),
        ("dia", "day"),
        ("semana", "week"),
        ("mes", "month"),
        ("ano", "year"),
    ];
    for (needle, replacement) in replacements {
        text = replace_word_bounded(text.as_str(), needle, replacement);
    }
    text = replace_time_markers(text.as_str(), &["as", "a"]);
    text
}

fn normalize_pt_br_weekly_weekday_phrases(input: &str) -> String {
    let mut text = input.to_string();
    let weekday_aliases = [
        ("segunda-feira", "monday"),
        ("terca-feira", "tuesday"),
        ("quarta-feira", "wednesday"),
        ("quinta-feira", "thursday"),
        ("sexta-feira", "friday"),
        ("sabado", "saturday"),
        ("domingo", "sunday"),
        ("segunda", "monday"),
        ("terca", "tuesday"),
        ("quarta", "wednesday"),
        ("quinta", "thursday"),
        ("sexta", "friday"),
    ];
    let weekly_prefixes = ["toda a ", "toda ", "semanalmente na ", "semanalmente no "];
    for (pt_weekday, en_weekday) in weekday_aliases {
        for prefix in weekly_prefixes {
            let needle = format!("{prefix}{pt_weekday}");
            let replacement = format!("every {en_weekday}");
            text = replace_word_bounded(text.as_str(), needle.as_str(), replacement.as_str());
        }
    }
    let weekly_suffixes = [" toda a semana", " toda semana", " semanalmente"];
    for (pt_weekday, en_weekday) in weekday_aliases {
        for suffix in weekly_suffixes {
            let needle = format!("{pt_weekday}{suffix}");
            let replacement = format!("every {en_weekday}");
            text = replace_word_bounded(text.as_str(), needle.as_str(), replacement.as_str());
        }
    }
    text
}

fn normalize_es(input: &str) -> String {
    let mut text = input.to_string();
    let replacements = [
        ("todos los dias habiles", "every weekday"),
        ("fin de semana", "weekend"),
        ("todos los dias", "every day"),
        ("cada dia", "every day"),
        ("diariamente", "daily"),
        ("todas las semanas", "every week"),
        ("cada semana", "every week"),
        ("semanalmente", "weekly"),
        ("todos los meses", "every month"),
        ("cada mes", "every month"),
        ("mensualmente", "monthly"),
        ("todos los anos", "every year"),
        ("cada ano", "every year"),
        ("anualmente", "yearly"),
        ("proxima semana", "next week"),
        ("proximo mes", "next month"),
        ("pasado manana", "in 2 days"),
        ("manana", "tomorrow"),
        ("hoy", "today"),
        ("lunes", "monday"),
        ("martes", "tuesday"),
        ("miercoles", "wednesday"),
        ("jueves", "thursday"),
        ("viernes", "friday"),
        ("sabado", "saturday"),
        ("domingo", "sunday"),
        ("cada", "every"),
        ("dias", "days"),
        ("semanas", "weeks"),
        ("meses", "months"),
        ("anos", "years"),
        ("dia", "day"),
        ("semana", "week"),
        ("mes", "month"),
        ("ano", "year"),
    ];
    for (needle, replacement) in replacements {
        text = replace_word_bounded(text.as_str(), needle, replacement);
    }
    text = replace_time_markers(text.as_str(), &["a las", "a la", "a"]);
    text
}

fn replace_time_markers(input: &str, markers: &[&str]) -> String {
    let mut text = input.to_string();
    for marker in markers {
        let prefix = format!("{marker} ");
        text = replace_word_bounded(text.as_str(), prefix.as_str(), "at ");
    }
    text
}

fn collapse_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn replace_word_bounded(input: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return input.to_string();
    }

    let mut result = String::new();
    let mut cursor = 0;
    while let Some(found) = input[cursor..].find(needle) {
        let start = cursor + found;
        let end = start + needle.len();

        if !is_boundary(input, start, end) {
            result.push_str(&input[cursor..end]);
            cursor = end;
            continue;
        }

        result.push_str(&input[cursor..start]);
        result.push_str(replacement);
        cursor = end;
    }
    result.push_str(&input[cursor..]);
    result
}

fn is_boundary(input: &str, start: usize, end: usize) -> bool {
    let left_ok = input[..start]
        .chars()
        .next_back()
        .is_none_or(|c| !c.is_alphanumeric());
    let right_ok = input[end..]
        .chars()
        .next()
        .is_none_or(|c| !c.is_alphanumeric());
    left_ok && right_ok
}

fn fold_diacritics(input: &str) -> String {
    input
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            _ => c,
        })
        .collect()
}
