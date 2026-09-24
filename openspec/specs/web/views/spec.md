# web/views Specification

## Purpose
Defines the shared left-hand navigation menu scaffolding used across the web app's pages, including which sections appear on each page type, how the active item is determined, and how supporting page data (neighbor team lists, league display names, country lookups) is prepared for those views.

## Requirements

### Requirement: Common menu sections on every page
Every left-hand navigation menu the system builds SHALL begin with a "Home" entry and a "Search" entry, and SHALL end with a "Watchlist" entry (except the About page menu, which ends with an "About" entry instead of Watchlist).

#### Scenario: Any competition page menu
- **WHEN** the system builds the left menu for a league, cup, playoff, country, or continental-competition page
- **THEN** the resulting section list starts with a Home section and a Search section

#### Scenario: About page menu
- **WHEN** the system builds the left menu for the About page
- **THEN** the resulting sections are Home, Search, then About, with no Watchlist section

### Requirement: Active item highlighting by current path
Each navigation item SHALL be marked active when the current page path exactly matches its target URL, or, for country/league/cup/playoff/team links, when the current path starts with the item's URL followed by a path separator (so sub-pages of that link also show it as active).

#### Scenario: Exact path match
- **WHEN** the current path equals a menu item's URL (e.g. the watchlist link while viewing the watchlist page)
- **THEN** that menu item is marked active

#### Scenario: Sub-page of a league link
- **WHEN** the current path is a sub-page under a league's URL (e.g. `/en/leagues/premier-league/table` under `/en/leagues/premier-league`)
- **THEN** the league's menu item is still marked active

#### Scenario: Unrelated path
- **WHEN** the current path does not match a menu item's URL and is not a sub-path of it
- **THEN** that menu item is not marked active

### Requirement: League list collapses beyond two entries
A section listing a country's leagues (or a club's league list) SHALL be marked collapsible when it contains more than two items, showing only the first two by default, unless one of the items beyond the first two is the currently active item, in which case the section is expanded by default.

#### Scenario: Three or more leagues, none active beyond the first two
- **WHEN** a country has more than two leagues and the active league (if any) is within the first two
- **THEN** the league section is collapsible and starts collapsed

#### Scenario: Active league beyond the first two
- **WHEN** a country has more than two leagues and the currently active league is the third or later in the list
- **THEN** the league section is collapsible and starts expanded

#### Scenario: Two or fewer leagues
- **WHEN** a country has two or fewer leagues
- **THEN** the league section is not collapsible and all items are shown

### Requirement: Cup and playoff links stay outside the collapsible league list
The domestic cup entry and the playoffs entries SHALL each appear in their own standalone menu section, separate from the (possibly collapsed) league-pyramid section, so they remain visible regardless of the league section's collapsed state. A country with no playoffs SHALL produce no playoffs section.

#### Scenario: Country with a cup and a collapsed league list
- **WHEN** the system builds a menu for a country that has more than two leagues and a domestic cup
- **THEN** the cup link appears in its own section beneath the league section and is not hidden by the league section's collapse toggle

#### Scenario: Country with no playoffs
- **WHEN** the country being viewed runs no grouped-competition playoffs
- **THEN** the resulting menu contains no playoffs section

### Requirement: Continental competition section scoped to viewer's continent
A continental club-competitions section (Champions League, Europa League, Conference League) SHALL be included for countries in Europe, a Copa Libertadores section SHALL be included for countries in South America, and no continental section SHALL be included for countries on any other continent.

#### Scenario: European country page
- **WHEN** the system builds the country, cup, or playoff menu for a country belonging to the European continent
- **THEN** the menu includes a section with Champions League, Europa League, and Conference League links

#### Scenario: South American country page
- **WHEN** the system builds the country, cup, or playoff menu for a country belonging to the South American continent
- **THEN** the menu includes a section with a single Copa Libertadores link

#### Scenario: Country on another continent
- **WHEN** the system builds the country, cup, or playoff menu for a country outside Europe and South America
- **THEN** the menu includes no continental-cup section

### Requirement: National team level switch on the country landing sections
The shared home-and-country section builder SHALL always produce, for a given country, a senior national team entry and a U21 national team entry, each linking to its own page and marked active only when the current path matches that specific team's page.

#### Scenario: Viewing the senior national team page
- **WHEN** the current path is the country's senior team URL
- **THEN** the senior team menu item is active and the U21 item is not

#### Scenario: Viewing the U21 national team page
- **WHEN** the current path is the country's U21 team URL
- **THEN** the U21 team menu item is active and the senior item is not

### Requirement: Club neighbor-team list ordering
The list of a club's other teams (for the team page's neighbor-team section) SHALL be sorted first by team-type menu order (Main first, then Second, B, Reserve, and the U23-down-to-U18 age teams), and within the same team type, by descending world reputation.

#### Scenario: Club with a first team and a reserve team
- **WHEN** the system builds the neighbor-teams list for a club that has a Main team and a Reserve team
- **THEN** the Main team entry appears before the Reserve team entry regardless of reputation

#### Scenario: Two teams of the same type
- **WHEN** a club has two teams of the same team type with different world reputation values
- **THEN** the team with the higher world reputation appears first

### Requirement: League display name includes country adjective
The display name shown for a league SHALL be the league's own name prefixed with the localized country adjective for the league's country, when that adjective is available; otherwise the league's own name is used unprefixed.

#### Scenario: League with a known country
- **WHEN** the system formats a league's display name and the league's country has a localized adjective (e.g. "English")
- **THEN** the displayed name is the adjective followed by the league's name (e.g. "English Premier League")

#### Scenario: League with no resolvable country
- **WHEN** the system formats a league's display name and no country adjective can be resolved for it
- **THEN** the displayed name is exactly the league's own name, with no prefix
