# web/workers Specification

## Purpose
Describes the operator-facing Workers monitoring page, which lets a user view the health and throughput of the local match-simulation pool and any remote match workers, and add or remove remote workers.

## Requirements

### Requirement: Workers page summary tiles
The workers page SHALL display summary tiles for the count of ready workers out of total workers, total thread count across all workers, total batches sent, total matches completed, and total failures.

#### Scenario: Operator opens the workers page
- **WHEN** a user navigates to the workers page for a given language
- **THEN** the page shows tiles for "ready/total match workers", "total threads", "batches sent", "matches done", and "failures", each populated from the current worker registry snapshot plus the local in-process pool

### Requirement: Local in-process pool always listed
The workers table SHALL always include a synthetic first row representing the local in-process match engine pool, labeled distinctly from remote workers and without per-batch counters.

#### Scenario: Page renders with no remote workers registered
- **WHEN** no remote workers have been added
- **THEN** the table still shows one row for the local pool, with status "local", its own computer name, CPU brand, and thread count, and dashes in place of batches sent, matches completed, and failures

### Requirement: Worker table columns
The workers table SHALL list, for each worker (local and remote), its status, host name, network address, version, CPU description, thread count, batches sent, matches completed, failure count, last batch latency in milliseconds, throughput in matches per second, and the last error message if any.

#### Scenario: Remote worker has completed batches
- **WHEN** a remote worker with status "ready" has processed batches
- **THEN** its row shows numeric values for batches sent, matches completed, failures, last latency, and throughput, and a dash for last error when none has occurred

#### Scenario: Worker has an unresolved field
- **WHEN** a worker's computer name, version, CPU brand, latency, throughput, or last error is not yet known
- **THEN** the corresponding cell renders a muted placeholder dash instead of an empty value

### Requirement: Worker status indication
The workers page SHALL display each worker's status as one of ready, local, version mismatch, unreachable, or connecting, using a distinct badge per state.

#### Scenario: Remote worker fails its version handshake
- **WHEN** a worker's status is a version mismatch
- **THEN** its badge shows the version-mismatch label and the row's status detail area shows the worker's reported version

#### Scenario: Remote worker cannot be reached
- **WHEN** a worker's status is unreachable
- **THEN** its badge shows the unreachable label and the status detail area shows the failure reason

### Requirement: Add a remote worker
The workers page SHALL let the user open a dialog to add a remote worker by host and port, submit it to be dialed and version-checked, and see the outcome without leaving the page.

#### Scenario: Operator adds a reachable, compatible worker
- **WHEN** the user enters a host and port in the add-worker dialog and submits it, and the worker responds with a matching version
- **THEN** the dialog shows a success message including the worker's version and thread count, and the page reloads shortly after to show the new row

#### Scenario: Operator adds a worker with a mismatched or unreachable address
- **WHEN** the submitted host/port fails the version check or cannot be reached
- **THEN** the dialog shows a failure message with the reason, the submit button re-enables, and no page reload occurs

### Requirement: Remove a remote worker
The workers page SHALL let the user remove a registered remote worker from its table row, after confirming the action, and the local in-process row SHALL NOT offer a remove control.

#### Scenario: Operator removes a remote worker
- **WHEN** the user clicks the remove control on a remote worker's row and confirms the prompt
- **THEN** the worker is dropped from the registry and the page reloads so the row and summary tiles no longer include it

#### Scenario: Local pool row has no remove control
- **WHEN** the user views the local pool's row
- **THEN** no remove button is present for that row

### Requirement: Live status polling
The workers page SHALL periodically refresh the status badge, detail text, and per-worker counters of existing rows, along with the summary tiles, without a full page reload.

#### Scenario: A ready worker becomes unreachable while the page is open
- **WHEN** the periodic status poll returns an updated status for a worker already shown in the table
- **THEN** the row's badge, status detail, batches/matches/failures counters, last latency, throughput, and last error update in place, and the summary tiles update to match

#### Scenario: A worker is added or removed by another session while the page is open
- **WHEN** the periodic poll includes a worker address not present in the current table, or omits one that is present
- **THEN** the table does not gain or lose rows from polling alone; only a page reload reflects the addition or removal
