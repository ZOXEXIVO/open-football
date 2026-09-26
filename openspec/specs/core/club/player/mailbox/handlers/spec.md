# core/club/player/mailbox/handlers Specification

## Purpose
Owns how a player answers proposals sent to his mailbox — weighing a contract proposal's full package against the negotiator's skill and installing the resulting contract on acceptance.

## Requirements

### Requirement: A player's contract response weighs the negotiator's persuasive skill
The player's acceptance decision on a contract proposal SHALL factor in every offered term together (salary, bonuses, clauses, promised role) along with the staff member's negotiation skill, not salary alone.

#### Scenario: Club negotiates a long-term renewal for a first-team player
- **WHEN** the club sends a contract proposal that includes a loyalty bonus, a promised squad status and an appearance-based wage escalator
- **THEN** the player's acceptance decision factors in all offered terms together with the staff member's negotiation skill

### Requirement: An accepted renewal keeps the role the club promised
When a player accepts a contract renewal, three rules SHALL apply:
- An unexpired role promise on the contract being replaced SHALL carry over to the new contract with its original
  expiry.
- A renewal that promises a role above the player's current squad status SHALL bind that role as a promise, for the
  same period a signing promise binds. It replaces any lower promise that would otherwise carry over.
- A renewal that restates his current role SHALL NOT create a new binding promise.

#### Scenario: A promise survives an extension
- **WHEN** a player promised a first-team regular's role four months ago accepts an extension that states no role
- **THEN** the new contract carries the regular's promise until its original expiry
- **AND** the monthly squad-status pass cannot demote him below it before then

#### Scenario: A negotiated promotion binds
- **WHEN** a backup demands a rotation role as a renewal term and accepts the renewal that grants it
- **THEN** the rotation role binds as a floor on his squad status for one year from acceptance

#### Scenario: A restated role does not bind
- **WHEN** a key player accepts a renewal that restates his key role, and he holds no unexpired promise
- **THEN** the new contract carries no role promise
