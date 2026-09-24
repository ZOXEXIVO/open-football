# web/face Specification

## Purpose
Capture the existing behavior of the procedural player face/avatar generation system: what player data drives it, what visual features it covers, how it guarantees consistency, and how it is served over HTTP.

## Requirements

### Requirement: Deterministic per-player appearance
The system SHALL derive a player's generated visual appearance entirely from the player's identifier and record data, such that the same player id and inputs always produce the identical rendered image.

#### Scenario: Repeated requests for the same player
- **WHEN** a portrait is generated twice for the same player id with the same age, ancestry distribution, build, aggression, and jersey color inputs
- **THEN** the two generated images are byte-for-byte identical

#### Scenario: Different players produce different appearances
- **WHEN** portraits are generated for different player ids
- **THEN** the sampled visual traits (hairstyle, eye shape, face shape, beard, skeletal proportions, etc.) vary independently between players rather than repeating a fixed set of templates

### Requirement: Appearance driven by player attributes
The system SHALL vary the generated appearance according to the player's age, nationality-derived ancestry mix, body build, and personality-derived expression, in addition to the per-player random identity.

#### Scenario: Age affects presentation
- **WHEN** a player's age is supplied to the generator
- **THEN** age-appropriate traits are reflected in the output, including that young players (23 and under) are never shown bald or with a receding hairline, and that grey hair and denser facial hair only appear as age increases

#### Scenario: Body build affects facial fullness
- **WHEN** a player's height and weight imply a build heavier or leaner than an athletic reference weight for that height
- **THEN** the generated face reflects that deviation (fuller or leaner cheeks, jaw, and neck) rather than always rendering an average build

#### Scenario: Personality affects expression
- **WHEN** a player has low temperament and/or high dirtiness attributes
- **THEN** the generated facial expression reads as harder/more aggressive (lowered, drawn brows, heavier lids, tighter mouth) than a player with calmer, cleaner attributes

#### Scenario: Nationality affects ancestry-linked traits
- **WHEN** a player's nationality maps to a known ancestry distribution for that country
- **THEN** the generator's skin tone band, hair/eye color palette, eye-shape family, and facial-feature proportions are sampled consistent with that distribution; when no distribution is known for the player's country, a mixed default distribution is used instead of assigning a false nationality-specific look

### Requirement: Covers a defined set of visual features
The system SHALL render a complete head portrait composed of a consistent set of facial and body features for every player.

#### Scenario: Full feature set present
- **WHEN** a portrait is generated for any player
- **THEN** the output includes skin-toned head and neck shading, ears, eyes (including iris, pupil, lids, and catchlights), eyebrows, nose, mouth, scalp hair (or an appropriate bald/receding treatment), and, when applicable for the player's age and ancestry class, facial hair (beard and/or moustache)

#### Scenario: Facial hair presence follows age and ancestry
- **WHEN** a player is under 20 years old
- **THEN** no beard or moustache is drawn, and growth likelihood and style otherwise scale with age and the player's ancestry-linked growth density

### Requirement: Two renderable presentation frames
The system SHALL support generating the same underlying head in at least two distinct presentation frames: a full studio portrait with background, shoulders, and jersey, and a head-only cutout on a transparent background with no studio elements.

#### Scenario: Portrait frame includes studio elements
- **WHEN** the portrait frame is requested
- **THEN** the rendered image includes a background card, shoulders rendered in a club or fallback jersey color, and a vignette

#### Scenario: Cutout frame omits studio elements and preserves the head
- **WHEN** the cutout frame is requested for the same player and inputs as a portrait
- **THEN** the rendered image contains no background card, no shoulders/jersey, and no vignette, while the head itself (shape, tone, and features) is unchanged from the portrait rendering of the same player

### Requirement: Jersey color reflects club affiliation
The system SHALL color the shoulders/jersey shown in the portrait frame using the player's current club's colors when the player is affiliated with a club, and SHALL fall back to a deterministic per-player color when no club affiliation is known.

#### Scenario: Player with a club
- **WHEN** a player is found to be registered to a club with defined colors
- **THEN** the jersey drawn in the portrait uses that club's color

#### Scenario: Player without a club
- **WHEN** a player has no resolvable club affiliation (e.g. a free agent)
- **THEN** the jersey uses a color derived deterministically from the player's id rather than a random or unset color

### Requirement: Served as an HTTP image resource
The system SHALL expose the generated portrait as an image over an HTTP route keyed by player id, returned as an SVG image, and cacheable long-term.

#### Scenario: Requesting a known player's face image
- **WHEN** a client requests the face image route for an existing player id
- **THEN** the response is served with an SVG image content type and a long-lived, immutable cache-control header

#### Scenario: Requesting an unknown player
- **WHEN** a client requests the face image route for a player id that does not exist in the current simulation data
- **THEN** the system responds with a not-found result rather than a generated image

#### Scenario: Requesting the cutout variant
- **WHEN** a client requests the face image route with a cutout query flag set
- **THEN** the head-only cutout frame is returned instead of the default full portrait frame
