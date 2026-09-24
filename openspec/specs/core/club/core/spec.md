# core/club/core Specification

## Purpose
The club's own top-level `core` files (as distinct from its `academy`, `boardroom`, `squad`, and `treasury` children) hold the club's overall identity derivation and its per-tick orchestration entry point.

## Requirements

### Requirement: Club philosophy is derived from board youth focus and academy capability, not reputation
The club's trading philosophy (develop-and-sell, sign-to-compete, loan-focused, balanced) SHALL be derived from the board's youth-focus preference combined with whether the academy actually produces first-team-capable players (or is explicitly a player-trading academy), and SHALL NOT be inferred from club reputation alone.

#### Scenario: Small club with a strong academy and big club with a weak one
- **WHEN** a small club has an academy standard above the trading threshold and a youth-focused board, and a big club with equal reputation has a weak academy
- **THEN** the small club derives a develop-and-sell philosophy while the big club derives a buy/loan-oriented philosophy, despite their differing reputations
