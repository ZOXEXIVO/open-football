# core/transfers/pipeline/approach Specification

## Purpose
Orchestrates the weekly transfer pipeline's ordering and opens negotiations on a club's shortlisted targets, domestic clubs before foreign ones.

## Requirements

### Requirement: The transfer pipeline runs a fixed weekly sequence of orchestration phases
The system SHALL run the transfer pipeline in a fixed weekly order — staff recommendation generation, recommendation intake, shortlist building, board approval review, domestic approach, foreign approach, and market circulation/diagnosis — gated by each country's own market-cadence schedule.

#### Scenario: Domestic approach precedes foreign approach
- **WHEN** the weekly pipeline runs for a country
- **THEN** the domestic approach pass (including loan-out listing processing) completes before the foreign approach pass for that country begins
