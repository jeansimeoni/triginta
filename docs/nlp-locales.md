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

## Duration and time ranges

A due phrase with a time of day can also carry a duration, matching
Todoist's own grammar:

- explicit duration: `for 30 minutes`, `por 30m`, `durante 30 minutos`
- time range: `from 8am to 8:30am`, `das 8h as 8h30`, `07:00-07:30`

Duration/range marker words (`for`/`por`/`durante`, `to`/`as`/`a`/`-`, …)
are recognized directly off whichever words are present in the text
immediately following the parsed start time, rather than through the
active locale's normalization table. This means a phrase mixing
languages — such as a Todoist-produced due string combining an English
recurrence with a Portuguese duration suffix — still parses correctly.
See `parse_duration_or_range_suffix` and `parse_duration_quantity` in
[`src/task_nlp/mod.rs`](../src/task_nlp/mod.rs).

## How to add a new locale

1. Edit [`src/task_nlp/locales.rs`](../src/task_nlp/locales.rs).
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
