# core/club/board/sale Specification

## Purpose
The sale capability turns a board demand for money into a concrete, trackable action against a named player rather than an unresolvable general order.

## Requirements

### Requirement: Forced sale demands name a specific player and price
When the board demands the club raise money by selling, it SHALL identify a specific player — the highest earner the club can afford to lose — and an asking price with a deadline, rather than issuing an unresolvable general demand.

#### Scenario: Board mandates a sale during financial distress
- **WHEN** the board issues a forced-sale demand
- **THEN** a target player (age 27+, generally the club's highest earner it can part with) and an asking price are attached to the demand, and the demand is only satisfied once that much money has come in before the deadline
