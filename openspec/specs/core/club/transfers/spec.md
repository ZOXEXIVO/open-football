# core/club/transfers Specification

## Purpose
The club's transfers capability owns its recruitment identity and its offer construction: what kind of players it wants, how hard it negotiates, and the full personal-terms package it puts in front of a target, all derived from board vision and financial stance.

## Requirements

### Requirement: Recruitment identity is composed of four independent policies
The club's transfer recruitment behaviour SHALL be governed by four separate, independently configurable policies — what to sign (philosophy/stance/preference/age), how hard to negotiate, how much to protect resale value, and domestic bias — derived from board vision and financial stance rather than a single scalar aggressiveness value.

#### Scenario: Two clubs share a financial stance but different signing preferences
- **WHEN** two clubs both run a balanced financial stance but one board prefers domestic youth and the other prefers proven experience
- **THEN** their recruitment policies diverge on age preference and domestic bias while their negotiation discipline can remain aligned, because the policies are set independently

### Requirement: Contract proposals carry a full negotiable package
The club SHALL be able to offer a player a contract proposal composed of salary, term, bonuses (signing, appearance, goals, promotion/relegation), release clauses and installment/sell-on terms, and a promised squad role, built from the club's own recruitment and negotiation policy rather than a single flat figure.

#### Scenario: Club negotiates a permanent signing for a first-team role
- **WHEN** the club's transfer strategy builds an offer for a target it intends to promise a first-team role
- **THEN** the resulting personal-terms package prices the wage against that promised status, and attaches signing bonus, agent fee, release-clause and installment terms according to the club's own financial stance and negotiation policy rather than a value computed independently of what was promised
