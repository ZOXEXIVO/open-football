# core/continent/rankings Specification

## Purpose
Owns the continent's country and club coefficient rankings, recalculated from continental competition performance, and the qualification spot counts they determine for the following cycle.

## Requirements

### Requirement: Continental rankings drive qualification spot counts
Continental country rankings SHALL be recalculated from clubs' continental competition points, and the resulting rank SHALL determine each country's Champions League and Europa League qualification spot counts for the following cycle.

#### Scenario: A country ranks in the top 4 of its continent
- **WHEN** continental rankings are recalculated and a country is ranked 0-3
- **THEN** that country is allotted 4 Champions League spots and 2 Europa League spots
