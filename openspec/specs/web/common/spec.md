# web/common Specification

## Purpose
Defines the shared web scaffolding behaviors that every page and route relies on: static asset serving, unmatched-route handling, stylesheet bundle integrity, canonical player-page resolution, star-rating display, and season-based fixture navigation.

## Requirements

### Requirement: Unmatched requests redirect to a language-prefixed URL or 404
The system SHALL serve any request that matches no other route through a fallback handler that first attempts to serve it as a static asset, then, for a page-shaped path missing a supported language prefix, redirects permanently to the same path under the default language.

#### Scenario: Path is a known static asset
- **WHEN** the requested path resolves to an embedded asset (e.g. under `static/`)
- **THEN** the system SHALL serve that asset's bytes with an appropriate content type and cache-control header

#### Scenario: Path under static/ is missing
- **WHEN** the requested path starts with `static/` but no such embedded asset exists
- **THEN** the system SHALL respond with a plain not-found response rather than a language redirect

#### Scenario: Page path is missing its language segment
- **WHEN** the requested path's first segment is not a supported language code and the path is not empty
- **THEN** the system SHALL respond with a permanent redirect to the same path prefixed with the default language

#### Scenario: Empty path with no language prefix
- **WHEN** the requested path is empty
- **THEN** the system SHALL respond with a plain not-found response rather than a redirect

### Requirement: Compressed-only assets are transparently served
The system SHALL serve an asset that is embedded only in gzip form under its uncompressed name, returning the stored gzip bytes as-is to a client that accepts gzip, and inflating them server-side for a client that does not.

#### Scenario: Client accepts gzip
- **WHEN** a request for a gzip-only embedded asset carries `Accept-Encoding: gzip`
- **THEN** the system SHALL return the stored compressed bytes with `Content-Encoding: gzip` and the asset's real MIME type

#### Scenario: Client does not accept gzip
- **WHEN** a request for a gzip-only embedded asset has no `Accept-Encoding: gzip`
- **THEN** the system SHALL inflate the bytes and return them uncompressed with the asset's real MIME type

### Requirement: A player slug resolves to its canonical page across employment states
The system SHALL resolve a `{player_slug}` path segment to the player it names by checking, in order, players on a team, free agents, and retired players, and SHALL redirect a non-canonical slug to the canonical `{id}-{name}` form.

#### Scenario: Slug is already canonical
- **WHEN** a player-scoped page is requested with the slug exactly matching the player's canonical `{id}-{name}` form
- **THEN** the system SHALL render the page for that player directly

#### Scenario: Slug is stale or numeric-only
- **WHEN** a player-scoped page is requested with a slug that resolves to a real player but does not match his current canonical slug (e.g. a bare numeric id, or a name that has since changed)
- **THEN** the system SHALL respond with a permanent redirect to the canonical URL, preserving any sub-path such as a contract tab

#### Scenario: Slug does not resolve to any player
- **WHEN** the leading digits of the slug match no active, free-agent, or retired player
- **THEN** the system SHALL respond with a not-found error

### Requirement: Historical player references degrade gracefully to a slug
The system SHALL build a `{id}-{name}` link for a player referenced from historical data (e.g. a past transfer or loan record) by preferring the player's live canonical slug when he can still be resolved, falling back to slugifying the stored name, and finally falling back to the bare numeric id.

#### Scenario: Referenced player still resolvable
- **WHEN** a historical record's player id still resolves to a live or retired player
- **THEN** the generated slug SHALL be that player's own canonical slug

#### Scenario: Referenced player no longer indexed
- **WHEN** a historical record's player id resolves to no live or retired player, but a stored display name is available
- **THEN** the system SHALL slugify the stored name into an `{id}-{name}` link

#### Scenario: No usable stored name
- **WHEN** slugifying the stored name yields an empty string
- **THEN** the system SHALL fall back to the bare numeric id as the link, which the canonical-slug redirect resolves on click

### Requirement: Ability and potential are shown as absolute half-star ratings
The system SHALL render a player's current and potential ability as a 0–5 star rating on a half-star scale, derived from an observable 1–200 ability figure, never from the hidden underlying ability value directly.

#### Scenario: Potential floor never sits below current ability
- **WHEN** a potential-ability read (by a coach's judgement or by market consensus) computes to fewer stars than the player's current-ability rating
- **THEN** the displayed potential rating SHALL be raised to match the current-ability rating rather than showing a ceiling below the floor

#### Scenario: Potential is read through a team's own coaching staff
- **WHEN** the player belongs to a team with an assigned head coach
- **THEN** the potential rating SHALL reflect that specific coach's judgement, which can differ from the player's true ceiling and from another coach's read of the same player

#### Scenario: No employing club to judge the player
- **WHEN** the player has no employing team (a free agent or a retired player)
- **THEN** the potential rating SHALL fall back to a club-independent market-consensus read

### Requirement: Season-based fixture navigation only reaches seasons with matches
The system SHALL compute a season stepper for any fixture list whose matches have each been filed under a season (identified by the year the season opened). The stepper SHALL land on the requested season if it has matches, else the season under way when the caller supplies one and it has matches, else the most recent season that has matches. It SHALL only offer previous/next links to adjacent seasons that actually contain matches. Each stop SHALL be displayed by its season label: `YYYY/YY` when any match filed under that season belongs to a competition whose seasons cross the new year, otherwise the single year. Links SHALL address a season by a `season` query parameter carrying the season's opening year.

#### Scenario: No requested season and season in progress
- **WHEN** no season is requested, the caller supplies the season under way, and that season has matches
- **THEN** the stepper selects the season under way

#### Scenario: No season under way supplied
- **WHEN** no season is requested and the caller supplies no season under way
- **THEN** the stepper selects the most recent season that has matches

#### Scenario: Requested season has no matches
- **WHEN** the requested season is not among the seasons that have matches
- **THEN** the stepper falls back to the season under way if it has matches, else the most recent season that does

#### Scenario: Empty fixture list
- **WHEN** the fixture list has no matches in any season
- **THEN** the system SHALL produce no stepper at all, and the page SHALL show its empty state instead

#### Scenario: Adjacent seasons with no matches are skipped
- **WHEN** stepping forward or backward from the selected season
- **THEN** the next/previous link SHALL point to the nearest season that actually has matches, skipping any silent seasons in between

#### Scenario: Mixed calendars on one list
- **WHEN** a list holds season 2025 with only calendar-year competition matches, and season 2026 with matches from both a calendar-year league and an autumn-spring league
- **THEN** the stops SHALL read `2025` and `2026/27`

#### Scenario: Legacy year parameter
- **WHEN** a page is requested with the retired `year` query parameter and no `season` parameter
- **THEN** the parameter SHALL be ignored and the stepper SHALL land as if no season were requested

### Requirement: Generated stylesheet bundle integrity is guarded
The build-time-generated, minified CSS bundle SHALL preserve calc() operator spacing, media-query combinator spacing, and other minification-sensitive constructs required for the browser to parse it correctly.

#### Scenario: Minifier collapses a calc() operator's spacing
- **WHEN** the generated bundle contains a `calc()` expression whose binary `+`/`-` operator lost the whitespace on one side
- **THEN** this SHALL be treated as a defect in the generated bundle, since the declaration becomes invalid in every browser
