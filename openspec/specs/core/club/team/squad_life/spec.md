# core/club/team/squad_life Specification

## Purpose
Squad life owns the ongoing membership state of a team's squad — captaincy, mentorship, chemistry, social standing and the monthly squad-status label each player carries — as the baseline other domains read rather than re-deriving.

## Requirements

### Requirement: Assigned squad status is a rank-derived baseline, never lowered by a player's own belief
The club SHALL assign each player a squad-status label (KeyPlayer / FirstTeamRegular / RotationPlayer / Backup / NotNeeded / etc.) derived from his current-ability rank within his own position group, refreshed on a monthly cadence, and this club-assigned label SHALL act only as a floor — a player's own belief about the playing-time share he deserves may exceed it but SHALL NOT be used to lower it.

#### Scenario: Ambitious player outperforms his backup label
- **WHEN** a player labelled as backup has banked a full season of starts on loan and carries high ambition
- **THEN** his personally expected start share is computed above the club's backup-derived baseline, while a player with low ambition and the same history expects no more than the club's baseline implies

### Requirement: A newcomer's arrival status comes from the monthly squad-status ranking
The club SHALL be able to project the squad status a prospective signing would hold on arrival. The projection ranks
him, at his ability and age, inside the main squad's position group alongside its current members, then applies the
club's level ceiling. This SHALL be the same ranking and the same ceiling the monthly squad-status pass uses. For a
signing whose standing has not changed since he arrived, the role the club offered and the label it later assigns
therefore cannot disagree.

#### Scenario: The projection matches the first monthly pass
- **WHEN** a player signs after being projected as a backup
- **AND** nothing about his position group changes before the next monthly pass
- **THEN** that pass assigns him the backup label he was projected

#### Scenario: A goalkeeper beyond the backup slots projects as not needed
- **WHEN** a goalkeeper would rank fourth in a group that already holds three goalkeepers
- **THEN** his projected arrival status is NotNeeded
