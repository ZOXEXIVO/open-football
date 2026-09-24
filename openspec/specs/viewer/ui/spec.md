# viewer/ui Specification

## Purpose
Captures the existing on-screen HUD of the match replay viewer: the score display, the transport/playback bar, the pre-match team sheets, the full-time result card, touch-based playback and camera furniture, and the project watermark, as observed from the current source.

## Requirements

### Requirement: Score display tracks the playhead
The viewer SHALL show a small score panel in the top-left corner of the picture, naming both clubs and displaying the goal tally each side has reached as of the current playhead position, not the eventual final score.

#### Scenario: Score updates as playback reaches a goal
- **WHEN** playback of the recording advances past the timestamp of a goal
- **THEN** the scoring side's figure on the score panel increments on that frame, and the figure briefly takes on the scoring club's own colour before returning to the panel's neutral ink

#### Scenario: Score panel names both clubs without truncation
- **WHEN** the viewer is configured with home and away club names of any length
- **THEN** the score panel displays both names in full, uppercased, without truncating or wrapping them, and the panel grows to fit

#### Scenario: Own goals credit the conceding side
- **WHEN** a recorded goal is marked as an own goal
- **THEN** the tally on the score panel credits the side the goal was scored against, not the scoring player's own side

### Requirement: Full-time result card appears when playback ends
When the playhead reaches the end of the recording, the viewer SHALL present a card over the paused picture showing the final score, each side's goalscorers with the minute they scored, and shall remove that card if playback is restarted.

#### Scenario: Replay reaches the final whistle
- **WHEN** the playhead reaches the duration of the match recording
- **THEN** a card rises into view over a darkened version of the frozen picture, showing the two club names, the final score, and a list of scorers (name and minute, with an "(OG)" suffix for own goals) grouped under the side they are credited to

#### Scenario: Long scorer lists are summarized
- **WHEN** one side has more goalscorer entries than the card's visible limit
- **THEN** the card lists the limit's worth of names and shows a "+N" count for the remainder instead of growing further

#### Scenario: Restarting playback dismisses the card
- **WHEN** the viewer restarts playback after reaching full time
- **THEN** the full-time card and its darkening overlay are removed immediately rather than fading out

### Requirement: Team sheets are shown during the pre-match walkout
The viewer SHALL display each club's starting eleven and substitutes, grouped under a club-coloured heading, during the pre-match aerial camera sequence, and SHALL hide this card once the ceremony moves to ground level.

#### Scenario: Walkout begins
- **WHEN** the pre-match ceremony's aerial camera sequence is active
- **THEN** two team-sheet panels are shown, each listing its club's starting players with shirt number, first name, and surname, followed by a substitutes section

#### Scenario: Ceremony reaches the players
- **WHEN** the pre-match ceremony sequence ends
- **THEN** the team-sheet panels are hidden immediately, without a fade transition

#### Scenario: Card fits a narrow window
- **WHEN** the browser window is too small to show the team sheets at their normal size
- **THEN** the sheets are scaled down uniformly to fit within the visible picture above the transport bar, never scaled larger than their normal size

### Requirement: Transport bar provides playback controls
The viewer SHALL provide a bar fixed to the bottom of the picture containing a play/pause control, a scrubbable progress rail, a playback speed control, a mute/unmute control, a camera-reset control, and a running match clock.

#### Scenario: Viewer presses play/pause
- **WHEN** the viewer clicks or taps the play/pause control
- **THEN** playback toggles between playing and paused, and if the recording had already finished, playback restarts from the beginning and resumes playing

#### Scenario: Viewer drags the progress rail
- **WHEN** the viewer presses and drags anywhere along the scrub rail, with a mouse or a touch
- **THEN** the playhead moves to the corresponding position in the match, snapping away from stretches of the recording that were not captured

#### Scenario: Viewer cycles playback speed
- **WHEN** the viewer clicks the speed control
- **THEN** playback speed advances to the next step in the cycle (and moves to the previous step when clicked while holding Shift), and the displayed speed label updates to match

#### Scenario: Viewer mutes the stadium sound
- **WHEN** the viewer clicks the sound control
- **THEN** stadium audio is muted or unmuted, and the control's highlighted appearance reflects whether sound is currently on

### Requirement: Progress rail marks goals, chances, and substitutions
The transport bar's progress rail SHALL display a distinct marker, positioned by time and colored by the scoring/involved side's kit, for every goal, notable chance, and substitution in the match, and SHALL visually distinguish goals from the other two event types, and SHALL show grey coverage over stretches of the match that were not recorded.

#### Scenario: Recording contains a mix of event types
- **WHEN** the match recording includes goals, chances, and substitutions
- **THEN** each is drawn as a pin on the rail at its proportional time position, in the shirt color of the side it belongs to, with goals drawn as larger markers than chances or substitutions

#### Scenario: Playhead is dragged into an ungapped recording
- **WHEN** part of the match was never captured in the recording
- **THEN** that stretch of the rail is covered with a flat grey overlay, and dragging the playhead into it snaps to the nearest edge of a recorded clip instead

### Requirement: Match clock and loading state are displayed
The transport bar SHALL show the current match time and half, and the viewer SHALL show a loading notice while the recording has not finished loading, or a "no recording" notice when the match produced no recorded clips.

#### Scenario: Recording has clips and is still loading
- **WHEN** the recording is not yet fully loaded and does contain recorded clips
- **THEN** a "loading" notice is shown over the picture and is hidden once loading completes

#### Scenario: Recording has no clips at all
- **WHEN** the match produced no recorded clips of any kind
- **THEN** a "no recording available" notice is shown instead of the loading notice, and is not replaced by it

### Requirement: Touch controls provide playback and camera flight input on touch devices
On a device where a touch has been detected, the viewer SHALL reveal an on-screen directional stick and a pair of altitude buttons for flying the free camera, in addition to the touch-adapted transport bar controls, and SHALL keep these controls hidden until a touch actually occurs.

#### Scenario: First touch is detected
- **WHEN** the viewer receives its first touch input on the canvas
- **THEN** the flight stick and altitude buttons become visible in the bottom corners of the screen, clear of the transport bar

#### Scenario: Viewer drags the flight stick
- **WHEN** the viewer drags a thumb on the flight stick beyond its dead zone
- **THEN** the free camera moves in the corresponding direction, with the stick's knob following the thumb and changing color to indicate active movement, and moving faster once the thumb reaches the stick's outer rim

#### Scenario: Viewer uses one or two fingers on the open pitch
- **WHEN** the viewer drags one finger over the open pitch (outside any control)
- **THEN** the camera orbits as it would from a mouse drag; **WHEN** the viewer pinches with two fingers instead, the camera zoom scales by the change in distance between the fingers

### Requirement: Project watermark is shown as static furniture
The viewer SHALL display a small, fixed watermark bearing the project's mark in a corner of the picture, clear of the transport bar, for the entire duration of the replay, unaffected by playback state.

#### Scenario: Replay is viewed at any point in playback
- **WHEN** the replay is playing, paused, or showing the full-time card
- **THEN** the watermark remains visible in its fixed corner position and does not change appearance
