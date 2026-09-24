# web/ai Specification

## Purpose
Captures the existing behavior of the web layer's AI assistant integration: configuring an OpenAI-compatible LLM endpoint, running an agentic tool-calling loop against a snapshot of the simulated football world, and streaming its progress to the client via long-polling.

## Requirements

### Requirement: LLM endpoint configuration
The system SHALL let an operator save, retrieve, and clear an OpenAI-compatible LLM connection contract (base URL, model name, optional API key) held in memory for the life of the process.

#### Scenario: Saving valid settings
- **WHEN** a client submits a base URL and model name (with or without an API key)
- **THEN** the system stores the settings in memory and responds with an "ok" status, and subsequent config reads report the connection as configured

#### Scenario: Saving with a missing required field
- **WHEN** a client submits a save request with an empty base URL or model
- **THEN** the system rejects the save, returns an "error" status with a descriptive message, and does not change any previously saved configuration

#### Scenario: Reading configuration before anything is saved
- **WHEN** a client requests the current AI configuration and nothing has been saved yet
- **THEN** the system reports the connection as not configured and returns a set of pre-filled default connection values

#### Scenario: Clearing configuration
- **WHEN** a client requests that the AI configuration be disabled
- **THEN** the system discards any saved settings so subsequent reads report the connection as not configured

### Requirement: Agentic report generation over world data
The system SHALL run an autonomous loop that sends a system prompt and task to the configured LLM, executes any tools the model requests against a snapshot of the simulated world, and feeds each tool's result back to the model until it returns a final text report or a fixed step limit is reached.

#### Scenario: Model completes without further tool calls
- **WHEN** a chat response from the LLM contains no tool calls
- **THEN** the loop ends and the final assistant text is recorded as the completed result

#### Scenario: Model requests one or more tools
- **WHEN** a chat response includes tool calls
- **THEN** each requested tool is executed against the world snapshot, its call is recorded for live progress reporting, and its JSON result is appended to the conversation before the next round is sent to the model

#### Scenario: Step limit exceeded
- **WHEN** the model has not produced a final answer after the maximum number of allowed rounds
- **THEN** the run is marked as failed with a message indicating it did not finish within the step limit

#### Scenario: LLM request fails
- **WHEN** the HTTP call to the LLM endpoint errors, returns a non-success status, or returns a response that cannot be parsed
- **THEN** the run is marked as failed with a short human-readable reason, and no further rounds are attempted

### Requirement: World-data lookup tools
The system SHALL expose a fixed set of read-only tools the model can call during a run, each returning a JSON string describing club or player data, or a JSON error object if the request cannot be satisfied.

#### Scenario: Looking up a club by id
- **WHEN** the model calls the club lookup tool with a numeric club id that exists
- **THEN** the result includes the club's identity, philosophy, location, colors, status, finance, facilities, academy, rivals, and a summary of each of its teams (id, name, type, slug, league, player count, reputation)

#### Scenario: Looking up a club's squad
- **WHEN** the model calls the squad lookup tool with a numeric club id that exists
- **THEN** the result lists each of the club's teams with its players' id, name, age, position, current ability, and potential ability, omitting detailed skill breakdowns

#### Scenario: Looking up a player by id
- **WHEN** the model calls the player lookup tool with a numeric player id that exists
- **THEN** the result includes the player's identity, age, birth date, nationality, current team (if any), positions, preferred foot, current and potential ability, and full attribute/skill/personality detail

#### Scenario: Lookup target does not exist
- **WHEN** the model calls a lookup tool with an id that has no matching club or player
- **THEN** the tool returns a JSON object describing the error instead of raising a failure that aborts the run

#### Scenario: Unknown tool or malformed arguments
- **WHEN** the model calls a tool name the system does not recognize, or omits/malforms a required id argument
- **THEN** the tool dispatch returns a JSON error object describing the problem rather than crashing the run

### Requirement: Live progress streaming for agent runs
The system SHALL let a client observe an in-progress agent run's tool activity and final outcome by long-polling with a job id and a cursor position.

#### Scenario: Client polls a running job with new activity
- **WHEN** a client polls a known job id with a cursor behind the number of tool calls made so far
- **THEN** the response resolves promptly with the job's current status, the tool calls made since the given cursor, and an updated cursor

#### Scenario: Client polls a running job with no new activity
- **WHEN** a client polls a known, still-running job whose tool-call count has not advanced past the given cursor
- **THEN** the request is held open for a bounded interval before returning the current snapshot, so the client can poll again without busy-looping

#### Scenario: Job completes
- **WHEN** an agent run finishes successfully
- **THEN** a poll against that job id reports a "done" status together with the final report text

#### Scenario: Job fails
- **WHEN** an agent run fails (LLM error or step-limit exceeded)
- **THEN** a poll against that job id reports an "error" status together with a failure detail message

#### Scenario: Polling an unknown job
- **WHEN** a client polls a job id that does not exist in the registry
- **THEN** the system responds indicating the job is unknown rather than returning a snapshot
