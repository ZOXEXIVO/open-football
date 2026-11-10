# core/club/player/mind Specification

## Purpose
Owns the player's internal mind faculties — the sub-mind contract and the career/competitive/financial/professional/social organs that read his situation and form his mood, goals and plans.

## Requirements

### Requirement: Player mind faculties report silence, not false zero, when they have no view
A player's internal mind faculties SHALL report a mood contribution paired with a confidence value, and a faculty with nothing to go on SHALL report zero confidence (silent) rather than a confident neutral value.

#### Scenario: A faculty has no relevant memory to draw on
- **WHEN** a mood faculty is queried for a domain it has no evidence about
- **THEN** it returns a silent contribution (confidence zero) which downstream aggregation treats differently from an actual neutral verdict

### Requirement: A career plan follows from his anchors and outlasts a window

A player's career plan SHALL follow from his dream and his summit before it follows from this week's situation. A plan
SHALL NOT end because a transfer window closed with nothing done, or because a week went badly. It SHALL end when what
it was for has happened, when the facts that formed it have gone for long enough that it fades, or when the dream it
serves has been revised.

- A man reaching for a level above where he plays SHALL plan to step up once he has settled at his club, without
  waiting two seasons, and his conviction SHALL set how hard he holds the plan.
- A plan to step up or to go out on loan SHALL be judged the day after the country's main window, the longer of its
  two, has shut, by the whole of that market. A main window closed with no move SHALL keep the plan, count the attempt,
  and be something the dream's next review weighs. A shorter window in between SHALL count no attempt.
- A man who has arrived SHALL NOT plan to step up.
- A plan to step up SHALL end once his dream no longer reaches above his club, except that a returnee whose claim on a
  place went unanswered SHALL keep it until he plays.
- A plan to settle at his level SHALL form only for a man who has arrived, or whose dream asks for no more.
- A plan to finish at home SHALL form from that dream, and from a late career abroad as it does today.
- A plan to stay and lead SHALL form from a dream to belong here, and from long service as it does today.

The arcs that answer not playing (breaking through, going out on loan, stepping down to play) SHALL keep forming from
his situation. Each SHALL leave his dream unchanged.

#### Scenario: A window closes on a plan to step up

- **WHEN** a player planning to step up reaches the end of the main window with no move
- **THEN** he still plans to step up, with one more attempt counted

#### Scenario: A short window is not the attempt

- **WHEN** a player at a club on the European calendar plans in October to go out on loan, and January passes without one
- **THEN** his plan stands until the summer window has shut, with no attempt counted

#### Scenario: A plan to step up lasts

- **WHEN** a world of ambitious players at clubs below their dream plays several seasons
- **THEN** the median plan to step up lasts at least a season

#### Scenario: Nobody at the top wants a bigger club

- **WHEN** a regular has arrived at the level his dream names
- **THEN** he forms no plan to step up, however long he has been at the club

#### Scenario: Arrived and out of the side, he does not plan to step up

- **WHEN** a player who has arrived loses his place and his claim to it fails
- **THEN** he holds no plan to step up

#### Scenario: A returnee passed over keeps wanting a move

- **WHEN** a returnee whose claim on a place went unanswered plans to step up, with a dream that names no level
- **THEN** he keeps the plan until he plays

#### Scenario: A step up his dream no longer asks for ends

- **WHEN** a benched player who is not such a returnee holds a plan to step up that his dream no longer reaches for
- **THEN** the plan ends

#### Scenario: Not playing does not end a dream

- **WHEN** a 20-year-old reaching for the top goes out on loan to play
- **THEN** his plan is the loan and his dream is unchanged
