# core/club/team/squad/contract_renewal Specification

## Purpose
The club's contract-renewal pass decides which contracted players it offers fresh terms, when, and on what evidence.
A renewal is a verdict on a player the club has actually watched under his current deal.

## Requirements

### Requirement: A proactive renewal waits for evidence under the current deal
The club SHALL NOT make a proactive renewal offer on a contract until the player has served part of it, counted from
the contract's start. The required tenure SHALL be half of the contract's term, but never more than 180 days. It SHALL
apply whatever the squad-status threshold, Bosman pressure or final-month urgency would otherwise say. A contract with
no recorded start SHALL be treated as long-running. The expiry-day last-chance offer SHALL stay available regardless of
tenure.

#### Scenario: A one-year deal is not renewed the day after signing
- **WHEN** a player signed a one-year contract yesterday, and his squad status makes any contract with less than
  eighteen months left a renewal candidate
- **THEN** no renewal offer is made
- **AND** once 180 days have passed, if he still qualifies, the club may offer one

#### Scenario: A long-running contract is not delayed
- **WHEN** a player's contract started three years ago and has entered his renewal window
- **THEN** the tenure requirement does not delay the offer

#### Scenario: The expiry-day offer is unaffected
- **WHEN** a contract reaches its expiry day before the tenure requirement is met
- **THEN** the club's last-chance offer is still evaluated
