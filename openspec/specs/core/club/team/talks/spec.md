# core/club/team/talks Specification

## Purpose
Team talks let the manager deliver a pre-match, half-time or full-time speech in a chosen tone, translating that tone into morale effects on the squad.

## Requirements

### Requirement: Team talks apply personality-weighted morale effects
The club SHALL let its manager deliver a team talk with one of several tones (praise, criticise, encourage, passionate, silent), and the morale effect on each player SHALL depend on both the tone chosen and that player's own mental-strength/personality profile.

#### Scenario: Same criticising team talk given to two different personalities
- **WHEN** a critical team talk is delivered before a match
- **THEN** a mentally resilient player responds well while a nervy player's morale is harmed, from the same talk

### Requirement: The half-time morale event is the talk given in the match
The half-time team talk's morale event SHALL be the talk the coach gave at the interval of that match: the tone recorded with the result, delivered to the whole matchday squad that was in the dressing room. It SHALL NOT be a talk reconstructed afterwards from the half-time score. The pre-match and full-time talks SHALL keep their current rule.

#### Scenario: The recorded tone is the one applied
- **WHEN** a match result records that a side's coach gave a critical half-time talk
- **THEN** the side's half-time morale events use the critical tone, whatever tone the half-time score alone would have suggested

#### Scenario: The whole dressing room heard it
- **WHEN** a match result records a side's half-time talk
- **THEN** every player of that side's matchday squad receives the half-time morale event: those who started, those who came on, and those who stayed on the bench
