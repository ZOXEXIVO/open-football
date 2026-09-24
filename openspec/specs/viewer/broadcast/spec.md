# viewer/broadcast Specification

## Purpose
Captures the existing observable behavior of the match replay viewer's camera and presentation system: how the picture is framed, moved, and handed between the automatic broadcast shot and viewer-driven controls.

## Requirements

### Requirement: Broadcast follow camera
The viewer SHALL present a default camera positioned high on the halfway-line side of the pitch that pans, tilts, and slides a limited distance to keep the ball (or an active subject) framed, without otherwise leaving its resting position.

#### Scenario: Idle match play
- **WHEN** no viewer control has been touched and no special moment (substitution, goal celebration, line-up, subject lock) is active
- **THEN** the camera stays on its default high, set-back position, gently panning and sliding to follow the ball's position on the pitch

#### Scenario: Ball leaves the pitch
- **WHEN** the ball goes out of play
- **THEN** the camera eases its aim back toward the center of the pitch rather than continuing to chase the ball's last position

### Requirement: Manual camera controls
The viewer SHALL let a viewer orbit the camera around the pitch, zoom the lens, and fly a free camera by hand, and SHALL provide a way to restore the default shot.

#### Scenario: Dragging to orbit
- **WHEN** the viewer holds the right or middle mouse button (or performs the equivalent one-finger touch drag) and drags
- **THEN** the camera orbits around the center of the pitch at a fixed distance, changing bearing and elevation within bounded limits, while it is not in a special moment or free-flight

#### Scenario: Scrolling to zoom
- **WHEN** the viewer scrolls the mouse wheel or performs a pinch gesture
- **THEN** the camera's field of view narrows or widens smoothly within a bounded range, applied on top of whatever shot currently owns the camera

#### Scenario: Flying free
- **WHEN** the viewer presses a movement key (WASD/arrows, Q/E) or uses the on-screen flight stick
- **THEN** the camera detaches from the automatic broadcast rig and moves freely in the pressed direction, continuing from its current on-screen position rather than jumping

#### Scenario: Resetting the view
- **WHEN** the viewer activates the reset control
- **THEN** the camera returns to its default position, default zoom, ball-following behavior, and any followed player or free flight is released

### Requirement: Following an individual player
The viewer SHALL let a viewer pick out one player on the pitch and have the camera track and visually mark that player until released.

#### Scenario: Clicking a player
- **WHEN** the viewer clicks or taps a player's on-screen figure
- **THEN** the camera smoothly tightens and re-centers onto that player over roughly half a second, and a glowing ring marker appears on the ground beneath him

#### Scenario: Releasing the followed player
- **WHEN** the viewer clicks empty grass, presses Escape, or otherwise takes back manual control of the camera
- **THEN** the camera smoothly widens back out to following the ball over the same rough half-second, and the ring marker disappears

#### Scenario: Followed player leaves the pitch
- **WHEN** the followed player is substituted off, sent off, or otherwise no longer present
- **THEN** the camera automatically releases the lock and resumes following the ball

### Requirement: Substitution camera sequence
The viewer SHALL show a dedicated close-up camera sequence for a substitution: the incoming player(s)' faces, then a swing to view their names on their backs, then their run onto the pitch, before returning to the broadcast shot.

#### Scenario: Single substitution
- **WHEN** a substitution begins with one incoming player standing at the touchline
- **THEN** the camera moves close to that player's face, swings around behind him to show the name on his shirt, holds while he begins running on, and then eases back to the broadcast camera

#### Scenario: Multiple substitutions at once
- **WHEN** several players come on at the same stoppage
- **THEN** the camera pans along the row of incoming players' faces before swinging round to their backs, giving proportionally more time to the pan the more players are involved

#### Scenario: A bystander blocks the shot
- **WHEN** another player on the pitch stands between the camera and the substitute being shown
- **THEN** the camera moves closer to avoid the obstruction while widening its lens to keep the subject the same visual size, rather than showing the obstruction

### Requirement: Goal celebration camera
The viewer SHALL hold the camera on the ball briefly after a goal is scored, then shift attention to wherever players have gathered to celebrate, before returning to normal play framing.

#### Scenario: A goal is scored and celebrated
- **WHEN** the ball crosses the goal line and players subsequently gather in celebration
- **THEN** the camera stays on the netted ball briefly, then swings toward and tightens on the cluster of celebrating players, following whichever group is most tightly packed together

#### Scenario: No real celebration forms
- **WHEN** a goal is scored but players remain spread out or standing in normal defensive/attacking shape rather than gathering
- **THEN** the camera does not commit to a celebration shot and stays on ordinary play framing

### Requirement: Pre-match line-up ceremony
The viewer SHALL open certain matches with an uninterrupted camera sequence showing both teams standing in a line before kickoff, flying in from overhead, then passing along the row of players' faces, before cutting to the start of play.

#### Scenario: Match opens with a line-up
- **WHEN** a replay with a recorded starting line-up begins playback from the start
- **THEN** the camera descends from high over the center of the pitch, swings around the end of the line of players, and passes along their faces left to right before cutting to the normal match camera at kickoff

#### Scenario: Viewer skips the ceremony
- **WHEN** the viewer presses play, seeks, clicks a player, or otherwise interacts with playback while the line-up sequence is showing
- **THEN** the line-up sequence ends immediately and playback resumes from wherever it would normally be

### Requirement: Cut transition on playback jumps
The viewer SHALL visually signal when the replay's playhead jumps between non-contiguous recorded clips, rather than showing a jarring hard cut.

#### Scenario: Playback jumps to the next highlighted clip
- **WHEN** the playhead advances past the end of one recorded clip into the start of the next
- **THEN** the screen darkens instantly at the moment of the jump and then fades back to a clear picture over about half a second

#### Scenario: Playing at higher speed
- **WHEN** a clip-to-clip jump occurs while the viewer is playing back at a faster-than-normal speed
- **THEN** the fade-back-in completes proportionally faster, taking up correspondingly less of the following clip

### Requirement: Camera never leaves the playing area
The viewer SHALL keep every camera position, whether automatic or manually flown, within a bounded region around the pitch so the picture never shows an empty void or passes through solid scenery.

#### Scenario: Viewer flies to the edge of the allowed area
- **WHEN** the viewer flies the free camera outward from the pitch
- **THEN** the camera stops at a boundary some distance beyond the stands rather than continuing indefinitely, and stays above a minimum height above the ground
