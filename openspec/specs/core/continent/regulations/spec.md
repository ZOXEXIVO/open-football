# core/continent/regulations Specification

## Purpose
Owns the continent's regulatory ceilings: Financial Fair Play deficit limits that respond to economic health, and the foreign-player and youth-investment rules downstream enforcement reads.

## Requirements

### Requirement: Continental Financial Fair Play thresholds respond to economic health
Continental regulations SHALL adjust the FFP maximum permitted deficit based on the economic zone's health: a weak economy tightens the deficit ceiling, and a strong economy loosens it.

#### Scenario: Economic health falls below 0.5
- **WHEN** the continental economic health indicator is below 0.5 and FFP thresholds are updated
- **THEN** the maximum permitted deficit is reduced from its previous value

#### Scenario: Economic health rises above 0.8
- **WHEN** the continental economic health indicator is above 0.8 and FFP thresholds are updated
- **THEN** the maximum permitted deficit is increased from its previous value

### Requirement: Continental foreign-player and youth-investment regulations exist as configurable ceilings
Continental regulations SHALL define a maximum non-EU/foreign player count, a minimum homegrown-player count, and minimum youth-academy investment and squad-size requirements, usable by downstream enforcement.

#### Scenario: Regulations are constructed with defaults
- **WHEN** continental regulations are initialized
- **THEN** the maximum non-EU player count defaults to 3 and the homegrown minimum defaults to 8
