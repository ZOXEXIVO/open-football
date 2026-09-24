# web/i18n Specification

## Purpose
Captures the existing observable behavior of the web frontend's localization system: how a request's language is resolved, how page chrome, country names, happiness-event copy, and press copy are translated per scope, and the guarantees the shipped locale files uphold.

## Requirements

### Requirement: Supported language set and default
The system SHALL support exactly nine languages (English, Spanish, French, German, Portuguese, Russian, Traditional Chinese, Turkish, Japanese), each identified by a two-letter code, and SHALL treat English as the default language used whenever a request's language cannot be resolved.

#### Scenario: Unsupported language code falls back to default
- **WHEN** a caller requests translations for a language code that is not one of the nine supported codes
- **THEN** the system resolves and serves content in English instead

### Requirement: Accept-Language header resolution
The system SHALL parse a client's `Accept-Language` header, respecting per-tag quality weights, and SHALL select the highest-quality supported language found; if no supported language is present, it SHALL fall back to the default language.

#### Scenario: Multiple weighted tags with a supported preference
- **WHEN** a caller sends `Accept-Language: en;q=0.5, de;q=0.8, fr;q=0.9`
- **THEN** the system selects French because it carries the highest quality weight among the supported languages

#### Scenario: No supported tags present
- **WHEN** a caller sends an `Accept-Language` header containing only unsupported language tags
- **THEN** the system falls back to the default language

### Requirement: Scoped translation catalogs stay isolated
The system SHALL organize translated text into separate scopes — page chrome, country names, happiness-event copy, and press/newspaper copy — such that a key defined in one scope is not resolvable from another scope, except that happiness-event copy SHALL be allowed to fall back to the page-chrome scope for shared generic labels.

#### Scenario: Page chrome cannot resolve a press or event key
- **WHEN** a caller looks up a press-scope key (e.g. a news-desk label) or an event-scope key (e.g. a cause label) through the page-chrome lookup
- **THEN** the lookup returns the key itself, indicating no translation was found in that scope

#### Scenario: Happiness-event copy falls back to shared chrome labels
- **WHEN** a caller looks up a generic label (e.g. an award name) through the happiness-event lookup and that label is not present in the event bundle
- **THEN** the lookup returns the page-chrome translation of that label instead of the raw key

### Requirement: Missing translation resolves to the default language, then the key
For any scope, a lookup SHALL first check the requested language's bundle, then fall back to the default-language bundle for that same scope, and SHALL return the lookup key itself only when neither bundle has an entry for it.

#### Scenario: Key present only in the default-language bundle
- **WHEN** a caller requests a key in a non-English language and that language's bundle omits the key while the English bundle has it
- **THEN** the system returns the English text for that key

#### Scenario: Key absent from every bundle in the scope
- **WHEN** a caller requests a key that exists in no language bundle for that scope
- **THEN** the system returns the key text itself, unchanged

### Requirement: Locale-aware date formatting
The system SHALL format a given calendar date differently depending on the resolved language: a spelled-out day/month-name/year form for English, day/month numeric with `/` separators for Spanish, French and Portuguese, day/month numeric with `.` separators for German, Russian and Turkish, and a year/month/day form with Chinese-character separators for Traditional Chinese and Japanese; any other resolved language SHALL use the `.`-separated numeric form.

#### Scenario: Formatting the same date for different languages
- **WHEN** a caller formats 3 January 2026 once under English and once under French
- **THEN** the English result reads as a spelled-out "3 <localized January> 2026" and the French result reads as "03/01/2026"

### Requirement: Count-dependent noun forms follow each language's own pluralization rule
The system SHALL resolve a count-dependent phrase (such as an age in years) by selecting among the pipe-separated forms stored for that key, using each language's own counting rule: Russian SHALL use the three-way East-Slavic rule (with teen numbers always taking the third form regardless of their final digit), Traditional Chinese, Japanese and Turkish SHALL always use the first (invariant) form, and every other supported language SHALL use a one-vs-many rule. A translation value with no pipe-separated forms SHALL be returned unchanged for any count.

#### Scenario: Russian age phrase picks the correct form by count
- **WHEN** a caller resolves the "years old" phrase in Russian for the ages 21, 24, and 11
- **THEN** the system returns the first form for 21, the second form for 24, and the third form for 11 (because 11 is a teen exception)

#### Scenario: Non-plural language ignores the count
- **WHEN** a caller resolves the same phrase in a language whose value has no pipe-separated forms
- **THEN** the system returns that language's single stored value regardless of the count supplied

### Requirement: Country name resolution with English-code fallback
The system SHALL translate country codes into localized display names per language, and SHALL additionally expose the English-language spelling of a country name (independent of the currently resolved language) for callers that need the untranslated form, falling back to the raw code when no English entry exists.

#### Scenario: Looking up a country's English name regardless of active language
- **WHEN** a caller is operating under a non-English resolved language and asks for a country's English-language name
- **THEN** the system returns the English spelling from the default-language bundle, not the localized one

### Requirement: Locale bundles maintain full key parity with English
For every translation scope shipped in the localization assets, every non-English locale file SHALL contain exactly the same set of keys as the English (`en.json`) file for that scope — no missing keys and no extra keys — and every supported language (English, German, Spanish, French, Japanese, Portuguese, Russian, Turkish, Chinese) SHALL ship a bundle file for each scope (page chrome, country names, happiness events, press/news).

#### Scenario: A key added to the English chrome file must exist in every other locale
- **WHEN** a new key is added to `assets/i18n/en.json`
- **THEN** the same key MUST also be present in `assets/i18n/de.json`, `es.json`, `fr.json`, `ja.json`, `pt.json`, `ru.json`, `tr.json`, and `zh.json`, or the locale is considered out of sync with English

#### Scenario: Every supported language has a bundle file per scope
- **WHEN** the localization assets are inspected for the chrome, country, events, and news scopes
- **THEN** each of the nine supported language codes has a corresponding `.json` bundle file present in each scope's directory

### Requirement: Locale prose and placeholders must be genuinely localized
For prose-length translated values (long enough to be a phrase rather than a short code), a non-English locale SHALL NOT reuse the identical English text unless the key is explicitly exempted as a proper noun or punctuation-only template; and any `{placeholder}` tokens present in an English value SHALL appear, as the same set of names, in the corresponding translated value.

#### Scenario: A translated sentence still contains the untouched English text
- **WHEN** a non-exempt, prose-length key's non-English value is identical to its English value
- **THEN** that locale is considered to still carry untranslated English text for that key

#### Scenario: A translation drops or renames a placeholder
- **WHEN** a non-English value's set of `{...}` placeholder names differs from the English value's set for the same key
- **THEN** that locale is considered to have a broken placeholder for that key
