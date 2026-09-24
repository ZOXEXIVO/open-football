# web/worker Specification

## Purpose
Captures the existing behavior of the background match-processing worker dispatch system: how a batch of matches is split and routed to remote workers or run locally, how failures are handled, and what guarantees the caller gets about the result set.

## Requirements

### Requirement: Batch routing across local and remote capacity
The system SHALL split a batch of matches to be played across available background workers and, optionally, a local processing share, using a shared pull-based work queue so that faster capacity picks up more work than slower capacity within the same batch.

#### Scenario: Batch dispatched to multiple ready workers
- **WHEN** a batch of matches is dispatched and more than one worker is in the ready state
- **THEN** each ready worker (and the local share, if enabled) pulls chunks of work from a shared queue until the queue is empty, so no single worker is left idle while others still have queued work

#### Scenario: Small batch stays local
- **WHEN** a batch is small enough to be fully absorbed by the configured local processing share in a single chunk
- **THEN** the batch is run entirely locally and no remote worker is contacted for it

### Requirement: No available capacity falls back to the caller
The system SHALL report an explicit failure to the caller when there is no local share configured and no worker is in the ready state, rather than silently dropping the batch.

#### Scenario: No workers and no local share
- **WHEN** a batch is submitted, local share is disabled, and no worker is currently ready
- **THEN** dispatch returns the original batch back to the caller unprocessed so the caller can run it through its own fallback path

### Requirement: Failed remote work is retried and never lost
The system SHALL guarantee that every match in a submitted batch eventually appears exactly once in the result set, even if one or more remote workers fail mid-batch.

#### Scenario: A remote worker fails on its assigned chunk
- **WHEN** a chunk of matches sent to a worker fails (connection error, error response, a timed-out reply, or a reply whose shape does not match what was requested)
- **THEN** that worker is taken out of consideration for further work in the batch, and the chunk it was working on is returned to the front of the shared queue so a healthy worker retries it before any other queued work

#### Scenario: All capacity fails during a batch
- **WHEN** every worker that could have processed a given match fails, or the task handling it terminates unexpectedly
- **THEN** the unprocessed matches are run locally as a final step, so the caller always receives a complete, correctly sized result set for the original batch

### Requirement: One-time capability handshake per worker connection
The system SHALL require a worker to complete a capability/compatibility handshake as the first exchange on a new connection before any match work is sent, and SHALL refuse to use a worker whose handshake indicates incompatibility.

#### Scenario: Version or protocol mismatch
- **WHEN** a worker's reported software version or wire protocol version does not match the coordinator's
- **THEN** the worker rejects (or the coordinator marks unusable) the connection with a stated reason, and the connection is not used for match work

#### Scenario: Successful handshake exposes worker capacity
- **WHEN** a worker completes the handshake successfully
- **THEN** the worker reports how many matches it can process in parallel, along with identifying host information, and becomes eligible to receive match batches

### Requirement: Recording preference is set by the requester, not the worker
The system SHALL let the party that will ultimately serve or store the match replay decide what gets recorded, communicated once per connection, rather than allow a worker to decide independently.

#### Scenario: Requester has recording disabled
- **WHEN** the requester's own recording settings are off at the time a worker connection is established
- **THEN** the worker performs no position/event recording for matches on that connection, and the response carries no replay data for them

#### Scenario: Requester has recording enabled
- **WHEN** the requester's recording settings request position and/or event capture
- **THEN** the worker records accordingly and returns the finished replay data alongside the corresponding match result

### Requirement: Idle connections are health-checked and reconnected automatically
The system SHALL periodically verify that idle, previously-ready workers are still responsive, and SHALL periodically attempt to reconnect workers that are not currently usable, without requiring manual intervention.

#### Scenario: A ready worker goes silent while idle
- **WHEN** a worker has not shown proof of activity for longer than the idle threshold
- **THEN** it receives a liveness probe; if no timely valid reply arrives, the worker is marked unusable and removed from consideration until it reconnects

#### Scenario: A worker recovers after being unreachable
- **WHEN** a previously unusable worker becomes reachable again on a later health check
- **THEN** it re-completes the handshake and, on success, becomes eligible again for new batches, with its historical statistics preserved

### Requirement: A submitted batch produces exactly one outcome per submitted item, in order
The system SHALL preserve a one-to-one correspondence between each item submitted in a batch and each outcome returned, matched by the item's position in the batch, regardless of how the batch was internally split or reordered during dispatch.

#### Scenario: Mixed batch of match kinds
- **WHEN** a batch contains a mix of match kinds (e.g. scheduled league/cup fixtures and standalone squad-vs-squad fixtures)
- **THEN** each submitted item receives a matching outcome of the same kind, correctly paired back to its original position or identifier

#### Scenario: Replay data accompanies its match without being embedded in the result
- **WHEN** a match that was recorded finishes processing on a worker
- **THEN** the finished replay artifacts are delivered separately from the match result but are unambiguously associated with that result before the caller sees the final outcome
