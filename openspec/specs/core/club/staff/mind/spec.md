# core/club/staff/mind Specification

## Purpose
Beyond the `organs/judgements/` child, this directory's own files implement
the manager's mind faculties (ambition, authority, integration, judgement,
football philosophy/conviction, situational read, the submind contract, and
welfare) that feed his mood and his opinions on staff decisions.

## Requirements

### Requirement: Player mind faculties report silence, not false zero, when they have no view
Both the player's and staff's internal mind faculties SHALL report a mood contribution paired with a confidence value, and a faculty with nothing to go on SHALL report zero confidence (silent) rather than a confident neutral value.

#### Scenario: A faculty has no relevant memory to draw on
- **WHEN** a mood faculty is queried for a domain it has no evidence about
- **THEN** it returns a silent contribution (confidence zero) which downstream aggregation treats differently from an actual neutral verdict
