# core/club/player/contract Specification

## Purpose
Owns a player's contract data model — squad-status classification against a club's level anchor, bonuses, clauses, wage-escalation triggers, and the agent lens used to weigh negotiation.

## Requirements

### Requirement: Contract proposals carry a full negotiable package
A player's contract SHALL be composed of salary, term, bonuses (signing, loyalty, appearance, goals, clean sheets, promotion/relegation), release and buy-back style clauses, wage-escalation triggers, and a promised squad role, held together as one negotiable package rather than salary alone.

#### Scenario: Club negotiates a long-term renewal for a first-team player
- **WHEN** the club sends a contract proposal that includes a loyalty bonus, a promised squad status and an appearance-based wage escalator
- **THEN** the resulting contract carries all offered terms together, not salary alone
