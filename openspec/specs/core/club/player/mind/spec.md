# core/club/player/mind Specification

## Purpose
Owns the player's internal mind faculties — the sub-mind contract and the career/competitive/financial/professional/social organs that read his situation and form his mood, goals and plans.

## Requirements

### Requirement: Player mind faculties report silence, not false zero, when they have no view
A player's internal mind faculties SHALL report a mood contribution paired with a confidence value, and a faculty with nothing to go on SHALL report zero confidence (silent) rather than a confident neutral value.

#### Scenario: A faculty has no relevant memory to draw on
- **WHEN** a mood faculty is queried for a domain it has no evidence about
- **THEN** it returns a silent contribution (confidence zero) which downstream aggregation treats differently from an actual neutral verdict
