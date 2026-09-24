# core/country/rules Specification

## Purpose
Owns the declarative squad-registration rulebook (`CountryRegulations`): which countries get nationality-based foreign-player and homegrown limits, and the pure decision helpers (who gets omitted, whether a wage bill breaches a cap) that other modules consume without themselves knowing the rule.

## Requirements

### Requirement: Country-level foreign-player and homegrown regulations are nationality-based and country-specific

`CountryRegulations` SHALL express only rules this model can represent honestly: nationality-based (passport, not club-trained) foreign-player registration limits for the specific countries whose real rule counts passports (Turkey, Russia, Ukraine, Mexico, Brazil, Saudi Arabia, UAE, US/Canada, China, Japan, South Korea, Qatar, Iran), and a homegrown minimum for England; countries whose real rule is EU/non-EU or club-trained-based (Spain, Italy, Germany, France, etc.) SHALL receive no foreign-player limit from this model.

#### Scenario: A country code known to count passports is queried
- **WHEN** regulations are built for country code "tr" or "TUR"
- **THEN** the foreign player limit is 14, regardless of code case or length

#### Scenario: A country whose real quota is EU-based is queried
- **WHEN** regulations are built for "es", "it", "de", or "fr" (or their 3-letter equivalents)
- **THEN** no foreign player limit is set for that country

#### Scenario: England is queried
- **WHEN** regulations are built for "eng"
- **THEN** the homegrown requirement is 8 and no foreign player limit is set

### Requirement: Foreign-player limit enforcement omits the weakest foreign players first

When a squad exceeds its country's foreign-player limit, the players omitted from registration SHALL be the excess foreign players with the lowest current ability, with ties broken by descending player id for determinism; domestic players (matching the club's country) are never omitted by this rule.

#### Scenario: A squad has 4 foreign players and a limit of 2
- **WHEN** foreign-limit enforcement runs with abilities 50, 80, 120, and 160 among the foreigners
- **THEN** the two lowest-ability foreigners (50 and 80) are omitted and the two highest keep their registration

### Requirement: Salary cap is opt-in and reports overage without auto-remediation

`salary_cap_exceeded` SHALL report true only when a cap is configured and the total annual wage bill exceeds it; with no cap configured it always reports false. The regulations layer itself SHALL NOT auto-remediate an over-cap squad.

#### Scenario: No salary cap is configured
- **WHEN** `salary_cap_exceeded` is checked against any wage total
- **THEN** it returns false
