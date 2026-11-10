# core/country/result/transfers/free Specification

## Purpose
The country's daily free-agent market decides which unattached or out-of-contract players a club signs, against
which request, and in what role. A free signing must answer a real squad need, and the player must be told the truth
about his place.

## Requirements

### Requirement: A free-agent signing meets the terms of the request it fills
A club SHALL sign a free agent against an open transfer request only when he meets that request's own terms. His
ability SHALL be at or above the request's ability floor, and his age SHALL fall inside the request's age band. The
market's zero-fee discount MAY lower a floor that measures a level of play. It SHALL NOT lower a floor that measures
improvement over the player already in the shirt. A candidate's career pressure SHALL widen what he will accept and
SHALL NOT lower what the club asked for.

#### Scenario: A desperate keeper below the incumbent cannot fill an upgrade request
- **WHEN** a club's goalkeeper request demands an improvement over its current number one, and the only free agents
  willing to sign sit below that number one's level but inside the club's pressure-widened quality band
- **THEN** no free agent is signed against that request and the request stays open

#### Scenario: A cover request admits a zero-fee candidate just below its level floor
- **WHEN** a cover request carries no improvement requirement and a free agent sits within the zero-fee discount
  below its ability floor
- **THEN** he may be signed against that request

#### Scenario: The request's age band binds
- **WHEN** a succession request asks for a successor inside an age band, and a free agent older than the band
  otherwise meets its ability floor
- **THEN** he is not signed against that request

### Requirement: No free-agent signing lands surplus on arrival
Every free-agent route SHALL apply the squad-fit rule the paid recruitment paths apply. The routes are request
matching, staged depth negotiations, emergency fills, the market-clearing pass and pre-contract agreements. No route
SHALL sign a player who would be surplus in the buying club's main squad on the day he arrives. A signing that
displaces a weaker incumbent SHALL remain allowed. A squad too thin to field a side SHALL still take who it can get.

#### Scenario: A fourth keeper below the third is refused
- **WHEN** a club already carries three senior goalkeepers and a free agent ranks below all three
- **THEN** no free-agent route signs him to that club

#### Scenario: A better keeper may displace the weakest
- **WHEN** a free agent would outrank the club's third goalkeeper
- **THEN** the squad-fit rule does not block him, though every other gate still applies

#### Scenario: Market clearing skips stocked position groups
- **WHEN** the market-clearing pass looks for a landing club for a long-unemployed goalkeeper
- **THEN** a club where he would arrive surplus is not a candidate, however well his quality band fits it

#### Scenario: A deal in flight is dropped once the position fills
- **WHEN** a staged free-agent pursuit or a pre-contract reaches completion, and the buyer has filled the position
  since it was agreed
- **THEN** the signing does not complete, because he would now arrive surplus

### Requirement: A free agent is offered the role his arrival would give him
The role a club offers a free agent SHALL be the squad status he would hold on arrival. That status is his rank in the
buying club's main-squad position group, counting him, capped by the club's own level. It is the same ranking the
club's monthly squad-status pass applies. The same role SHALL drive:
- the offer's wage weighting;
- the player's acceptance scoring;
- any role promise installed on his contract.

A route whose pitch is a modest squad role MAY offer less than that projection. No route SHALL offer more. A backup
role or anything below it SHALL carry no role promise.

#### Scenario: A third keeper is offered a backup's role
- **WHEN** a free agent would rank behind the buying club's two best goalkeepers
- **THEN** the offer prices a backup role and installs no role promise
- **AND** the squad status he holds after the next monthly pass is the role he was offered

#### Scenario: A real upgrade is promised what he will get
- **WHEN** a free agent would rank first in his group and clears the club's key-player level
- **THEN** he is offered and promised a key role
- **AND** the monthly pass does not demote him below it while the promise binds

#### Scenario: A pre-contract uses the same projection
- **WHEN** a club agrees a pre-contract with an out-of-contract player from another domestic club
- **THEN** the promised role is his projected rank at the buying club, not a role read off the league baseline alone

### Requirement: A free agent's acceptance respects his level floor

A free agent SHALL carry a level floor read from his ability, his name and his held summit, with no current division
to cap it, and with his career pressure acting as his market resignation so the floor falls the longer he stays
unsigned. His willingness to sign SHALL drop as the buyer's division sits further under that floor, on every route that
asks him: request matching, staged depth negotiations, emergency fills and the market-clearing pass. A pre-contract
SHALL read the floor of the employed player he still is.

This SHALL only narrow what he will accept. It SHALL NOT lower what a club asked for, and it SHALL NOT stop a player's
floor falling far enough for him to find a club.

#### Scenario: Days out of a top-flight club
- **WHEN** a 29-year-old who was a regular in an 8800 division last season has been a free agent for a few days, and a
  club in a 5000 division offers him a key role
- **THEN** he refuses

#### Scenario: A year unsigned
- **WHEN** the same player has been unsigned for over a year and the same club offers again
- **THEN** his floor has fallen far enough that the division no longer stops him signing

#### Scenario: A journeyman has no such objection
- **WHEN** a free agent who has only ever been a regular in a 3000 division is offered a place in a 3000 division
- **THEN** the floor does not reduce his willingness at all

#### Scenario: An employed man's pre-contract
- **WHEN** a regular in a 9000 division, near the end of his deal, is considered for a pre-contract by a club in a 6000
  division
- **THEN** the club is not an acceptable buyer for him
