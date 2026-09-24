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
