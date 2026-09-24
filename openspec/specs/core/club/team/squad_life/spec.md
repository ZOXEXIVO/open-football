# core/club/team/squad_life Specification

## Purpose
Squad life owns the ongoing membership state of a team's squad — captaincy, mentorship, chemistry, social standing and the monthly squad-status label each player carries — as the baseline other domains read rather than re-deriving.

## Requirements

### Requirement: Assigned squad status is a rank-derived baseline, never lowered by a player's own belief
The club SHALL assign each player a squad-status label (KeyPlayer / FirstTeamRegular / RotationPlayer / Backup / NotNeeded / etc.) derived from his current-ability rank within his own position group, refreshed on a monthly cadence, and this club-assigned label SHALL act only as a floor — a player's own belief about the playing-time share he deserves may exceed it but SHALL NOT be used to lower it.

#### Scenario: Ambitious player outperforms his backup label
- **WHEN** a player labelled as backup has banked a full season of starts on loan and carries high ambition
- **THEN** his personally expected start share is computed above the club's backup-derived baseline, while a player with low ambition and the same history expects no more than the club's baseline implies
