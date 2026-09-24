# core/league/table Specification

## Purpose
Owns the league table's row model and ranking: deterministic tie-break ordering and the separation between earned points and sanction deductions.

## Requirements

### Requirement: League table computation and tie-breaking
The league table SHALL rank teams by points, then by a configurable, deterministic tie-break chain (goal difference, goals scored, wins, then team id) so that any given set of match results always sorts to the same order regardless of insertion order.

#### Scenario: Two teams level on points, goal difference, and goals scored
- **WHEN** two teams are equal through every football-based tie-break criterion
- **THEN** the team with the numerically smaller id is ranked ahead, guaranteeing a total order

### Requirement: Points deductions are tracked separately from earned points
A league table row SHALL track earned points and any points deduction (from disciplinary or financial fair play sanctions) as separate values, and standings SHALL use the effective points (earned minus deduction, floored at zero) for ranking.

#### Scenario: Club sanctioned mid-season
- **WHEN** a financial fair play case resolves in a points deduction against a club
- **THEN** the club's earned points figure is unchanged, its recorded deduction total increases, and the table re-sorts using the reduced effective points
