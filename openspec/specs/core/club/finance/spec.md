# core/club/finance Specification

## Purpose
The finance capability owns a club's ledger and money model: continuous broadcast/matchday/commercial revenue, the sponsorship book, balance history, and the debt-standing classification with the interventions (facility, owner injection, embargo, administration) each standing carries.

## Requirements

### Requirement: Club revenue responds continuously to standing, not by reputation tier
Broadcast, matchday and commercial revenue SHALL be computed as continuous functions of league position/tier, stadium capacity/utilisation, and reputation, so a small reputation change never produces a cliff-edge change in income.

#### Scenario: Club reputation drops by a small margin
- **WHEN** a club's blended reputation score decreases slightly without crossing a division boundary
- **THEN** broadcast income (a shared league pool) is unaffected by the reputation change and matchday/commercial income shift smoothly rather than jumping between fixed tiers

### Requirement: Debt escalates through named standings with real interventions
A club's finances SHALL classify outstanding debt into an ordered standing (solvent, leveraged, owner-funded, emergency, administration) from its balance against an overdraft facility sized off trailing revenue, and each standing SHALL determine concrete consequences (transfer spending blocked, a wage-ratio ceiling, points deduction and embargo) rather than compounding interest indefinitely.

#### Scenario: Club debt exceeds its facility and the owner won't fund it
- **WHEN** a club's debt goes past its agreed overdraft facility and no owner injection follows
- **THEN** the club enters emergency trading measures (no transfer spending, a tighter wage-ratio ceiling) before any move to administration, and entering administration writes the unpayable balance down to something serviceable in exchange for a points deduction and a dated embargo

### Requirement: The season's transfer money remembers what was sold and spent
A club's available transfer money SHALL at every moment be what the board set for the season, moved by everything
that has happened since: the board's own grants and cuts, the share of each sale the club recycles, and every fee the
club has committed. A periodic recompute of budgets SHALL arrive at the same figure, so that neither a recycled sale
nor a fee already spent is undone by the turn of a month. Financial distress and an embargo SHALL still throttle the
figure, as a factor applied to the whole.

The share of a sale that is recycled SHALL remain the club's own, by its philosophy.

#### Scenario: A sale at the end of July
- **WHEN** a club recycles 40M from a sale on 28 July and spends nothing
- **THEN** its available transfer money on 2 August is still 40M higher than before the sale

#### Scenario: A purchase in early August
- **WHEN** a club spends 25M on 5 August
- **THEN** its available transfer money on 2 September is still 25M lower

#### Scenario: A new season
- **WHEN** the board sets the next season's budget
- **THEN** the money recycled and spent in the season before no longer moves it
