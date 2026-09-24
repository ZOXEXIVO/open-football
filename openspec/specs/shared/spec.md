# shared Specification

## Purpose
Defines the contract of the logic shared between the server (core/web) and the wasm match-viewer targets, guaranteeing that a player's visual appearance is computed identically wherever it is drawn.

## Requirements

### Requirement: A player's appearance is a pure, deterministic function of his identity
A player's rendered appearance (skin tone, hair color, eye color, and phenotype classification) SHALL depend only on his player id and his nationality's population-distribution record. It MUST NOT depend on wall-clock time, process state, or any other source of non-determinism.

#### Scenario: The same player renders identically on every request
- **WHEN** a player's appearance is computed twice, in the same process or across different processes, given the same player id and the same nationality distribution
- **THEN** the resulting skin, hair, and eye selections SHALL be identical both times

### Requirement: Portrait rendering and match-viewer rendering agree on every player's appearance
The two independent call sites that need a player's appearance — the profile-portrait renderer and the match-replay viewer — SHALL resolve to the same skin, hair, and eye result for a given player, whether each computes it independently from the player id or shares an already-initialized random stream.

#### Scenario: An independently-seeded draw matches a draw made from a shared, already-advanced stream
- **WHEN** a player's appearance is computed once by seeding fresh from his id, and once by drawing from a stream the portrait generator had already been using for other purposes, with appearance drawn as the first thing off that stream
- **THEN** both computations SHALL produce the same skin, hair, and eye indices

### Requirement: A country's ancestry mix determines the population-level distribution of appearances for its players
A country's population is described by three ancestry-share percentages (white/black/metis) plus a geographic region. The combination of a rolled ancestry bucket and the country's region SHALL resolve to one coherent phenotype classification per player, and that classification SHALL bound which skin tones, hair colors, and other features are eligible for him.

#### Scenario: Players from a population of one dominant ancestry stay within that ancestry's tone range
- **WHEN** many players are generated for a country whose population distribution is (near) entirely one ancestry bucket
- **THEN** all of their resolved skin tones SHALL fall within that ancestry/region's defined tone band

#### Scenario: A genuinely mixed country's players show more than one tone
- **WHEN** many players are generated for a country whose population distribution spans multiple ancestry buckets in meaningful proportion
- **THEN** the resulting set of skin tones across those players SHALL include more than a single tone

### Requirement: Every phenotype's feature tables index only colors that exist
For every defined phenotype classification, its skin-tone band, hair-color weight table, and eye-color weight table SHALL only ever reference indices that exist in the shared color palettes, so a lookup can never go out of bounds.

#### Scenario: No phenotype's tables reference a palette index beyond its bounds
- **WHEN** any phenotype's skin band, hair table, or eye table is inspected
- **THEN** every index referenced SHALL be within the bounds of the corresponding shared palette

### Requirement: A country code resolves to a region regardless of letter case, with a safe universal fallback for unknown codes
Region lookup by two-letter country code SHALL be case-insensitive. A code that matches no known mapping SHALL resolve to a defined default region rather than failing, so appearance generation never errors on an unrecognized nationality.

#### Scenario: An unrecognized or empty country code still produces a valid appearance
- **WHEN** a player's nationality code does not match any entry in the region mapping (including an empty string)
- **THEN** region resolution SHALL fall back to a fixed default region, and appearance generation SHALL still complete successfully

### Requirement: Appearance data crosses the server/viewer boundary as compact indices, not raw color values
A computed appearance SHALL be represented as small integer indices into the shared color palettes (skin/hair/eyes) rather than as literal color values, so the same palette definitions govern rendering on both the server-rendered portrait and the wasm-viewer pitch figure.

#### Scenario: Changing a palette's color values changes both renderers together
- **WHEN** a color palette entry is updated
- **THEN** both the portrait renderer and the match viewer SHALL reflect the updated color for any player whose resolved index points at that palette entry, since both read the same palette definitions
