# web/about Specification

## Purpose
Defines the About page, a static explainer of the project that renders without depending on a loaded game world.

## Requirements

### Requirement: About page renders without a loaded world
The system SHALL serve an About page at `/{lang}/about` that renders successfully whether or not a game world has been loaded, since it describes the project itself rather than any simulated state.

#### Scenario: Requesting the about page before a world is loaded
- **WHEN** a client requests `/{lang}/about` and no game world has been loaded yet
- **THEN** the page SHALL still render successfully, showing the application version and static descriptive content

#### Scenario: Requesting the about page with a world loaded
- **WHEN** a client requests `/{lang}/about` while a game world is loaded
- **THEN** the page SHALL render the same static content, unaffected by the state of the loaded world

### Requirement: About page reports the running build's version
The system SHALL display the application's build version on the About page.

#### Scenario: Version is shown
- **WHEN** the About page renders
- **THEN** it SHALL display the version the binary was built with
