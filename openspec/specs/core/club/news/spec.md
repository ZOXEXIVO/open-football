# core/club/news Specification

## Purpose
The club's own news files hold the dated affair diary that records institution-level events as they happen, plus the desk taxonomy and editor that select which desk's stories run.

## Requirements

### Requirement: Institution-level events are recorded as dated diary entries at the moment they happen
The club SHALL record institution-level events (manager dismissals/appointments, severance paid, takeovers, facility changes) as dated diary entries in a bounded log at the moment they happen, rather than requiring any reader to infer them from before/after state comparisons.

#### Scenario: Manager sacked and a caretaker steps up, then a permanent replacement is appointed a month later
- **WHEN** the head-coach identity changes twice within a month for two different reasons
- **THEN** the diary carries distinct entries for the sacking, the caretaker appointment and the later permanent appointment, so a reader can tell each apart instead of the two identity changes being conflated into one story
