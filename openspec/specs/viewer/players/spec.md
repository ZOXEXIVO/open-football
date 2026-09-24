# viewer/players Specification

## Purpose
Captures the existing observable behavior of the match replay viewer's players module: how the twenty-two footballers and the ball are drawn, dressed, animated and reacted to a recorded match, purely from positions and match events streamed to the viewer.

## Requirements

### Requirement: Player motion is derived from recorded positions, not simulated
A player's on-screen running, turning, pace and posture SHALL be computed each frame from the recorded position track sampled at the current playhead time, rather than from an independent physics simulation, so that the viewer reproduces the same match the engine actually played.

#### Scenario: A player accelerates out of a stationary start
- **WHEN** the position track shows a player covering increasing ground between consecutive sampled frames
- **THEN** the viewer eases the player's drawn speed and heading toward the values implied by that ground covered, rather than snapping instantly to them

#### Scenario: A gap in the recording is crossed
- **WHEN** the distance between two consecutive samples for a player exceeds the teleport threshold
- **THEN** the viewer treats the frame as a discontinuity (a restart or seek) instead of drawing the player sliding across the gap at an extreme speed

### Requirement: Goalkeeper animation reflects saves, dives and the ball
The goalkeeper's pose SHALL react to the ball's approach and to recorded height changes: coiling before a take-off, tracking flight direction and reach during a dive or jump, and settling into a landing recovery afterward.

#### Scenario: The recorded track shows the keeper leaving the ground
- **WHEN** the keeper's recorded height rises above the airborne threshold following a normal (non-teleported, non-hop) run-up
- **THEN** the viewer loads the legs in the lead-up, drives the flight pose toward the ball's approach point, and blends out of the jump pose once the recorded height returns to the ground

#### Scenario: A shot is saved and held
- **WHEN** the ball comes to rest within the keeper's reach and stays there across frames
- **THEN** the viewer closes the gloves around it (a carry/cradle pose) and plays a brief recoil immediately after contact, fading out afterward

### Requirement: Goals trigger a shared, time-limited team reaction
When a goal is registered in the fixture's goal list and the playhead has passed its recorded time, the viewer SHALL apply an elated reaction to every player on the scoring side and a dejected reaction to every player on the conceding side, holding at full strength for a fixed duration and then fading out.

#### Scenario: The playhead moves just past a goal's recorded time
- **WHEN** the current match clock is at or just after a goal event's time and no goal follows it
- **THEN** players on the side the goal belongs to are drawn as elated and players on the other side are drawn as despairing, both at full strength

#### Scenario: The playhead is far from any goal
- **WHEN** the current match clock has no goal in the fixture's list at or before it, or the reaction window for the most recent goal has fully faded
- **THEN** no player on the pitch is drawn with any elation or despair from this system

### Requirement: Squad members are visually individualized
Each player's build, stride length, running cadence, arm carriage, stance at rest, and reaction style SHALL be derived deterministically from that player's own identifier, so that a squad does not read as one animation repeated twenty-two times, while consecutive player ids do not produce visibly correlated appearances.

#### Scenario: Two players of the same height run at the same pace
- **WHEN** two on-pitch players share a height band but have different ids
- **THEN** their stride length, bounce, elbow bend and running stance differ, rather than both playing an identical run cycle

#### Scenario: Skin, hair and eye coloring is requested for a player
- **WHEN** the viewer builds a player's appearance
- **THEN** skin, hair and eye tones come from the values supplied for that player (reflecting nationality) rather than being derived from the player's numeric id, while non-meaningful traits (build, stride, stance, boot color) are derived from the id

### Requirement: Kit colors are chosen for readability, not copied verbatim
Each side's shirt, shorts, socks, trim and printed numbers/names SHALL be derived from the club's registered colors with contrast rules applied, and goalkeepers SHALL always wear one of two fixed keeper colors distinct from both outfield kits, so that all four kits stay visually distinguishable from each other and from the pitch.

#### Scenario: A club's registered colors are too close together
- **WHEN** a club's background and foreground colors do not separate enough to read as two colors
- **THEN** the shorts fall back to a fixed light or dark neutral chosen by the shirt's own brightness, instead of repeating the near-identical foreground color

#### Scenario: Both goalkeepers are in frame together
- **WHEN** the home and away goalkeepers are both visible at once
- **THEN** they wear two different fixed shades of the same keeper color family, distinguishable from each other and from the grass

### Requirement: A player's face is a real photograph when one is available, never a hybrid
The viewer SHALL request the picture URLs supplied for a player (photograph first, then a drawn portrait) after that player has been dressed for the pitch, and SHALL replace his drawn/default face, matching skin tone, and hair color with values read from whichever picture is fetched successfully; a player whose picture never arrives keeps his originally drawn face rather than a partial repaint.

#### Scenario: A player's photograph fetch fails or is blocked
- **WHEN** the photograph URL 404s, is cross-origin blocked, or the request never resolves
- **THEN** the viewer falls back to the drawn portrait URL if one was supplied, and if that also fails the player keeps the face, skin tone and hair the viewer originally drew for him

#### Scenario: A photograph arrives successfully
- **WHEN** a player's photograph is fetched, decoded and its background keyed out
- **THEN** the viewer repaints that player's face sheet with the photograph, retones his skin and shared-limb materials to the nearest shared tone sampled from the picture, and hides the drawn hair cap so his own photographed hair shows instead

### Requirement: Portrait fetching is rate-limited to protect frame time and network priority
The viewer SHALL space out portrait requests and fold at most one decoded picture into the scene per frame, rather than issuing or applying a whole squad's worth of picture fetches at once.

#### Scenario: A full squad is dressed for kickoff in quick succession
- **WHEN** many players are sent for pictures within the same few frames
- **THEN** requests go out one at a time with a minimum real-time spacing between them, and decoded pictures already waiting are applied to the scene no faster than one per rendered frame

### Requirement: Garment surfaces deform continuously with the underlying body rig
Shirts and shorts SHALL be built as a single continuous surface that follows the chest, pelvis and limb joints of the pose beneath them, rather than as separate rigid pieces, so that sleeves and cuffs do not visibly separate from the arms and legs they cover as a player moves.

#### Scenario: A player raises an arm during play
- **WHEN** the shoulder joint rotates through its running or reaching pose
- **THEN** the sleeve and cuff mesh deforms with the arm's skin weights so the cloth and the arm move together without a seam or gap opening at the join
