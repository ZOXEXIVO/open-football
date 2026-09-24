# viewer/scene Specification

## Purpose
Captures the existing observable behaviour of the match replay viewer's stadium scene: the pitch surface, stands, crowd, goal netting, and sky, as built from a recorded match and a venue's fixture data.

## Requirements

### Requirement: Regulation pitch geometry
The viewer SHALL render a playing surface sized to a regulation pitch (105 x 68.125 metres) with markings (penalty areas, goal areas, centre circle, penalty spots, corner arcs) positioned to match the dimensions the match engine simulated on, so recorded player and ball positions line up with the drawn pitch and goal frame.

#### Scenario: Recorded positions land on the drawn pitch
- **WHEN** a recorded match position is converted from engine grid units to world space
- **THEN** the resulting point lies within a pitch 105 metres long and 68.125 metres wide, centred on the world origin, with the vertical axis left in metres unscaled

### Requirement: Graded playing surface condition
The viewer SHALL vary the pitch's visual condition (grass colour, mowing stripes, worn patches, marking-paint colour, perimeter floodlighting) along a single continuous "how well kept" scale derived from the venue, rather than switching between a fixed set of pitch presets.

#### Scenario: A well-kept ground shows a mown, tended pitch
- **WHEN** the venue's fixture indicates a well-supported, well-funded club
- **THEN** the rendered pitch shows a saturated green surface with visible mowing stripes, minimal goalmouth wear, and crisp white markings

#### Scenario: A modest ground shows a neglected pitch
- **WHEN** the venue's fixture indicates a small, poorly-supported club
- **THEN** the rendered pitch shows a paler, less saturated surface, faint or absent mowing stripes, more pronounced bare patches around goalmouths and the centre/penalty spots, and weathered, greyer markings

### Requirement: Worn and uneven turf detail
The viewer SHALL show localized wear on the pitch surface concentrated where play actually happens (the goalmouths, penalty spots, centre circle, wide touchline channels, and the middle of the pitch), and SHALL show slow, irregular colour variation across the turf rather than a uniform flat colour or a visibly repeating tile pattern.

#### Scenario: Goalmouths appear more worn than the wings
- **WHEN** the pitch surface is rendered at any upkeep level
- **THEN** the ground directly in front of each goal, the penalty spots, and the centre spot appear more discoloured/worn than the flanks of the pitch, and the amount of wear scales with how poorly kept the ground is

### Requirement: Stadium scaled to the fixture
The viewer SHALL build a stand (terracing) around the pitch whose height, footprint, and wraparound around the corners scale continuously with the hosting club's drawing power (its typical attendance and reputation), rather than from a small fixed set of stadium sizes.

#### Scenario: A big club's ground is taller and wraps further
- **WHEN** the venue belongs to a club with a large typical gate and high reputation
- **THEN** the stand is built with more rows of terracing, a taller crest, and wraps further around the corners of the pitch than a small club's ground

#### Scenario: A small or youth venue gets a minimal terrace
- **WHEN** the venue belongs to a very small or youth/academy side
- **THEN** the stand is built at the minimum row count the scene ever produces, rather than disappearing or growing arbitrarily thin

### Requirement: Stand occupancy reflects the specific fixture
The viewer SHALL fill a variable share of the seating with spectators, where that share reflects both the home club's typical attendance and how appealing this particular fixture is (a big away side draws a fuller house than a routine midweek match against a lesser opponent), always leaving the stand somewhere between clearly not-full and clearly not-empty.

#### Scenario: A visit from a prestigious side fills more seats
- **WHEN** the visiting side has notably higher reputation than the home side
- **THEN** the rendered crowd occupies a larger share of the stand than it would for a visit from a lower-reputation side, up to a near-capacity but never completely full house

#### Scenario: An academy fixture is sparsely attended
- **WHEN** the venue is flagged as a youth venue
- **THEN** the rendered crowd is sparse regardless of the parent club's usual attendance figures

### Requirement: Crowd seating is visually uneven, not uniform
The viewer SHALL distribute spectators unevenly across a stand: massed most densely in the area directly behind the goal ends, thinning toward the corners and along the touchline stands, and clumped in patches rather than spread as an even, evenly-spaced sprinkle across every seat.

#### Scenario: The area behind the goal reads as the densest part of the crowd
- **WHEN** a stand behind a goal is rendered
- **THEN** the section directly behind the goal is visibly denser with spectators than the far corners of the same stand

#### Scenario: End stands and side stands wear different colours
- **WHEN** the crowd is rendered in an end stand versus a touchline (side) stand
- **THEN** the end stand shows a noticeably higher proportion of spectators in the home or away club's colours than the touchline stand, which is mostly neutral coats

### Requirement: Goal netting deforms with the ball, only inside the goal
The viewer SHALL visually bulge the goal netting outward at the point the ball is pressing on it whenever the recorded ball position is inside the goal volume, continue a brief decaying wobble after the ball stops pushing, and leave the netting completely undisturbed for any ball position that is not inside the goal (including shots that miss wide or fly over the crossbar).

#### Scenario: A ball settling in the net bulges the mesh around it
- **WHEN** the recorded ball comes to rest inside the back of the goal netting
- **THEN** the netting mesh is displaced outward around that contact point by an amount consistent with how far the ball has pushed past the panel, and the displacement fades to flat after the ball stops moving

#### Scenario: A shot that misses wide leaves the net still
- **WHEN** the recorded ball crosses the goal-line plane outside the width of the goalposts, or passes above the crossbar
- **THEN** none of the goal netting panels show any deformation

### Requirement: Sky forms a seamless backdrop that follows the camera
The viewer SHALL render a continuous sky gradient surrounding the stadium that recentres on the camera as the camera moves, so the horizon never appears to slide relative to the camera and the stands never appear to run out into empty space at their back edges.

#### Scenario: Flying the camera down the touchline keeps the horizon steady
- **WHEN** the replay camera translates across the pitch during playback
- **THEN** the sky backdrop is repositioned to stay centred on the camera each frame, so the horizon remains at a constant apparent height rather than sliding past
