# core/club/team/behaviour Specification

## Purpose
Defines the coaching staff's individual training plans: which player gets which plan, how a plan progresses, and what
it may change. Attribute growth stays with the training ground and the development tick.

## Requirements

### Requirement: Individual plans never write attributes directly

An individual training plan's monthly progression SHALL NOT change any technical, mental, physical or goalkeeping
attribute. A specialty-skill plan's effect SHALL come only from the extra reps the daily training sessions give its
skill, under the same ceilings and potential as every other gain.

#### Scenario: A lagging skill under a specialty plan
- **WHEN** a specialty plan for a lagging skill progresses through five monthly passes
- **THEN** that skill and the player's technical average are unchanged by those passes

### Requirement: A specialty plan ends when its skill catches up or its block runs out

A specialty-skill plan SHALL end in either of two cases:

- the skill reaches the player's technical average minus half the lag that made him eligible for the plan
- the plan has run for 180 days

#### Scenario: The skill has caught up
- **WHEN** a player's planned skill is already within half the eligibility lag of his technical average
- **THEN** the plan ends at its next progression

#### Scenario: The block runs its course
- **WHEN** a specialty plan's skill is still lagging after 180 days
- **THEN** the plan ends
