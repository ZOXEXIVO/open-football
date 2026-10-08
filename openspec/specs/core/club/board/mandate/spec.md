# core/club/board/mandate Specification

## Purpose
The signing mandate is the board's statement of what a club bought a player for, and the minutes that implies. It
travels from the purchase to the player's plan, and every later squad decision reads it. This capability guarantees
that every permanent arrival carries one, whoever made the move.

## Requirements

### Requirement: Every permanent arrival carries its buying board's mandate and a fresh plan

Every permanent arrival at a club SHALL carry a signing mandate from the buying club's board, and a plan made from that
mandate dated the day of the arrival. Permanent arrivals are:

- a negotiated transfer;
- a free-agent signing;
- a loan option or obligation bought out;
- a transfer an editor makes by hand.

A plan SHALL belong to the club that owns the player. A permanent move SHALL replace the previous owner's plan and SHALL
NOT carry it across. A loan SHALL leave the owning club's plan where it is.

While the new plan's evaluation window runs, the club's automatic squad sweeps SHALL treat the arrival as a protected
signing. That holds whichever path made the move.

#### Scenario: An editor's move arrives with the buying club's plan
- **WHEN** an editor moves a player by hand from one club to another
- **THEN** the player carries a mandate from the buying club's board and a plan dated that day
- **AND** no plan written by the selling club survives the move

#### Scenario: A hand-moved signing is not trimmed as surplus
- **WHEN** the buying club's weekly squad rebalance runs inside that plan's evaluation window, with the player ranked
  past the depth cap for that position group
- **THEN** the player is not chosen as surplus, and no loan is staged

#### Scenario: A buyout arrives with a plan of its own
- **WHEN** a borrower buys out a loanee through an option or an obligation
- **THEN** the player carries the borrower's mandate and a plan dated the day of the buyout

#### Scenario: A loan leaves the owner's plan in place
- **WHEN** a club loans a player out
- **THEN** the player still carries the owning club's plan, unchanged

### Requirement: A move nobody negotiated takes its purpose from the role it gives the player

When no hearing set the purpose (an editor's move, a buyout, a free agent walking in), the mandate's purpose SHALL be
read from the squad role the move gives the player:

- a key player or first-team regular is a starter;
- a rotation player is rotation;
- a prospect or youngster label is a prospect;
- with no role in the side, a player with enough career ahead and an ability below the club's first-team band is a
  prospect, and anyone else is cover.

When a board hears a loan option, the purpose it hears SHALL be the purpose the plan will carry if the option is bought.

#### Scenario: A first-team regular moved by hand is a starter
- **WHEN** an editor moves a player into a squad whose depth makes the player a first-team regular
- **THEN** the mandate's purpose is starter

#### Scenario: A body for the bench is cover
- **WHEN** a 29-year-old arrives through a move nobody negotiated, with no role in the side and an ability below the
  club's first-team band
- **THEN** the mandate's purpose is cover

#### Scenario: The hearing and the plan agree
- **WHEN** a borrower's board hears an option on a loanee who was its rotation player, and then buys the player
- **THEN** the purpose the board heard and the purpose the plan carries are both rotation
