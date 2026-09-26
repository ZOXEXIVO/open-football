# core/club/player/happiness Specification

## Purpose
Beyond the `types/*` children, this directory's own files derive a player's
own career expectation and compute the weekly morale-factor processing
(playing-time frustration, club fit, coach credibility) that feeds his mood.

## Requirements

### Requirement: A player's career expectation can diverge from his assigned squad status
A player SHALL form his own belief about the playing-time share he deserves from his club-assigned squad label, his level-adjusted match-experience history, and his ambition — and this belief SHALL only ever raise his expected share above the club's own label, never lower it.

#### Scenario: Ambitious player outperforms his backup label
- **WHEN** a player labelled as backup has banked a full season of starts on loan and carries high ambition
- **THEN** his personally expected start share is computed above the club's backup-derived baseline, while a player with low ambition and the same history expects no more than the club's baseline implies

### Requirement: The playing-time morale factor shares the complaint gate
The weekly playing-time morale factor SHALL read neutral whenever the match-opportunity gate says the player's
minutes cannot be judged yet. That covers four cases:
- no eligible official matches since he joined;
- still inside the post-transfer hard grace window;
- below his squad status's minimum match sample;
- labelled NotNeeded.

This SHALL be the same gate that governs playing-time complaints, playing-time events and broken playing-time
promises.

#### Scenario: A frozen-out player is neutral about minutes
- **WHEN** a player labelled NotNeeded has sat out twenty eligible official matches
- **THEN** his playing-time factor is zero
- **AND** his wish to leave is carried by other signals, not by a minutes grievance

#### Scenario: A backup below his sample is neutral
- **WHEN** a backup has had fewer eligible official matches than a backup's minimum match sample
- **THEN** his playing-time factor is zero

### Requirement: Playing-time frustration grows with the matches a player is owed
When a player's involvement falls short of the share he expects, his frustration SHALL grow with the number of
matches he is owed. Matches owed are his expected start share multiplied by the eligible matches, less his weighted
involvement. Every squad status SHALL share one patience curve:
- a few owed matches are shrugged off;
- beyond that, frustration builds;
- it saturates at the factor's floor.

A squad status SHALL affect frustration only through the share it leads the player to expect, never through a
status-specific severity.

#### Scenario: A benched key player complains
- **WHEN** a key player past the grace window has been left out of all ten eligible official matches
- **THEN** his playing-time factor is at or below the playing-time complaint threshold

#### Scenario: An unused backup is restless, not aggrieved, early on
- **WHEN** a mid-career backup at full ability weighting has had no involvement in twenty eligible official matches
- **THEN** his playing-time factor is negative but above the major-concern band
- **AND** it is clearly milder than the benched key player's

#### Scenario: A barren season hardens a backup's grievance
- **WHEN** a young, ambitious backup has had no involvement across a full season of eligible official matches
- **THEN** his playing-time factor reaches the playing-time complaint threshold

#### Scenario: The same deficit brings the same frustration
- **WHEN** two players with the same ability weighting and career stake are owed the same number of matches under
  different squad statuses
- **THEN** their playing-time factors are equal

### Requirement: How much minutes matter scales with career stake
The magnitude of playing-time frustration SHALL scale continuously with the player's career stake. The stake rises
with the prime years he has left and with his ambition. It falls as his career winds down, with no age step. A veteran
SHALL still feel some frustration when he is owed matches.

#### Scenario: A young ambitious player against a veteran
- **WHEN** an ambitious 23-year-old and a 35-year-old are owed the same number of matches under the same squad
  status
- **THEN** the younger player's frustration is the larger
- **AND** the veteran's playing-time factor is still negative

#### Scenario: No birthday cliff
- **WHEN** a player turns 31
- **THEN** his career stake, and so his frustration for the same owed matches, changes by no more than the continuous
  drift of one day
