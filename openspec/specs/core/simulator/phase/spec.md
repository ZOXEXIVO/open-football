# core/simulator/phase Specification

## Purpose
Executes the fixed, non-reorderable sequence of daily-tick phases — prologue, matchday, periodic passes, world pass, epilogue, and honours — that advance the simulated world by exactly one day.

## Requirements

### Requirement: Fixed daily phase order
The daily tick SHALL execute its phases in a single, non-reorderable sequence: an opening phase that prepares world-level context, a matchday phase, a periodic-passes phase, a world-level settlement phase, an epilogue phase, and an honours phase, followed by advancing the calendar date.
Phases MUST NOT be run individually or out of order, since each phase depends on state produced by the ones before it.

#### Scenario: One tick advances one day
- **WHEN** the simulator ticks the world once
- **THEN** every phase runs exactly once in the fixed order and the world's current date advances by one day at the end of the tick

#### Scenario: A default configuration is available
- **WHEN** the simulator is invoked without an explicit configuration
- **THEN** it runs with a default set of tunables equivalent to explicitly supplying one

### Requirement: World-level context precedes continental work
Before any continent simulates matches, the tick SHALL refresh cross-continent data that individual continents cannot compute on their own: player nationality-to-continent assignments (on designated reseed days), international call-ups, and any national-team matches for the day, and SHALL record national-team match results into both the match history store and the tick's outward result.
A world-level view of tournament proximity and each nationality's home-league standing MUST be constructed once per tick and made available to every continent for that tick.

#### Scenario: National team matches are simulated once per day, not per continent
- **WHEN** the opening phase runs on a day with scheduled international fixtures
- **THEN** those fixtures are simulated once at the world level (so squads may include players based at clubs on other continents) and their results are appended to the day's match history and to the tick's result

#### Scenario: Nationality reseeding only happens on designated days
- **WHEN** the current date is not a nationality-reseed day
- **THEN** the per-player nationality-to-continent assignment pass is skipped for that tick

### Requirement: Matches are built per continent but dispatched as one global batch
Each continent SHALL independently and in parallel produce the list of matches it wants played for the day, without performing any engine dispatch itself. All matches produced by every continent SHALL then be combined into a single collection and dispatched to the match engine exactly once per tick, after which results are returned to each continent that contributed matches.

#### Scenario: A quiet day dispatches nothing
- **WHEN** no continent produces any matches for the day
- **THEN** the engine dispatch step is skipped and zero matches are reported as dispatched

#### Scenario: Results are returned to the correct continent
- **WHEN** multiple continents each contribute matches to the day's global batch
- **THEN** each continent receives back only the results corresponding to the matches it contributed

### Requirement: A single continent's failure does not abort the tick
If building matches for one continent, or processing that continent's match results, raises an unrecoverable error, the tick SHALL isolate the failure to that continent, substitute an empty result for it, continue processing every other continent normally, and record the occurrence in a process-wide counter together with a log entry identifying the continent and the failing stage.

#### Scenario: One continent fails to build matches
- **WHEN** a single continent's match-building work fails unexpectedly
- **THEN** that continent contributes no matches for the day, every other continent's matches are still built and dispatched, and the world-wide failure counter increases by one

#### Scenario: One continent fails while processing results
- **WHEN** a single continent's post-match processing fails unexpectedly after the global dispatch
- **THEN** that continent's contribution to the day's outcome is empty, the failure counter increases by one, and the other continents' results are still applied

### Requirement: Cross-country sweeps run once per tick, not once per country
Work that must consider every country together — pruning transfer interest after domestic signings and applying free-agent market feedback (offers, rejections, block reasons) — SHALL be aggregated across all countries first and then applied to the world in a single pass per tick, rather than being repeated once per country.

#### Scenario: Domestic signings from many countries are cleaned up together
- **WHEN** several countries each complete domestic transfer signings on the same day
- **THEN** the cross-country shortlist cleanup runs once for the combined set of signed players rather than once per country

### Requirement: Season-boundary snapshots run before loan returns
When any league's season rolls over during a tick, an affected country's career-history snapshot SHALL be taken before that tick's loan-return processing moves any borrowed player back to their parent club, so a loanee's season statistics are captured under the borrowing club before the move.

#### Scenario: A loanee's season ends while still out on loan
- **WHEN** a country's season rolls over on the same day a loan spell is due to end
- **THEN** the career-history snapshot for that country is taken before the player is moved back to their parent club

### Requirement: World-only passes bridge cross-country actors
Certain actions can only be resolved at the world level because the two sides may belong to different countries: a manager moving between clubs, a parent club's share of a loanee's wages, and competitions whose entrants are drawn from more than one country. These SHALL be processed after the matchday and its periodic passes have completed for the day.

#### Scenario: A manager can move between clubs in different countries
- **WHEN** the manager market is processed for the day
- **THEN** moves are evaluated at the world level regardless of whether the origin and destination clubs are in the same country

#### Scenario: Monthly settlement runs on the first of the month only
- **WHEN** the current date is the first day of a calendar month
- **THEN** unsettled loan wage shares are settled and long-unemployed free agents are evaluated for retirement; on any other day these steps are skipped

### Requirement: End-of-tick cleanup settles derived state
After all of a day's matches (continental and global) have been played, the tick SHALL release any international-duty flags, move players whose contracts were cleared that day into the global free-agent pool, rebuild derived lookup indexes only when something changed that day, and seed career-history records for any player created that day.

#### Scenario: A released player becomes visible to other countries on a later tick
- **WHEN** a player's contract is cleared during a tick
- **THEN** they are swept into the global free-agent pool during that same tick's cleanup, but other countries' clubs only see them as available starting from the next tick's market snapshot

#### Scenario: Derived indexes are not rebuilt on a no-transfer day
- **WHEN** no transfer moved a player between clubs during the tick
- **THEN** the derived index rebuild step is skipped for that tick

### Requirement: Weekly, monthly, and yearly honours follow a fixed award order
On the day of the week designated for weekly honours, the tick SHALL build a shared per-league weekly aggregate once and then run, in order: the young weekly award, the senior weekly award, the young team-of-the-week selection, the senior team-of-the-week selection, and club news generation for the week. Monthly and calendar-boundary honours (monthly awards, league news, season awards, team of the year, and world player of the year) SHALL run after the weekly step, with league news generated only after monthly awards have been finalized.

#### Scenario: Weekly awards share one aggregate instead of recomputing per award
- **WHEN** the weekly honours step runs
- **THEN** the per-league weekly match aggregate is built exactly once and reused by every weekly award and team selection that day

#### Scenario: A larger award dampens a smaller one for the same player
- **WHEN** the same player would qualify for both a larger weekly award and a smaller one on the same day
- **THEN** the larger award is evaluated first so the award-reputation pipeline can reduce the smaller award's impact for that player

#### Scenario: League news is generated only after monthly awards are finalized
- **WHEN** the monthly/periodic honours step runs
- **THEN** league news generation occurs strictly after monthly awards have been computed, so it can report on frozen scoring charts
