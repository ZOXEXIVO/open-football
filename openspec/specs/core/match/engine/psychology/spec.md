# core/match/engine/psychology Specification

## Purpose
Tracks each player's in-match psychological state — composure, confidence, momentum — and feeds it back into that player's on-pitch execution.

## Requirements

### Requirement: In-match player psychology
A player's in-match psychological state (such as composure and confidence) SHALL be able to shift based on match events, and SHALL feed back into that player's effective execution of skill-dependent actions (e.g. passing accuracy) as a bounded modifier rather than an unbounded swing.

#### Scenario: Low composure reduces pass accuracy
- **WHEN** a player's psychological state reflects reduced composure and first-touch reliability
- **THEN** that player's evaluated pass success probability is nudged downward relative to a neutral psychological state, within a small bounded range
