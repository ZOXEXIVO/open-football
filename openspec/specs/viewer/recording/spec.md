# viewer/recording Specification

## Purpose
Captures the existing behavior of the match replay viewer's recording playback: how a recorded match is fetched, streamed, played back, and presented to a viewer, including partial (goals-only) recordings and gaps in coverage.

## Requirements

### Requirement: Recording metadata drives playback setup
The viewer SHALL request recording metadata (chunk count, chunk duration, total match duration, and optionally the recorded time ranges) before it can begin playback, and SHALL retry the request periodically if the recording is not yet available.

#### Scenario: Metadata is not yet available
- **WHEN** the viewer requests metadata for a match and the server reports the recording does not exist yet (the match may still be in progress)
- **THEN** the viewer waits several seconds and requests the metadata again, repeating until it succeeds

#### Scenario: Metadata establishes match duration
- **WHEN** the metadata response includes a total match duration and the viewer has not already established one
- **THEN** the viewer adopts that duration as the length of the playback timeline

### Requirement: Match data streams in incrementally
The viewer SHALL load the recording in discrete time-bounded chunks rather than requiring the whole match to be downloaded before playback starts, and SHALL keep a small number of chunks ahead of the current playback position loaded in advance.

#### Scenario: Playback begins as soon as the first usable chunk arrives
- **WHEN** the first chunk containing ball movement is loaded
- **THEN** the viewer marks itself ready and begins playing without waiting for the rest of the match to download

#### Scenario: Read-ahead adapts to device capability
- **WHEN** the viewer is running on a constrained device (e.g. a handheld) rather than a full-featured one
- **THEN** it keeps fewer chunks loaded in advance of the playhead than it would on a roomier device, trading network responsiveness for lower memory use

#### Scenario: A failed chunk is not retried forever
- **WHEN** a requested chunk fails to load a second time
- **THEN** the viewer stops asking for that chunk again rather than retrying it every frame for the rest of the match

### Requirement: Only the recorded portions of a match are fetched and played
A recording MAY cover only part of a match (for example, clips around each goal) rather than the full ninety minutes. The viewer SHALL only request and play back the time ranges the recording actually contains, and SHALL treat an unrecorded stretch as a gap rather than as missing data to wait for.

#### Scenario: A goals-only recording skips the gaps between clips
- **WHEN** the playhead reaches the end of a recorded clip and no more recording follows immediately after it
- **THEN** playback jumps forward to the start of the next recorded clip instead of continuing to advance across the unrecorded gap

#### Scenario: A recording with nothing kept at all
- **WHEN** the metadata reports recorded time ranges but the list is empty (a goalless match under clip-based recording)
- **THEN** the viewer treats the match as having nothing to play and does not wait further for chunks that will never arrive

#### Scenario: Opening a match that starts mid-recording
- **WHEN** the recording's first content begins after time zero (kickoff itself was not kept)
- **THEN** the viewer opens on the first available recorded moment rather than sitting on an empty pitch at time zero

### Requirement: Playback position advances at real time with adjustable speed
The playhead SHALL advance at wall-clock speed while playing, scaled by a speed multiplier the viewer can change, and SHALL stop advancing once it reaches the end of the match.

#### Scenario: Playhead reaches full time
- **WHEN** the playhead reaches or passes the total match duration
- **THEN** it is clamped to exactly the match duration and playback stops

#### Scenario: Speed can be adjusted up or down
- **WHEN** the viewer changes the playback speed
- **THEN** the position advances proportionally faster or slower than real time, without changing the recorded positions themselves

#### Scenario: A match with no known duration is never "at full time"
- **WHEN** the total duration is not yet known (e.g. metadata has not arrived)
- **THEN** the viewer does not report playback as finished

### Requirement: Seeking and jumping present as a cut, not a glide
When the playhead is moved by the viewer (scrubbing) or by the playback engine (crossing a gap between recorded clips), dependent systems SHALL be told to jump discontinuously rather than smoothly interpolate through the skipped time.

#### Scenario: Manual scrub jumps immediately
- **WHEN** a viewer drags the timeline to a new position
- **THEN** everything that tracks the playhead (camera, on-field markers, console log) jumps to the new position for that frame instead of animating through the intervening time

#### Scenario: Automatic jump across a recording gap is marked distinctly
- **WHEN** playback automatically advances past the end of a recorded clip into the next one
- **THEN** the jump is flagged separately from an ordinary manual seek, so the display can signal a scene change (e.g. a brief dip) rather than treating it like a user-initiated scrub

#### Scenario: Scrubbing into an unrecorded gap lands on the nearest edge
- **WHEN** a viewer scrubs the timeline to a point that falls inside an unrecorded gap
- **THEN** the position used for playback snaps to the nearest boundary of a recorded clip rather than remaining on the requested but unrecorded instant

### Requirement: Entity positions are read back as interpolated movement or as instantaneous placement
Between two recorded samples of the same entity (ball or player), the viewer SHALL interpolate a smooth position when the gap represents plausible travel, and SHALL instead hold the earlier position until the later sample's time is reached when the gap represents an instantaneous placement (e.g. a restart, a substitution, a kickoff reset) that could not physically have been travelled at that speed.

#### Scenario: A struck ball is shown as continuous movement
- **WHEN** two consecutive ball samples are close enough in time and the implied speed of travel between them is within what a struck ball or a running player could plausibly achieve
- **THEN** the viewer shows the entity moving smoothly between the two recorded points

#### Scenario: A restart or reset is shown as a cut, not a flight
- **WHEN** two consecutive samples of the same entity imply a speed of travel far beyond anything achievable by running or a kicked ball (e.g. the ball reappearing at a restart spot, or a player relocated for a set piece)
- **THEN** the viewer holds the entity at its earlier recorded position until the playhead reaches the later sample's time, rather than showing it fly or skate across the intervening distance

#### Scenario: A vertical drop counts toward the same judgment
- **WHEN** the height component of a sample changes sharply between two consecutive samples
- **THEN** that vertical change is included when deciding whether the movement was plausible travel or an instantaneous placement

### Requirement: Entities absent from the recording near the playhead are not shown as present
A player or the ball SHALL only be considered "on the pitch" at a given playback time if the recording has actual sample data near that time, distinct from the recording simply not having downloaded yet.

#### Scenario: A player with no nearby samples is treated as off the pitch
- **WHEN** the playhead is at a time further than the recording's presence tolerance from any sample of a given player
- **THEN** that player is treated as not on the pitch at that moment (e.g. not yet substituted on, or substituted off)

#### Scenario: Data still streaming in is not mistaken for absence
- **WHEN** the chunk covering the current playback time has not finished loading yet
- **THEN** the viewer does not conclude that a player with no samples loaded there is genuinely absent from the match

### Requirement: Loaded recording data is bounded in memory around the playhead
The viewer SHALL discard recorded samples and chunk data that fall well outside a window around the current playhead position, so that memory use does not grow to hold the entire match regardless of how long or how far the viewer has scrubbed.

#### Scenario: Chunks far behind the playhead are dropped
- **WHEN** the playhead has moved well past a previously loaded chunk
- **THEN** that chunk's sample data is discarded, and it can be requested again later if the playhead returns to that time

#### Scenario: An upcoming clip is preserved even if it is far away
- **WHEN** the next recorded clip is many chunks ahead of the current playhead (as happens in a goals-only recording)
- **THEN** the samples for that upcoming clip are kept (or fetched) rather than being swept away by the general distance-based eviction

### Requirement: Recorded match events are surfaced as the playhead passes them
The viewer SHALL expose the recorded log of in-match events (e.g. ball and player occurrences) to an observer, emitting each event once as playback reaches its recorded timestamp, and SHALL re-align which events have already been announced whenever the playhead jumps.

#### Scenario: Events are announced in order during normal playback
- **WHEN** playback advances past the timestamp of a recorded event
- **THEN** that event is reported exactly once, in timestamp order

#### Scenario: A seek does not replay old events
- **WHEN** the viewer seeks to a new position on the timeline
- **THEN** only events at or after that position are eligible to be announced going forward; events before it are not re-announced
