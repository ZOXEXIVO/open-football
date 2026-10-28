# viewer/players Specification

## Purpose
Captures the existing observable behavior of the match replay viewer's players module: how the twenty-two footballers and the ball are drawn, dressed, animated and reacted to a recorded match, purely from positions and match events streamed to the viewer.

## Requirements

### Requirement: Player motion is derived from recorded positions, not simulated
A player's on-screen running, turning, pace and posture SHALL be computed each frame from the recorded position track sampled at the current playhead time, rather than from an independent physics simulation, so that the viewer reproduces the same match the engine actually played.

Motion SHALL advance on the clock of whatever has the picture:
- the playhead, while the replay is shown, so a replay that is paused, waiting for data or finished is a still picture;
- real time, while the pre-match line-up has the picture and holds the playhead.

After any discontinuity, a player SHALL be drawn in the stride that the recording implies at the new time. He SHALL NOT ease in from the state he was in before the jump.

#### Scenario: A player accelerates out of a stationary start
- **WHEN** the position track shows a player covering increasing ground between consecutive sampled frames
- **THEN** the viewer eases the player's drawn speed and heading toward the values implied by that ground covered, rather than snapping instantly to them

#### Scenario: A gap in the recording is crossed
- **WHEN** the distance between two consecutive samples for a player exceeds the teleport threshold
- **THEN** the viewer treats the frame as a discontinuity (a restart or seek) instead of drawing the player sliding across the gap at an extreme speed

#### Scenario: A paused replay is a still picture
- **WHEN** playback is paused, is waiting for recording data, or has stopped at full time
- **THEN** every player holds exactly the pose he had on the last frame the playhead moved, breathing, gestures and settling included
- **AND** when play resumes, he resumes from that pose

#### Scenario: The line-up stays alive while the playhead is held
- **WHEN** the pre-match line-up has the picture and the playhead is held at its start
- **THEN** the players standing in the line keep breathing and shifting their weight at real-time pace

#### Scenario: A cut lands in stride
- **WHEN** the playhead jumps, whether by a scrub, by a cut to the next recorded clip, or at the end of the line-up
- **THEN** on the first frame after the jump, each player's pace, heading and lean match what the recording implies around the new time, as closely as if the replay had played through to that time

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

### Requirement: Planted feet hold their place on the turf
A player on his feet SHALL keep each sole that has come down on the grass at the point where it landed, until his stride lifts it. The leg SHALL bend to keep reaching that point as the body moves over it.

If a foot could only be kept in place by stretching the leg past its reach, the foot SHALL come up off the grass toward that point, as at toe-off, rather than slide across it. A standing player whose held foot he has turned too far over, or drifted too far from, SHALL step it back under him.

The same rule SHALL hold at every pace and in every direction of movement, for outfielders and goalkeepers alike. Only a foot that is swinging at the ball is exempt.

#### Scenario: Running through a turn
- **WHEN** a running player changes direction
- **THEN** each planted sole stays where it landed until it lifts, the boot pivoting on it, and the turn is made across successive footfalls rather than by the feet sliding on the grass

#### Scenario: A standing player shifted slightly by the recording
- **WHEN** a standing player's recorded position moves a few centimetres back and forth
- **THEN** his soles stay where they are and his body sways over them

#### Scenario: A foot left behind is re-planted
- **WHEN** a standing player's recorded position drifts so far that one of his planted feet could only be held by over-reaching
- **THEN** he takes a step that brings that foot back under him

#### Scenario: Starting and stopping
- **WHEN** a player accelerates out of a standing start or pulls up from a run
- **THEN** no planted sole slides across the grass, and the change of pace is taken across his footfalls

#### Scenario: The kicking foot is free
- **WHEN** a player swings at the ball
- **THEN** the striking foot follows its swing, and only the standing foot is held to the grass

### Requirement: Outfielders keep the play in view at moderate pace
An outfielder's body SHALL face a direction between his direction of travel and the ball. One continuous rule SHALL set the weight between the two, from two inputs: his pace, and the angle between the two directions.
- At walking pace, he SHALL face the ball.
- As his pace rises, he SHALL turn toward where he runs.
- He SHALL make that turn at a lower pace when moving across the ball's line than when moving directly away from it.

There SHALL be no single pace at which his facing flips between the two.

His legs SHALL step in the direction he actually travels. He backpedals or shuffles sideways under a body that faces the play.

#### Scenario: A defender retreats at jogging pace
- **WHEN** an outfielder moves directly away from the ball at about 2.5 m/s
- **THEN** his body faces the ball and his legs backpedal with a running cycle

#### Scenario: A player sprints away from the ball
- **WHEN** an outfielder moves away from the ball at sprinting pace
- **THEN** he faces where he runs

#### Scenario: A player shuffles across the play
- **WHEN** an outfielder moves across the ball's line at walking to jogging pace
- **THEN** his body faces the ball, his legs side-step, and he turns to face where he runs as his pace rises

#### Scenario: Pace hovering around the old switching pace
- **WHEN** an outfielder's pace hovers around 1.1 m/s
- **THEN** his facing changes smoothly with pace and does not flip back and forth between the ball and his direction of travel

#### Scenario: Running at the ball
- **WHEN** an outfielder moves toward the ball
- **THEN** he faces where he runs at every pace

### Requirement: The stride stays readable at fast playback
The drawn leg cycle SHALL NOT advance by more than a quarter of a stride between two drawn frames. When playback speed, frame rate and a player's pace together would need more than that, his legs SHALL cycle at that limit, and his feet are allowed to fall behind the ground he covers.

#### Scenario: Fast-forward
- **WHEN** playback runs at 16× and a player sprints
- **THEN** his legs are seen swinging forward through each stride and never wheel backwards

#### Scenario: Ordinary speeds are untouched
- **WHEN** playback runs at 1× or 2× on a display drawing 60 frames a second
- **THEN** every player's stride is drawn exactly as it would be without this limit

### Requirement: Player detail follows on-screen size
Each footballer SHALL be drawn at a level of geometric detail chosen from how tall he appears on screen. The finest level SHALL be used only when the camera is close to him, and no player SHALL be drawn finer than his device class allows. A player whose on-screen size sits near the boundary between two levels SHALL NOT switch between them from one frame to the next.

Every level SHALL show the same kit, number, name and face.

#### Scenario: Broadcast view
- **WHEN** the camera shows the play from the gantry at its rest zoom
- **THEN** the players are drawn at the coarse level, wearing the same kit, numbers, names and faces as at the finer levels

#### Scenario: Flown close
- **WHEN** the camera is flown to arm's length of a player
- **THEN** he is drawn at the finest level his device allows

#### Scenario: Hovering near a boundary
- **WHEN** a player's on-screen height hovers around a switching size
- **THEN** he changes level at most once for each time he crosses the boundary, and not on every frame

#### Scenario: Handheld device
- **WHEN** the viewer runs on a handheld device
- **THEN** no player is drawn finer than the handheld level, however close the camera

### Requirement: Locomotion remains stable across recording sample boundaries

At ordinary playback speeds, a player's pace, travel direction, and body lean SHALL reflect sustained recorded movement rather than rounding changes at individual recording samples. Steady movement SHALL produce a steady stride, and displacement confined to the existing standing-motion tolerance SHALL NOT repeatedly start a running cycle or reverse the player's facing. The viewer SHALL preserve the recorded position and contact timeline while smoothing these animation cues.

#### Scenario: A steady jog crosses quantized samples
- **WHEN** a player follows a straight, constant-speed track containing the recorder's normal position quantization
- **THEN** his stride and lean remain steady across sample boundaries without repeated braking, acceleration, or alternating turns
- **AND** his root position continues to match the viewer's recorded-position interpolation at each playhead time

#### Scenario: A standing player jitters within the standing tolerance
- **WHEN** recorded positions move back and forth within the standing-motion tolerance without sustained travel
- **THEN** the player remains standing with planted feet and does not repeatedly start stepping or turn to face each displacement

#### Scenario: Nearby recording data is not loaded
- **WHEN** the current recorded position is available but neighboring samples used to infer motion are unavailable
- **THEN** the viewer uses the available contiguous movement evidence without predicting a path through missing data or delaying playback to obtain animation context
- **AND** additional context arriving during uninterrupted playback does not reset the stride or snap the body into a different pose

### Requirement: Changes of pace and direction carry through the supporting step

Starting, stopping, and changing travel direction SHALL coordinate the player's stride, supporting leg, pelvis, and trunk. A continuous movement transition SHALL preserve stride phase, ease body orientation and lean, and preserve valid foot contacts until toe-off. Forward running, backpedalling, and side-stepping SHALL blend through their shared direction of travel without switching the legs to an unrelated phase. The existing fast-playback stride limit SHALL continue to take precedence when full ground-speed matching is impossible.

#### Scenario: Standing to sprinting and back to rest
- **WHEN** a continuous recorded track accelerates a player from rest to a sprint and then brakes to a stop
- **THEN** his first steps grow into the run, his trunk responds to acceleration and braking, and his final step takes his weight into a supported stance
- **AND** neither transition resets the leg cycle or slides a planted sole to obtain the next pose

#### Scenario: A runner curves and reverses
- **WHEN** a player curves through a turn and reverses his travel direction while staying on his feet
- **THEN** his body orientation, hip opening, lean, and step direction change continuously without a one-frame flip
- **AND** the viewer preserves his recorded path while the feet release and replant as required by their reach

#### Scenario: A defender changes between a shuffle and a backpedal
- **WHEN** a defender or goalkeeper changes between sideways and backward movement while facing the play
- **THEN** the feet continue stepping in his actual travel direction through the transition
- **AND** the transition preserves the current step and does not snap the pelvis between separate directional poses

### Requirement: Ball actions join and leave locomotion continuously

Entering or leaving a strike, reception, goalkeeper save, or goalkeeper recovery SHALL blend from the player's current locomotion pose and return to a pose consistent with his current recorded movement. The striking or receiving foot SHALL be released when the action requires it, while a valid supporting-foot contact SHALL remain planted. Blending SHALL preserve recorded ball-contact and take-off times rather than shifting the event to make the pose easier to draw.

#### Scenario: A player passes while running
- **WHEN** a running player strikes a recorded pass
- **THEN** his hips and trunk turn into the strike from the current stride, the striking foot is free, and he returns to locomotion without a pose snap or stride restart
- **AND** the strike meets the ball at the recorded contact time

#### Scenario: A player receives and immediately moves away
- **WHEN** a reception is followed by continued running or another ball action
- **THEN** the receiving pose blends into the next supported step or action without freezing the legs or abruptly restoring a default run pose

#### Scenario: A goalkeeper recovers into movement
- **WHEN** the recording shows a goalkeeper landing and moving again
- **THEN** recovery gives way continuously to the required movement, without snapping upright or carrying a fully sprawled pose along the ground

### Requirement: Locomotion is consistent across normal playback cadences

For continuous recorded movement after two seconds of common warm-up, playback at 30, 60, or 120 rendered frames per second and at 0.25x, 0.5x, 1x, or 2x SHALL produce comparable locomotion at matching playhead times. On deterministic locomotion fixtures, speed SHALL differ by no more than 0.2 m/s, facing by no more than 5 degrees, and pelvis, trunk, hip, and knee rotations by no more than 10 degrees relative to the 60-fps, 1x reference. Comparisons SHALL exclude intervals where the existing per-frame stride limit activates; that limit SHALL retain its current fast-playback behavior.

All added motion state SHALL obey existing pause, buffering, full-time, line-up, and discontinuity behavior. A relocation SHALL NOT contribute acceleration, braking, or foot contact carried from its previous location.

#### Scenario: Different displays show the same turn
- **WHEN** the same warmed-up locomotion fixture is played at the specified frame rates and ordinary speeds and compared at common playhead times without stride limiting
- **THEN** speed, facing, and body rotations stay within the stated tolerances

#### Scenario: Playback pauses or waits for a chunk
- **WHEN** the replay stops advancing while a player is in a movement transition
- **THEN** his complete pose and all transition state remain frozen until playback advances again

#### Scenario: A seek or placement interrupts a transition
- **WHEN** playback seeks, jumps to a different recorded clip, or a player is instantaneously placed elsewhere
- **THEN** movement cues and foot contacts are reconstructed from the destination's contiguous recording, without a braking impulse, turn, or planted foot inherited from the previous location

#### Scenario: Playback runs at sixteen times normal speed
- **WHEN** 16x playback would advance a player's legs by more than a quarter stride per rendered frame
- **THEN** the stride remains limited to a quarter cycle per frame and does not wheel backward
