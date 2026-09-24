# web/watchlist Specification

## Purpose
Defines the personal watchlist: a user-curated list of players the manager wants to keep an eye on, viewable as a page and modifiable via API, spanning active, free-agent and retired players.

## Requirements

### Requirement: Watchlist page lists every tracked player regardless of employment state
The system SHALL serve a watchlist page at `/{lang}/watchlist` listing every player id in the watchlist, resolving each to an active-squad player, a free agent, or a retired player, and sorted by playing position.

#### Scenario: Watchlisted player is on a team
- **WHEN** a watchlisted player currently belongs to a club's team
- **THEN** the row shows his team name/slug, league name/slug, market value, whether he is unhappy, and whether he is transfer-listed

#### Scenario: Watchlisted player has retired
- **WHEN** a watchlisted player has retired
- **THEN** the row shows a "retired" team label, zero condition, and no club/league/value data

#### Scenario: Watchlisted player was released to the free-agent pool
- **WHEN** a watchlisted player has no team and is not retired, but is present in the free-agent pool
- **THEN** the row shows a "free agent" team label rather than being silently dropped from the list

#### Scenario: Watchlisted player cannot be resolved
- **WHEN** a watchlisted player id matches no active player, free agent, or retired player record
- **THEN** the system SHALL omit that id from the rendered list rather than erroring

#### Scenario: Every row shows ability and potential ratings
- **WHEN** the watchlist page is rendered
- **THEN** each player row includes a current-ability star rating and a potential-ability star rating, the potential read via the player's own head coach when he has a team, or a market-consensus read otherwise

### Requirement: Watchlist membership can be added and removed via API
The system SHALL expose `POST /api/watchlist/add/{player_id}` and `POST /api/watchlist/remove/{player_id}` to add or remove a player id from the watchlist.

#### Scenario: Adding a player already on the list
- **WHEN** a player id already present in the watchlist is added again
- **THEN** the system SHALL leave the watchlist unchanged (no duplicate entry) and respond success

#### Scenario: Removing a player not on the list
- **WHEN** a player id not present in the watchlist is removed
- **THEN** the system SHALL respond success without altering the watchlist

#### Scenario: World not loaded
- **WHEN** an add or remove request arrives before a game world has been loaded
- **THEN** the system SHALL respond success without performing any mutation
