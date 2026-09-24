# core/club/core/treasury Specification

## Purpose
The treasury capability is the club spending and earning on its own ledger month by month: billing wages and lump-sum bonuses, booking revenue, servicing and resolving debt through the escalation ladder the finance model defines, and shedding wages by selling players once a club's debt standing forces it to.

## Requirements

### Requirement: Debt escalates through named standings with real interventions
When a club's debt standing forces wage reduction, the treasury SHALL identify who may be sold to relieve it through an escalating ladder — least essential first, biggest earner first — whose severity scales with the debt standing and cash-distress level, rather than a single fixed protection list applied at every severity.

#### Scenario: A club moves from over-budget to a genuine fire sale
- **WHEN** a club's severity climbs from merely over its wage mandate to being in emergency trading measures or administration
- **THEN** the set of players eligible for a financially-motivated sale escalates from fringe/rotation depth only, to first-team players, to any player including the captain, and a genuine fire sale overrides the normal patience window on recently-signed players
