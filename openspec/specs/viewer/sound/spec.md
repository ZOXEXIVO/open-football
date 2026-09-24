# viewer/sound Specification

## Purpose
Captures the existing behavior of the match replay viewer's sound: which ball contacts are made audible, how loud and how bright each one sounds, and how playback state (mute, pause, seek) governs the audio output.

## Requirements

### Requirement: Only ball contacts are audible
The viewer SHALL produce sound only for the ball being sent away and the ball being taken under control. Ambient crowd noise, whistles, commentary, and the touches a player uses to carry or dribble the ball SHALL NOT produce any sound.

#### Scenario: A dribble touch stays silent
- **WHEN** a player knocks the ball ahead of himself and remains its owner a fraction of a second later
- **THEN** no sound is played for that touch

#### Scenario: A pass is heard going and arriving
- **WHEN** a player sends the ball away and a different player subsequently keeps it
- **THEN** exactly two sounds are heard: one when the ball leaves the sender, and one when the ball is gathered by the receiver

### Requirement: Contact loudness and tone scale with strike power
The viewer SHALL vary the loudness, brightness, and duration of a ball-contact sound according to how hard the ball was struck, rather than only its loudness. A harder strike SHALL sound both louder and brighter than a softer one, and the increase SHALL grow smoothly rather than switching abruptly at any speed threshold.

#### Scenario: A shot is louder and brighter than a gentle pass
- **WHEN** a ball is struck near the hardest speed observed in a match, compared with a gentle pass
- **THEN** the resulting sound is louder, has a brighter high-frequency component, and rings on longer than the gentle pass

#### Scenario: Nearby strike speeds sound continuous
- **WHEN** two balls are struck at speeds a small amount apart
- **THEN** the resulting sounds' loudness and tone differ only slightly, without a perceptible jump at any particular speed

### Requirement: Softest touches remain audible
Every ball contact SHALL be audible above silence, including the gentlest passes, regardless of how the surrounding mix is scaled.

#### Scenario: A five-metre pass can still be heard
- **WHEN** a ball is passed at the lowest speed anybody in a match plays it at
- **THEN** the resulting sound still has a clearly audible, non-negligible loudness

### Requirement: The surface making contact changes the sound
The viewer SHALL make a ball contact sound different depending on whether it was met with a boot, a head, or a throw, independent of how hard it was struck.

#### Scenario: A header never sounds like a boot strike
- **WHEN** a ball is headed at the same speed as it would be volleyed
- **THEN** the header's sound is duller and quieter than the volley's sound

#### Scenario: Throw-ins stay in the background
- **WHEN** a goalkeeper's throw or a throw-in occurs, at any strength
- **THEN** its sound is quieter than even the softest boot contact in the match

### Requirement: A reception is quieter and duller than a departure
The viewer SHALL make the sound of the ball being received distinguishably quieter and duller than the sound of the ball being sent, so a listener can tell which end of a pass they are hearing.

#### Scenario: The arriving half of a pass is the quieter half
- **WHEN** the same pass is heard both leaving one player and being taken under control by another
- **THEN** the arrival's sound is quieter and less bright than the departure's sound

### Requirement: A ball entering the goal produces exactly one net sound per goal
The viewer SHALL play a net sound once when the ball enters a goal, and SHALL NOT play a second net sound while the same ball remains inside or grazing the goal boundary before a subsequent restart.

#### Scenario: A ball settling into the net does not ring twice
- **WHEN** a scored ball rolls to rest against the goal boundary, crossing slightly in and out of the strictest goal boundary as it settles
- **THEN** only one net sound is played for that goal

#### Scenario: A goalkeeper picking the ball out of the net is not heard as a reception
- **WHEN** the ball has already been recorded as entering the goal
- **THEN** no departure or reception sound is played for the keeper standing near it afterward, only the single net sound already played

### Requirement: Sound positioning reflects pitch location
The viewer SHALL pan each ball-contact sound left-to-right according to where along the pitch's length the contact occurred, without ever panning fully to one side.

#### Scenario: Contacts at opposite ends of the pitch pan to opposite sides
- **WHEN** a contact occurs at one end of the pitch and another occurs at the far end
- **THEN** the two sounds are panned to opposite sides of the stereo image, each held short of the extreme left or right position

### Requirement: Sound follows playback and mute state
The viewer SHALL silence all sound while the replay is paused or the viewer has muted it, and SHALL resume producing sound without replaying stale or duplicate events when playback resumes or a jump in playback position occurs.

#### Scenario: Pausing the replay silences it
- **WHEN** the viewer pauses playback or enables mute
- **THEN** the audio output level is brought to silence and no further contact sounds are scheduled until playback resumes and mute is off

#### Scenario: Seeking does not replay old or phantom events
- **WHEN** the viewer jumps to a different point in the recording
- **THEN** no sound is played for events from the point the playback left, and possession state is re-established at the new position without an immediate false reception or departure sound
