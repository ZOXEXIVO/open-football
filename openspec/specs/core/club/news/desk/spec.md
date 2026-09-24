# core/club/news/desk Specification

## Purpose
The news desks each read a slice of club state and file candidate stories — match reports, squad and dugout beats, transfer business, boardroom and balance-sheet news — for the editor to select from, reading institution-level facts from the club's own dated diary rather than inferring them from before/after state.

## Requirements

### Requirement: The club press reads institution-level events from the dated diary, not inferred state
The news desks SHALL read institution-level events (manager dismissals/appointments, severance paid, takeovers, facility changes) only from the club's dated affair diary, rather than inferring them from before/after state comparisons.

#### Scenario: Manager sacked and a caretaker steps up, then a permanent replacement is appointed a month later
- **WHEN** the head-coach identity changes twice within a month for two different reasons
- **THEN** the board desk files a distinct story for the sacking, the caretaker appointment and the later permanent appointment, reading each from its own diary entry instead of conflating the two identity changes into one story
