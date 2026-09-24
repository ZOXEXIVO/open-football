# viewer/art Specification

## Purpose
Captures the existing, observable visual behavior of the replay viewer's generated art assets — faces, portraits, shirt lettering, perimeter boards, netting, turf, and the crowd — all painted at startup rather than loaded from external files.

## Requirements

### Requirement: Every visual texture is generated at startup, never fetched
The viewer SHALL produce every pixel surface it needs — player faces, portraits, shirt numbers and names, name plates, perimeter-board advertising, goal netting, turf, the football, stadium seating, the crowd, and the sky — by painting it locally when the replay starts, without requesting any image from a network asset server.

#### Scenario: Replay opens with no network image requests
- **WHEN** the replay viewer starts up
- **THEN** the pitch, stadium, kits, and every other visible surface are fully rendered from the first frame, with no loading gap spent waiting on downloaded images

### Requirement: Player faces are shown as photographs, generated portraits fill in the rest
A player on the pitch SHALL be shown wearing his own photograph on the front of his head where one is known, and a portrait generated from his profile where it is not; the sides, back, and underside of the head that no photograph ever covers SHALL be filled by a procedurally painted face using the player's skin, hair, eye color, brow weight, beard style, and baldness.

#### Scenario: A player with a known photograph takes the field
- **WHEN** a player who has a photograph is rendered
- **THEN** his photograph appears on the front of his head, orthographically projected and scaled so his measured inter-pupil distance matches the head model's eye spacing, and the rest of the head (not covered by the photo) is filled with a generated face toned to match the photograph's cheek and hair coloring

#### Scenario: A player's face never shows a hole or a mismatched patch
- **WHEN** a photograph is too narrow, too short, or missing for a player
- **THEN** the viewer falls back to a fully generated face rather than leaving a transparent gap or a visibly discolored patch on his head

### Requirement: Shirts are lettered with numbers, back names, and a front name plate
Every outfield shirt SHALL carry a squad number on the back, the player's surname above it when it can be printed, and a rounded name plate on the front showing the surname in the kit's print color knocked out of a plate in the shirt's own color; all lettering SHALL be drawn in capitals using one of two compiled-in typefaces (a Latin face and a wider-coverage fallback face), with unsupported accented characters folded to their closest plain Latin equivalent rather than dropped as a blank box.

#### Scenario: A name uses only accented Latin characters the primary face supports
- **WHEN** a player's surname (e.g. "Conceição") is printed on the back of his shirt
- **THEN** every character is set in the same typeface as the rest of the viewer's labels, printed as spelled rather than stripped of its accents

#### Scenario: A name contains no character either compiled-in face can draw
- **WHEN** a player's surname folds to nothing under both typefaces
- **THEN** the shirt is printed with its number alone and no blank or placeholder lettering appears where the name would go

#### Scenario: A short surname on the front name plate
- **WHEN** a two-letter surname such as "LI" is placed on the front name plate
- **THEN** the plate is still drawn wide enough to read as a name badge rather than shrinking to a near-square patch around the letters

### Requirement: Perimeter boards show a sponsor lockup that repeats around the pitch
Each perimeter hoarding SHALL display a lockup of a mark (initials in a rounded tile), a wordmark, a divider, and an address sharing one baseline, tiled repeatedly along the board's length with visible air between repeats; if none of the sponsor's text can be drawn by either compiled-in face, the board SHALL render as a plain, unlettered panel in the board's base color instead of an empty or broken texture.

#### Scenario: A sponsor lockup is viewed edge-on from the broadcast camera
- **WHEN** the camera views a perimeter board nearly end-on along the touchline
- **THEN** the lettering remains legible along the board's full length rather than blurring away on the far half

#### Scenario: Sponsor text cannot be rendered by any available face
- **WHEN** a hoarding is given text with no drawable characters in either typeface
- **THEN** the board is shown as a plain colored panel with no advertising, rather than an empty or corrupted texture

### Requirement: Goal netting visibly moves and reads as cord, not a flat sheet
The netting behind each goal SHALL be textured as a grid of cords rather than a flat translucent sheet, drawn thicker than a net's true scale and softened at the edges so that camera movement and ball deformation of the net mesh are visible as motion.

#### Scenario: The ball strikes the net
- **WHEN** the goal net deforms after a shot
- **THEN** the cord pattern on the net's surface shows the deformation moving, rather than an unchanging flat translucent plane

### Requirement: Turf is painted as a mown lawn with visible mowing stripes
The pitch surface SHALL be rendered as a dense sward of individually oriented grass blades — mostly laid flat by mowing with a minority standing upright and a speckle of pale cut tips — and SHALL show alternating light/dark banding where the mow direction reverses from strip to strip.

#### Scenario: A wide shot of the pitch is shown
- **WHEN** the camera frames the whole playing surface
- **THEN** alternating mowing stripes are visible across the pitch, and the turf reads as a uniform mown lawn rather than as a flat green fill or a scratched/carpet-like texture

### Requirement: The crowd is painted as individually distinguishable spectators
Stand seating SHALL show spectators wearing mostly neutral, desaturated coats, with the home club's colors worn only in blocks tied to allegiance and the visiting club's colors worn only in the away end; each spectator's head SHALL be textured as a full wrapped head (front, sides, and back) with the more detailed painting concentrated on the front-facing quarter, so faces remain legible at typical broadcast distance while the crowd still reads as one ground rather than a wall of identical bodies.

#### Scenario: The camera pans across the home end and the away end
- **WHEN** the broadcast camera shows both ends of the stadium
- **THEN** the home end shows spectators in mostly neutral coats with a share in the home club's colors, and the away end's visiting block shows spectators in the away club's colors, with no bright club colors scattered through the neutral blocks between them

### Requirement: Playback and event markers are drawn as simple vector icons
The transport controls (play, pause, altitude/lift arrows) and the match-timeline event markers (a goal ball, a missed-chance exclamation mark, a substitution double-arrow) SHALL each be rendered as small, antialiased flat-white icons on transparent backgrounds, distinct enough in silhouette from one another that they are not confused at icon size (e.g. the goal marker is not confused with the plain playhead disc).

#### Scenario: A goal is marked on the match timeline
- **WHEN** a goal event marker is placed on the timeline
- **THEN** it renders as a ball shape with panel detail cut out of it rather than a plain filled circle, so it is visually distinct from the timeline's playhead marker
