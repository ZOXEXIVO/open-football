# core/club/player/core Specification

## Purpose
Owns the player's own coordination surface — the `Player` struct, its builder/collection/context and the cross-faculty orchestration that does not belong to any single narrower player subfolder.

## Requirements

### Requirement: A player's career expectation can diverge from his assigned squad status
A player SHALL form his own belief about the playing-time share he deserves from his club-assigned squad label, his level-adjusted match-experience history, and his ambition — and this belief SHALL only ever raise his expected share above the club's own label, never lower it.

#### Scenario: Ambitious player outperforms his backup label
- **WHEN** a player labelled as backup has banked a full season of starts on loan and carries high ambition
- **THEN** his personally expected start share is computed above the club's backup-derived baseline, while a player with low ambition and the same history expects no more than the club's baseline implies
