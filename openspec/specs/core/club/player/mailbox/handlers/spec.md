# core/club/player/mailbox/handlers Specification

## Purpose
Owns how a player answers proposals sent to his mailbox — weighing a contract proposal's full package against the negotiator's skill and installing the resulting contract on acceptance.

## Requirements

### Requirement: A player's contract response weighs the negotiator's persuasive skill
The player's acceptance decision on a contract proposal SHALL factor in every offered term together (salary, bonuses, clauses, promised role) along with the staff member's negotiation skill, not salary alone.

#### Scenario: Club negotiates a long-term renewal for a first-team player
- **WHEN** the club sends a contract proposal that includes a loyalty bonus, a promised squad status and an appearance-based wage escalator
- **THEN** the player's acceptance decision factors in all offered terms together with the staff member's negotiation skill
