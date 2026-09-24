# core/club/staff/mind/organs/judgements Specification

## Purpose
The judgements organ capability holds a coach's persistent, revisable, scorable
opinions of the players he assesses, distinct from and complementary to a player's
own mind organs.

## Requirements

### Requirement: A coach's judgement of a player reports silence, not false zero, when he has no view
A coach's judgement organ SHALL report how sure he is of a player paired with a confidence value, and SHALL report zero confidence (silent) rather than a confident neutral value when he holds no view of that player at all.

#### Scenario: A coach is asked about a player he has never judged
- **WHEN** the judgement store is queried for a player it holds no `PlayerJudgement` for
- **THEN** it returns a silent confidence of zero, which a caller must treat as "no view" rather than as an actual neutral verdict
