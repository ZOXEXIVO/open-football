# web/watchlist Specification

## Purpose
Defines the personal watchlist: a user-curated list of players the manager wants to keep an eye on, stored in the browser as watch keys that identify each player by id plus a fingerprint of his name and birth date, resolved and pruned against the current world by the watchlist page, and spanning active, free-agent and retired players.

## Requirements

### Requirement: Watchlist page lists every tracked player regardless of employment state
The system SHALL serve a watchlist page at `/{lang}/watchlist` that lists every player whose stored watch key resolves,
by id and fingerprint, to a player anywhere in the current world, sorted by playing position. Each row is labelled by
the player's team, as retired, or as a free agent when he has no team and has not retired.

#### Scenario: Watchlisted player is on a team
- **WHEN** a watchlisted player currently belongs to a club's team
- **THEN** the row shows his team name/slug, his league name linking to the league's country-scoped page, market value, whether he is unhappy, and whether he is transfer-listed

#### Scenario: Watchlisted player has retired
- **WHEN** a watchlisted player has retired
- **THEN** the row shows a "retired" team label, zero condition, and no club/league/value data

#### Scenario: Watchlisted player was released to the free-agent pool
- **WHEN** a watchlisted player has no team and is not retired, such as a player in the free-agent pool
- **THEN** the row shows a "free agent" team label rather than being silently dropped from the list

#### Scenario: Watchlisted player cannot be resolved
- **WHEN** a stored watch key's id matches no player in the current world, or matches one whose fingerprint differs
  from the key's
- **THEN** the system SHALL omit that key from the rendered list rather than erroring

#### Scenario: No watchlisted player resolves
- **WHEN** storage is empty or none of its keys resolve
- **THEN** the page SHALL show the "no players in watch list" message

#### Scenario: Every row shows ability and potential ratings
- **WHEN** the watchlist page is rendered
- **THEN** each player row includes a current-ability star rating and a potential-ability star rating, the potential read via the player's own head coach when he has a team, or a market-consensus read otherwise

#### Scenario: Removing a row
- **WHEN** a user removes a player from the watchlist page
- **THEN** his watch key SHALL be removed from storage and his row SHALL disappear without a full page reload

### Requirement: Watchlist membership is stored in the browser
The system SHALL keep the watchlist in the browser's local storage as an ordered list of watch keys, one list per
browser profile. The server SHALL hold no watchlist state.

#### Scenario: Two browsers keep separate lists
- **WHEN** a user adds a player to the watchlist in one browser
- **THEN** the watchlist page opened from another browser against the same server SHALL NOT list that player

#### Scenario: The list survives a new game
- **WHEN** a user starts a new game in the same browser
- **THEN** the stored watch keys SHALL remain, subject to pruning when the watchlist page is next opened

#### Scenario: Adding a player already on the list
- **WHEN** a player whose watch key is already stored is added again
- **THEN** the stored list SHALL be unchanged, with no duplicate entry

#### Scenario: Removing a player not on the list
- **WHEN** a player whose watch key is not stored is removed
- **THEN** the stored list SHALL be unchanged

#### Scenario: Storage is unavailable
- **WHEN** the browser blocks or fails local storage access
- **THEN** the watchlist SHALL behave as empty and adding or removing SHALL fail silently without breaking the page

### Requirement: A watch key identifies a player by id and identity
The system SHALL give every player a watch key of the form `{id}-{fingerprint}`, where `fingerprint` is a fixed-length
lowercase hexadecimal hash of the player's last name, first name, middle name and birth date. The hash SHALL be
identical across runs, machines and toolchain upgrades.

#### Scenario: A database player keeps his key across new games
- **WHEN** a player loaded from the shipped database appears in a new game with the same id, name and birth date
- **THEN** his watch key SHALL equal the key he had in the previous game

#### Scenario: A reused generated id gets a different key
- **WHEN** a restarted game generates a different player under an id that a previous game's player held
- **THEN** the new player's watch key SHALL differ from the previous player's key

#### Scenario: Same name with a different birth date
- **WHEN** two players share an id and a full name but differ in birth date
- **THEN** their watch keys SHALL differ

### Requirement: Opening the watchlist page prunes stale entries
Each time the watchlist page is shown, the browser SHALL send its stored watch keys to the server, and after a
successful response SHALL remove from storage every key the server did not render as a row.

#### Scenario: Id no longer exists
- **WHEN** a stored key's id matches no player anywhere in the current world
- **THEN** the key SHALL be removed from storage and SHALL NOT appear on the page

#### Scenario: Id now belongs to a different player
- **WHEN** a stored key's id resolves to a player whose fingerprint differs from the key's
- **THEN** the key SHALL be removed from storage and the page SHALL NOT show the player now holding that id

#### Scenario: Malformed key
- **WHEN** a stored entry does not parse as `{id}-{fingerprint}`
- **THEN** the entry SHALL be removed from storage

#### Scenario: Valid entries are kept in order
- **WHEN** pruning removes some keys
- **THEN** the remaining keys SHALL stay in storage in their original order

#### Scenario: The server cannot answer
- **WHEN** the rows request fails or the game world is not loaded
- **THEN** the browser SHALL leave storage unchanged

#### Scenario: The page is refreshed after processing
- **WHEN** the game finishes processing while the watchlist page is open and the page content refreshes
- **THEN** the browser SHALL resend its keys and render and prune against the updated world

### Requirement: The player page toggle reads and writes browser storage
Every player page SHALL show a watchlist toggle whose initial state reflects whether that player's watch key is
stored. Activating it SHALL add or remove that key in storage without a server request.

#### Scenario: Initial state
- **WHEN** a player page opens and the player's watch key is stored
- **THEN** the toggle SHALL show the "remove from watchlist" state

#### Scenario: Adding replaces a stale entry for the same id
- **WHEN** a player is added while storage holds another key with the same id but a different fingerprint
- **THEN** the stale key SHALL be replaced by the player's key

#### Scenario: Toggling while the game is processing
- **WHEN** the game is processing
- **THEN** the toggle SHALL remain usable
