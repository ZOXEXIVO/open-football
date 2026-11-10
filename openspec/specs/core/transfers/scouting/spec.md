# core/transfers/scouting Specification

## Purpose
Beyond its `recruitment/` child, this directory's own files run one club's
scouting day: assignment scanning and observation (`assignment.rs`), the
performance-breakout discoverability signal (`breakout.rs`), tunable scouting
config (`config.rs`), the scout-market desk that opens and closes scouting
corridors (`desk.rs`), available-player market-exposure scoring
(`exposure.rs`), what a scout concludes from observable signals
(`judgement.rs`), the year-round breakout watch (`watch.rs`), and the club's
standing watchlist of names (`watchlist.rs`).

## Requirements

### Requirement: A breakout player must clear a discoverability threshold to enter scouting attention
The system SHALL compute a 0-100 breakout score for standout non-star performers from position-weighted output, discounted by league reputation (except for youth squads), and SHALL only treat a player as a breakout once that score clears a fixed threshold.

#### Scenario: Breakout threshold gates discovery
- **WHEN** a player's breakout score is 45.0 or higher
- **THEN** he becomes eligible to enter club watchlists and recommendation flows as a breakout candidate

#### Scenario: A player's own desire to leave can lower his discovery bar
- **WHEN** a player privately wants to leave and the watching league's reputation exceeds his own club's league reputation by 1200 or more
- **THEN** his effective breakout/discovery bar is lowered, down to a floor of 22.0

### Requirement: Every scouting route opens a file through the shared gate
The system SHALL open or deepen a club's scouting attention on a player only on the gate's reading of that move, whichever route found him:

- the scan a scout runs for an open transfer request;
- a match a scout attends, and a domestic cup tie that shows a player to the clubs watching;
- the year-round breakout watch;
- the standing watchlist and the files opened from it;
- a report carried over from an earlier window;
- a name recalled from the club's memory of players it has seen.

A route that watches all year SHALL ask for "may be privately scouted". The request scan, which is filling a shirt in the window it is shopping in, SHALL ask for "may open negotiation".

The reputation of the country a player plays in SHALL NOT decide whether a club may watch him. His level, his wage and the club's market decide, as the gate reads them.

#### Scenario: A watchlist does not hold a player the gate would not let the club watch
- **WHEN** the weekly watchlist refresh considers a top-flight squad player for a club in a division he refuses, and he would be the largest improvement on the club's incumbent
- **THEN** he is not put on the club's watchlist, and no file is opened on him

#### Scenario: A report from last window is read again before it is used
- **WHEN** a new window opens and a club holds a report from the last one on a player who has since moved to a division and a wage the club's standing no longer fits
- **THEN** the report seeds no assignment and no file

#### Scenario: A standout in a match the scout attended
- **WHEN** a scout attending a reserve match sees a standout whom the gate does not let his club watch
- **THEN** no file is opened on him

#### Scenario: A countryman abroad may be watched from a smaller country
- **WHEN** a club in a small country considers a countryman on the fringe of a club in a far stronger country, whose floor has widened to the club's division and whose asking wage the club could hold
- **THEN** the club may watch him, although the country he plays in outranks its own

### Requirement: The standing watchlist ranks improvement among players the club could pursue
The system SHALL fill each briefed shirt's watchlist from players whose standing fits the club, and SHALL rank them by how much they would improve on the incumbent. How far a player stands above the club SHALL NOT by itself put him on the list.

A shirt briefed with no fee envelope SHALL watch only players who could come without a fee: those their club would lend, and those whose contracts are running out.

#### Scenario: The best player in the world pool is not the first name on a small club's board
- **WHEN** a small club's shirt has two candidates: a far stronger player whose standing does not fit the club, and a modest improvement whose standing does
- **THEN** only the second is on its watchlist

#### Scenario: A shirt with no fee envelope
- **WHEN** a shirt is briefed with no fee envelope and its candidates are a player under a long contract whom his club is not lending, and a player his club has listed for loan
- **THEN** only the loan-listed player is on its watchlist

### Requirement: A scouting department closes a file when the case for it has gone
Once a week, each club SHALL read again every file it holds on another club's player: monitoring rows, watchlist places, staff recommendations and shortlist places. A file whose move no longer reaches the stage that kind of file needs SHALL be closed, and the club SHALL remember when it last read him and what stopped the move.

A club with a live negotiation for a player SHALL keep its file on him until the negotiation resolves.

#### Scenario: He moved out of the club's reach
- **WHEN** a club holds a monitoring row on a player who has since signed a contract at a wage the club could not hold for one shirt
- **THEN** the next weekly review closes the row

#### Scenario: A file that still stands is kept
- **WHEN** a club holds a monitoring row on a player whose standing still fits it
- **THEN** the weekly review leaves the row and its observations as they are

#### Scenario: A negotiation holds the file open
- **WHEN** a club's standing no longer fits a player it is in live talks for
- **THEN** its file on him stays until the talks resolve
