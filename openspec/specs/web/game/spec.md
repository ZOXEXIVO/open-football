# web/game Specification

## Purpose
Defines how a client starts a game session and advances the simulated world by a number of days, including progress polling and mid-run cancellation.

## Requirements

### Requirement: Game creation endpoint acknowledges session start
The system SHALL expose `GET /api/game/create`, responding with success to signal that a client may begin interacting with the game session.

#### Scenario: Requesting game creation
- **WHEN** a client requests `/api/game/create`
- **THEN** the system SHALL respond with a success status

### Requirement: Processing advances the world by a requested number of days
The system SHALL expose `POST /api/game/process?days={n}`, simulating `n` daily ticks (default 1) against the currently loaded world and publishing the updated world for subsequent page reads once complete.

#### Scenario: A single day is processed
- **WHEN** a process request is made with no `days` parameter
- **THEN** the system SHALL simulate exactly one day and respond with success once it is applied

#### Scenario: A multi-day run publishes intermediate progress
- **WHEN** a process request asks for more than seven days
- **THEN** the system SHALL publish an updated world snapshot at least once every seven simulated days, so readers observe intermediate state rather than only the final result

#### Scenario: A processing request arrives while another is already running
- **WHEN** a process request is received while a previous process request has not yet finished
- **THEN** the system SHALL respond immediately with success without starting a second concurrent run

#### Scenario: Finished matches are recorded when recordings are enabled
- **WHEN** a simulated day produces finished matches and match recordings are enabled
- **THEN** the system SHALL write each match's replay artifacts before the process request completes

### Requirement: Processing status can be polled
The system SHALL expose `GET /api/game/processing`, reporting whether a process run is currently in progress.

#### Scenario: No run in progress
- **WHEN** the processing status is requested and no process run is active
- **THEN** the response SHALL report `processing: false`

#### Scenario: A run is in progress
- **WHEN** the processing status is requested while a multi-day process run is under way
- **THEN** the response SHALL report `processing: true`

### Requirement: An in-progress run can be cancelled
The system SHALL expose `POST /api/game/cancel`, signalling an active process run to stop before completing its requested number of days.

#### Scenario: Cancelling a multi-day run
- **WHEN** a cancel request is received while a multi-day process run is under way
- **THEN** the running simulation SHALL stop advancing further days at the next opportunity and still publish the world state reached so far
