# core/country/media Specification

## Purpose
Owns a country's press coverage: coverage intensity that trends toward saturation after results, and weekly transfer-rumor story generation for clubs.

## Requirements

### Requirement: Media coverage intensity trends toward saturation and generates weekly stories

Media coverage intensity SHALL trend upward toward a ceiling of 1.0 after each result update (blended 90% previous / 10% ceiling), and weekly story generation SHALL probabilistically attach transfer-rumor stories to clubs.

#### Scenario: Media coverage is updated after a batch of league results
- **WHEN** `update_from_results` runs
- **THEN** intensity moves closer to 1.0 but never exceeds it
