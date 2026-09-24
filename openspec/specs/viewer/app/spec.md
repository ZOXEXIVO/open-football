# viewer/app Specification

## Purpose
Capture the existing observable behavior of the match replay viewer's top-level application layer: how it starts up, reports its loading progress, configures itself from the hosting page, and adapts its rendering quality and resolution to the machine it is running on.

## Requirements

### Requirement: Staged scene bring-up with progress reporting
The viewer SHALL construct its opening scene over multiple frames rather than in a single frame, and SHALL report its build progress to the hosting page as a sequence of named phases (starting, building, recording, squad, ready) so the page can render a loading indicator that reflects real progress.

#### Scenario: Opening a match for the first time
- **WHEN** a viewer session is started for a match
- **THEN** the hosting page receives a sequence of progress notifications naming each phase the viewer passes through, ending with a "ready" notification, and each notification includes how many build steps are complete out of the total

#### Scenario: A phase is only reported once
- **WHEN** the viewer remains in the same phase across multiple frames
- **THEN** the hosting page is not sent duplicate notifications for that phase; a notification is only sent when the phase changes

### Requirement: Overlay stays up until players are visibly on the pitch
The viewer SHALL keep the loading state active until footballers have actually been drawn on screen (not merely constructed) and a short settling delay has elapsed, so that the frame in which player materials first compile does not appear as a visible stall inside the reveal.

#### Scenario: Squad rendered after the recording loads
- **WHEN** the recording data has arrived and a player becomes visible on the pitch
- **THEN** the viewer waits a short real-time interval after that player is first drawn before declaring the "ready" phase

#### Scenario: A goalless recording with nobody to draw
- **WHEN** the loaded recording contains no player positions to play back, or the hosting page explicitly requested an empty pitch
- **THEN** the viewer declares "ready" without waiting for any player to appear, so the loading indicator is not left showing indefinitely

### Requirement: Configuration supplied by the hosting page
The viewer SHALL be configured entirely by a document supplied by the hosting page at startup, containing the recording location, the two teams' identities and colors, the full player roster with appearance data, match events (goals, notable chances, substitutions), venue facts, and display label text; fields absent from an older document SHALL fall back to sensible defaults rather than causing a failure.

#### Scenario: Older document without team names
- **WHEN** the configuration document has no team name fields
- **THEN** the viewer still renders correctly, simply presenting no team name text alongside the team colors

#### Scenario: Older document without venue information
- **WHEN** the configuration document has no venue fields
- **THEN** the viewer treats the match as being played at a large, comfortably full stadium

### Requirement: Diagnostic overrides reachable from the page address
The viewer SHALL accept a set of optional overrides in its configuration (device class, crowd size, squad presence, render scale, chunk read-ahead, debug overlay, performance overlay) that let a user force scene characteristics that would otherwise be auto-detected, so that a failure that leaves no error trace on a handheld device can still be diagnosed by trying the scene both ways.

#### Scenario: Forcing a handheld build on a desktop browser
- **WHEN** the configuration explicitly names the device class as handheld
- **THEN** the viewer builds the reduced scene sized for constrained memory regardless of what its own detection would have concluded

#### Scenario: An invalid override value
- **WHEN** an override field contains a value the viewer does not recognize
- **THEN** the viewer falls back to its own automatic decision for that setting instead of failing or silently picking an arbitrary option

### Requirement: Rendering quality adapts downward based on measured frame cost
The viewer SHALL measure how long each displayed frame takes and, after allowing an initial settling period for the scene to finish loading, SHALL reduce its multisample rendering quality if frame times remain in a sustained "struggling" range across multiple consecutive measurements; this reduction happens at most once per session and is never reversed.

#### Scenario: Sustained slow frames on an underpowered machine
- **WHEN** frame times stay in the struggling range for two consecutive measurement windows after the settling period has passed
- **THEN** the viewer switches to its lower-sampling rendering path and does not attempt to raise quality again for the remainder of the session

#### Scenario: A momentary stall is not mistaken for a slow machine
- **WHEN** a single frame takes an extremely long time (for example because the browser tab was backgrounded or paused)
- **THEN** the viewer does not treat this as evidence the machine is struggling and does not reduce quality because of it

### Requirement: Draw resolution scales down when the display cannot be kept up with
Independently of the sampling quality decision, the viewer SHALL periodically review measured frame cost and, when the display is being missed on sustained consecutive reviews, SHALL step the replay's drawn resolution down through a fixed sequence of progressively smaller sizes while preserving the on-screen aspect ratio; this only ever steps down, never back up.

#### Scenario: A machine that cannot sustain full resolution
- **WHEN** two consecutive periodic reviews find the frame rate falling behind the display's refresh
- **THEN** the viewer redraws the replay into a smaller image stretched to fill the same on-screen area, and continues to monitor for further reductions

#### Scenario: A machine comfortably keeping up
- **WHEN** frame timings are consistently within the comfortable range
- **THEN** the viewer leaves the replay's resolution at its current setting and never increases it again once any reduction has occurred

### Requirement: Scene sizing adapts to constrained-memory devices
The viewer SHALL detect whether it is likely running on a handheld device (based on pointer/touch characteristics, user agent, and the graphics adapter actually opened) and, when so, SHALL build a smaller scene and cap render target size to reduce the chance the browser tab is terminated for excessive memory use; this device classification is corrected at most once, downward only, once the real graphics adapter is known.

#### Scenario: A touch device is detected
- **WHEN** the browser reports any of several touch or handheld indicators
- **THEN** the viewer builds the constrained-memory scene variant from the start

#### Scenario: The graphics adapter reveals a handheld device the initial probe missed
- **WHEN** the actual graphics adapter opened for rendering turns out to be a touch-enabled Apple device after the viewer initially assumed a full-size desktop
- **THEN** the viewer corrects its device classification to handheld before the scene is built further, and does not revert this correction later

### Requirement: Session reports its own resource usage
The viewer SHALL be able to report, at any point in its lifecycle, how much memory it currently holds across the major categories of scene content (crowd, ground, textures, squad geometry, per-player materials, render targets, and recording data) plus the browser's committed memory, so that a session that fails without leaving any browser error trace can still be diagnosed from what was last shown on screen.

#### Scenario: The viewer reaches the ready phase
- **WHEN** the viewer completes its bring-up and reports the "ready" phase to the hosting page
- **THEN** it also emits a one-time summary of everything currently held in memory, broken down by category

#### Scenario: A device class is included with every progress update
- **WHEN** any progress phase is reported to the hosting page
- **THEN** the notification includes the current memory usage figures and which device-size classification the viewer is using, so the state at the moment of a failure is visible on screen
