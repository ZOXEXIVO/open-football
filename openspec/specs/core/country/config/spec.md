# core/country/config Specification

## Purpose
Owns a country's static/default construction settings: transfer-market pricing baseline and the procedural skin-color distribution used for player generation.

## Requirements

### Requirement: Country pricing and skin-color distribution default sensibly at construction

A newly constructed country's pricing SHALL default its price level to 1.0, and its skin-color distribution SHALL default to fixed baseline proportions (50 white / 20 black / 30 metis) usable for procedural player generation.

#### Scenario: A country is constructed with no explicit settings override
- **WHEN** `CountrySettings::default()` is used
- **THEN** the price level is 1.0 and the skin-color distribution sums to 100 across its three categories
