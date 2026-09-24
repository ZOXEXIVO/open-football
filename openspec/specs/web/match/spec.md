# web/match Specification

## Purpose
Defines the match-viewing page and its supporting streamed-replay data delivery: the scoreboard/squad page for a finished fixture and the chunked, gzip-compressed recording the in-browser 3D viewer fetches to play it back.

## Requirements

### Requirement: Match page renders the fixture scoreboard and squads
The system SHALL serve a match page at `/{lang}/match/{match_id}` showing the competition name, full-time score, both teams' goal-scorer lists (with minute and own-goal marker), player of the match, and each side's starting eleven and substitutes with position, rating, and substitution minutes.

#### Scenario: Viewing a finished league match
- **WHEN** a client requests `/{lang}/match/{match_id}` for a match that has finished with recorded details
- **THEN** the response renders the home and away team names/slugs, the score, each goal event linked to the scorer's player page, the player-of-the-match name, and the starting/bench squads for both sides

#### Scenario: International fixture resolves team identity from countries
- **WHEN** the requested match's league is `international`
- **THEN** the "team" name, slug and colours are resolved from the two countries involved rather than from club/team records, and the venue defaults to a generic national-stadium description

#### Scenario: Match not found
- **WHEN** the requested `match_id` does not exist in the global match store nor in any league's or domestic cup's match collection
- **THEN** the system SHALL respond with a not-found error

#### Scenario: Match has no recorded details or score yet
- **WHEN** a match exists but its result details or score have not been computed
- **THEN** the system SHALL respond with a not-found error rather than a partially filled page

### Requirement: Match page configures an embedded 3D replay viewer
The system SHALL emit, inline in the page, a JSON configuration describing every player, goal, chance, substitution, venue and team colour needed by the WebAssembly match viewer to replay the fixture, and SHALL only offer the replay when match recordings are enabled and the viewer build is available.

#### Scenario: Recordings enabled and viewer available
- **WHEN** match recordings are enabled for the running instance and the WebAssembly viewer was built
- **THEN** the page includes the replay stage, loading UI, and a viewer configuration document listing players (with shirt number, appearance, starting/bench flag, photo/face URLs), goal/chance/substitution timelines, and venue capacity/attendance/reputation figures

#### Scenario: Recordings enabled but viewer not built
- **WHEN** match recordings are enabled but the WebAssembly viewer was not built for this deployment
- **THEN** the page shows a message explaining the viewer is missing and how to add the required build target, instead of the replay stage

#### Scenario: Recordings disabled
- **WHEN** match recordings are disabled for the running instance
- **THEN** the page shows a message explaining recordings are off, without a viewer configuration or replay stage

### Requirement: Match metadata endpoint reports chunk layout
The system SHALL serve replay metadata at `/api/match/{match_id}/metadata`, returning the chunk count, the duration each chunk covers, the total match duration, and — for a clipped recording — the list of `[start, end]` millisecond ranges the recording actually covers.

#### Scenario: Full recording has no segment list
- **WHEN** metadata is requested for a match recorded in full (not clipped to goals/chances)
- **THEN** the response omits the `segments` field entirely, which the client is expected to read as "the whole match is covered"

#### Scenario: Clipped recording reports its covered ranges
- **WHEN** metadata is requested for a match whose recording was clipped to goal/chance windows
- **THEN** the response includes a `segments` array of `[start_ms, end_ms]` pairs naming exactly the covered stretches

#### Scenario: No recording stored for the match
- **WHEN** metadata is requested for a match that has no recording files on disk (recordings were off, or nothing was written yet)
- **THEN** the system SHALL respond with a not-found error

### Requirement: Match chunk endpoint streams gzip-compressed replay windows
The system SHALL serve one replay chunk at `/api/match/{match_id}/chunk/{chunk_number}`, returning the chunk's bytes with `Content-Type: application/gzip` and `Content-Encoding: gzip`, without re-inflating them server-side.

#### Scenario: Requesting an existing chunk
- **WHEN** a client requests a chunk number that has a file on disk for the match
- **THEN** the system SHALL return the stored gzip bytes unchanged with the gzip content headers

#### Scenario: Requesting an empty or out-of-range chunk
- **WHEN** a client requests a chunk number for which no file was written (an empty window, or a number past the recording)
- **THEN** the system SHALL respond with a not-found error naming the missing chunk and match

### Requirement: Replay recordings are written per league in five-minute chunks
When a match finishes and recordings are enabled, the system SHALL split its position/event recording into fixed-duration windows, gzip-compress each non-empty window, and write them plus a metadata document under a per-league directory keyed by match id.

#### Scenario: A goal-only clipped recording skips empty windows
- **WHEN** a match's recording only covers goal moments and long stretches of the match have no recorded data
- **THEN** only the windows containing data are written to disk, and the metadata's chunk count still reflects the full number of windows across the match so chunk indices stay aligned to match time

#### Scenario: An unrecorded match still gets metadata
- **WHEN** a match is played with recordings switched off or produces no position data
- **THEN** the system SHALL still write a metadata document (with zero chunks) so the viewer can distinguish "not recorded" from "not yet written"
