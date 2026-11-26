# web/ai Specification

## Purpose
Describes the web layer's Decision connection, which users configure from the landing page and the system verifies before storing in memory. The domestic league overview uses the connection to rank completed matches for highlights.

## Requirements

### Requirement: Decision model endpoint configuration
The system SHALL let an operator save, retrieve, and clear a decision-model connection made of an endpoint
URL and an optional API key, held in memory for the life of the process. The connection SHALL NOT carry a
model name. The URL SHALL be stored as the full endpoint address, trimmed of surrounding whitespace, and not
rewritten or extended. Before storing, the system SHALL send one minimal decision request to the URL, with
the API key as a bearer token when one is given, and SHALL store the connection only when the endpoint
answers with HTTP status 200.

#### Scenario: Saving a URL without an API key
- **WHEN** a client submits a non-empty URL and an empty or absent API key, and the endpoint answers the
  probe with 200
- **THEN** the system stores the URL with an empty API key and responds with an "ok" status, and subsequent
  config reads report the connection as configured with that URL

#### Scenario: Saving a URL with an API key
- **WHEN** a client submits a non-empty URL and a non-empty API key, and the endpoint answers the probe with
  200
- **THEN** the system sends the probe with the key as a bearer token, stores both trimmed of surrounding
  whitespace, and responds with an "ok" status

#### Scenario: Saving an unreachable endpoint
- **WHEN** a client submits a URL whose endpoint cannot be connected to or does not answer within the probe
  timeout
- **THEN** the system rejects the save with an "error" status and a detail saying the endpoint is not
  reachable, and does not change any previously saved connection

#### Scenario: Saving an endpoint that answers with another status
- **WHEN** a client submits a URL whose endpoint answers the probe with any status other than 200
- **THEN** the system rejects the save with an "error" status and a detail naming the returned status, and
  does not change any previously saved connection

#### Scenario: Saving with an empty URL
- **WHEN** a client submits a save request whose URL is empty or whitespace only
- **THEN** the system rejects the save without probing, returns an "error" status with a message naming the
  URL as required, and does not change any previously saved connection

#### Scenario: Reading configuration before anything is saved
- **WHEN** a client requests the current configuration and nothing has been saved yet
- **THEN** the system reports the connection as not configured and returns the URL
  `http://localhost:8080/v1/systemone` with an empty API key as pre-fill values

#### Scenario: Reading configuration after a save
- **WHEN** a client requests the current configuration after a successful save
- **THEN** the system reports the connection as configured and returns the saved URL and API key, and the
  response carries no model name

#### Scenario: Clearing configuration
- **WHEN** a client requests that the connection be disabled
- **THEN** the system discards any saved connection so subsequent reads report it as not configured and
  return the default pre-fill values

### Requirement: Model connection settings dialog
The system SHALL offer, from the landing page's AI badge, a settings dialog with exactly two inputs: a
required URL and an optional API key. The dialog SHALL NOT show a model name input. It SHALL provide Save,
Cancel and, while a connection is saved, Disable actions, and SHALL reflect the outcome on the badge without
a page reload.

#### Scenario: Opening the dialog
- **WHEN** an operator clicks the AI badge on the landing page
- **THEN** a dialog opens with a URL input pre-filled from the saved connection (or the default URL), an API
  key input marked optional and pre-filled with the saved key (or empty), and no model input

#### Scenario: Saving from the dialog
- **WHEN** the operator enters a non-empty URL and saves
- **THEN** the dialog submits the URL and API key, shows a saved confirmation on success, switches the badge
  to ON, reveals the Disable action, and closes

#### Scenario: Saving with an empty URL from the dialog
- **WHEN** the operator clears the URL input and saves
- **THEN** the dialog does not submit the settings

#### Scenario: Disabling from the dialog
- **WHEN** a connection is saved and the operator chooses Disable
- **THEN** the dialog clears the connection, shows a disabled confirmation, switches the badge to OFF, hides
  the Disable action, and closes

#### Scenario: Save or disable fails
- **WHEN** the server rejects the request or it cannot be completed
- **THEN** the dialog shows an error message, including the server's detail when one is returned, and keeps
  the dialog open with its actions re-enabled

### Requirement: Decision features on game pages

The system SHALL limit game-page features using the saved Decision connection to completed-match highlights on the domestic league overview.

#### Scenario: Team page with a saved connection

- **WHEN** a connection is saved and a user opens a main team's squad page
- **THEN** the page SHALL render with neither an AI report button nor an AI report dialog

#### Scenario: Player page with a saved connection

- **WHEN** a connection is saved and a user opens an active or retired player's page
- **THEN** the attributes panel SHALL render without an AI report button and the page SHALL carry no AI report dialog

#### Scenario: Removed report endpoints

- **WHEN** a client calls `POST /api/ai/team-report`, `POST /api/ai/player-report` or `GET /api/ai/progress`
- **THEN** the system SHALL respond as for any unknown route

### Requirement: Decision scoring of completed league matches

When a Decision connection is saved, the system SHALL score every completed match shown together in the domestic league overview's Fixtures & Results panel. The server SHALL send the league name and a list of completed matches with a match identifier, both team names, both teams' final goals, both teams' national reputations, and whether either club lists the other as a rival. It SHALL ask one typed interest-score question per match using a shared rubric that considers derbies, major matchups, upsets, and unusual scorelines from a fan's perspective. The request SHALL use the saved endpoint URL as entered and the saved API key as a bearer token when present.

#### Scenario: A panel groups fixtures across dates

- **WHEN** a league overview shows completed matches under several date headings alongside fixtures with no result
- **THEN** the server SHALL send one Decision request containing all the completed matches across those dates, including any completed 0–0 draw, and SHALL omit fixtures without a result
- **AND** each completed match SHALL have its own score question, keyed so its answer can be matched to that result

#### Scenario: No scoring input or saved connection

- **WHEN** a league overview has no completed match in its Fixtures & Results panel, or there is no saved Decision connection
- **THEN** the system SHALL make no Decision scoring request

#### Scenario: Scoring fails

- **WHEN** the Decision request times out, returns a non-200 status, cannot be decoded, or lacks a valid score for any requested match
- **THEN** the system SHALL return no highlights for that batch and the fixtures and results SHALL remain available
