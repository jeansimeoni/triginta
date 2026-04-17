# NLP Locale Packs

Triginta's due/recurrence NLP uses locale-priority parsing.

## Priority rules

- Local-only mode defaults to `en` first.
- If Todoist sync provides a language hint, that locale is tried first.
- Remaining supported locales are tried in deterministic fallback order.
- Parsed tasks persist both:
  - `due_string`: original user text
  - `due_lang`: detected locale code used for deterministic upstream sync

Current built-in locales:

- `en`
- `pt-BR`
- `es`

## How to add a new locale

1. Edit [`src/task_nlp/locales.rs`](/home/jeansimeoni/Projects/triginta/src/task_nlp/locales.rs).
2. Add a new `NlpLocale` variant and `code()` value.
3. Extend `from_language_hint` so Todoist/user language hints map to the locale.
4. Add the locale to `default_locale_priority()`.
5. Add `normalize_<locale>()` rules translating locale phrases to the parser's canonical grammar.

The parser engine itself stays language-agnostic; locale packs are normalization maps.

## Test expectations for new locales

For each new locale, add coverage for:

- day phrases (`today`, `tomorrow`, `next week` equivalents)
- recurring phrases (`every day/week/month/year` equivalents)
- weekdays and common abbreviations
- relative intervals (`every 2 days`, `in 3 weeks` equivalents)
- time markers (`at 10am`, `at 14:30` equivalents)

Also include at least one ambiguity test to verify locale-priority ordering remains deterministic.
