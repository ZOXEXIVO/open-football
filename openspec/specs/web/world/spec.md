# web/world Specification

## Purpose
Defines how the web application serves the simulated world to concurrent page and API requests while processing runs and editor actions change it. Reads never stall behind a writer, and no change is ever lost or shown before it is complete.

## Requirements

### Requirement: Each request reads one consistent world
The system SHALL serve every request that reads the world from one published world, fixed when the request first reads it. That world SHALL stay unchanged for the request's whole duration, including while the request waits on disk, the network or other shared state.

#### Scenario: A publication lands mid-request
- **WHEN** a request has started reading the world and processing publishes a newer world before the request finishes
- **THEN** the request SHALL finish against the world it started with, and a request that starts after the publication SHALL read the newer world

#### Scenario: A request waits on a replay file
- **WHEN** a match chunk or metadata request is still waiting for a replay file to be read
- **THEN** the match it resolves SHALL come from the world the request started with, however long the read takes

#### Scenario: No world is loaded yet
- **WHEN** a request reads the world before any world has been published
- **THEN** it SHALL observe that no world is loaded, and the endpoint SHALL respond as its own specification defines for that case

### Requirement: Reading the world never waits for anything else
Starting to read the world SHALL NOT wait for a processing run, a publication, an editor action, freeing a superseded world, or any other request, however long any of them takes. Publishing a world SHALL NOT wait for requests that are still reading an earlier one.

#### Scenario: A slow request is in flight when processing publishes
- **WHEN** one request is still rendering from the published world and processing publishes a newer one
- **THEN** the publication SHALL complete without waiting for that request
- **AND** requests that start afterwards SHALL be served from the newer world, without waiting for the slow request to finish

#### Scenario: An editor action is being applied
- **WHEN** an editor action is being applied to the world
- **THEN** requests that read the world SHALL be served from the latest published world without waiting for the action

#### Scenario: Processing is simulating a day
- **WHEN** a request arrives while a processing run is simulating a day
- **THEN** it SHALL be served from the latest published world without waiting for the day to finish

### Requirement: Changes to the world are applied one at a time, in order
The system SHALL apply every change to the world, from processing runs and editor actions alike, one at a time and in the order the changes were requested. Each change SHALL act on the world the previous change produced. No two changes ever act on the world at once, and none acts on an outdated copy of it.

#### Scenario: Two editor actions arrive together
- **WHEN** two editor actions are submitted at the same moment
- **THEN** the system SHALL apply them one after the other, the second against the world the first produced, and the published world SHALL contain the effects of both

#### Scenario: An editor action fails partway
- **WHEN** an editor action fails while it is being applied
- **THEN** the published world SHALL be left exactly as it was before the action, the action SHALL be answered with a server error, and later changes SHALL still be applied

### Requirement: An editor action taken during processing survives the run
The system SHALL apply an editor action submitted while a processing run is in progress to the world being processed. The action SHALL take effect no later than the start of the next simulated day, and the run's following days SHALL be simulated from the edited world. No publication by the run SHALL discard it.

#### Scenario: Clearing an injury during a holiday
- **WHEN** an editor clears a player's injury while a multi-day run is in progress
- **THEN** the run SHALL simulate the following days from a world in which the player is uninjured
- **AND** every world the run publishes afterwards SHALL contain the edit, unless a later simulated event changes the same state

#### Scenario: The run is cancelled with an edit waiting
- **WHEN** a processing run is cancelled while an editor action is waiting to be applied to it
- **THEN** the action SHALL still be applied, and the world published when the run stops SHALL contain it

### Requirement: An editor action is answered once its effect is visible
The system SHALL answer an editor action only after a world containing the action's outcome has been published. Any request that starts after the answer SHALL observe the outcome, whether or not a processing run is in progress.

#### Scenario: Reloading after an edit while idle
- **WHEN** an editor toggles a player's forced match selection while no processing run is in progress, and the client reloads the player page after the response
- **THEN** the reloaded page SHALL show the toggled state

#### Scenario: Reloading after an edit during processing
- **WHEN** an editor action is answered while a multi-day run is in progress
- **THEN** a page requested after the answer SHALL show the action's outcome, without waiting for the run's next weekly publication

#### Scenario: An edit published mid-run shows no unwritten replays
- **WHEN** a world is published to make an editor action visible during a run with match recordings enabled
- **THEN** every finished match in that world SHALL already have its replay written, as for any other publication of the run

### Requirement: A superseded world is freed without delaying any request
The system SHALL free a world that is no longer published once no request is reading it. Freeing it SHALL NOT run inside, or delay, any request, including the last request to finish reading it. The world SHALL be freed even if nothing further is published.

#### Scenario: The last reader of a superseded world finishes
- **WHEN** a request that started before a publication finishes after it, as the last reader of the superseded world
- **THEN** its response SHALL NOT be delayed by freeing that world, and the world SHALL be freed shortly after the request finishes

#### Scenario: Memory held while processing
- **WHEN** a processing run is in progress
- **THEN** the system SHALL hold no world besides the published one, the one being processed, and superseded worlds that in-flight requests are still reading
