# web/search Specification

## Purpose
Defines the global search page and its backing API, which lets a user look up countries, clubs, and players by name across the whole loaded world.

## Requirements

### Requirement: Search page provides a live lookup box
The system SHALL serve a search page at `/{lang}/search` containing a text input that queries the search API as the user types and renders grouped results (countries, clubs, players) without a full page reload.

#### Scenario: Opening the search page
- **WHEN** a client requests `/{lang}/search`
- **THEN** the page renders with an empty, focused search input and no visible results panel

### Requirement: Search API matches countries, clubs, and players by substring
The system SHALL serve `GET /api/search?q={text}`, matching the query case-insensitively as a substring against country names, club names, and player full names, and returning up to 15 results per category.

#### Scenario: Query shorter than four characters
- **WHEN** the query text (after trimming whitespace) is fewer than 4 characters
- **THEN** the system SHALL return empty result lists for all three categories

#### Scenario: Query matches across categories
- **WHEN** the query matches a country name, a club name, and one or more player names
- **THEN** the response includes matching countries (name, slug, code), matching clubs (name, slug of the main team), and matching players (id, slug, name, country code, team name, age, whether generated, whether a free agent)

#### Scenario: Results are capped and ranked
- **WHEN** more than 15 clubs or 15 players match the query
- **THEN** the system SHALL return only the top 15 of each, clubs ranked by team reputation and players ranked by current ability, both descending

#### Scenario: Free agents are searchable
- **WHEN** a matching player has no current club (a free agent)
- **THEN** the player result SHALL be included with an empty team name and `is_free_agent` set to true

#### Scenario: Country resolution falls back to a lookup table
- **WHEN** a matching player's `country_id` has no live `Country` record in the loaded world
- **THEN** the system SHALL fall back to a static country-info lookup for the player's country code, and use an empty code if neither source has it
