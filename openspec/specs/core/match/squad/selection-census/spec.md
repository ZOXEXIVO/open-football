# core/match/squad/selection-census Specification

## Purpose
Defines the selection census: a harness that runs a stubbed world for a season and reads every competitive first-team
sheet, reporting rotation, concentration of football, goalkeepers, benches and who starts, each beside a
real-football reference band, so that the realism of squad selection can be read from a whole world and compared
between two builds.

## Requirements

### Requirement: The census reads every competitive first-team sheet of a season
The census SHALL build the shipped world, run it on the match stub for a season, and read the starting XI, the bench
and the substitutes used of every competitive fixture played by a first team of a top-two-division league. It SHALL
classify each sheet as league, domestic cup or continental, and SHALL report top divisions and second divisions
apart. It SHALL read each player's age, planned role, squad status, market status and squad of origin as they stood
on the day.

#### Scenario: A league sheet
- **WHEN** a top-division first team plays a league fixture
- **THEN** its XI, bench and substitutes used are counted under league, in the top-division report

#### Scenario: A friendly
- **WHEN** a first team plays a friendly
- **THEN** the sheet is not counted in the competitive tables, and is counted in the pre-season table

### Requirement: The census reports rotation by what caused it
The census SHALL report the number of players changed between consecutive XIs:
- from one league match to the next, as a mean and a distribution;
- going into a sheet of each kind of competition;
- by days of rest since the previous competitive sheet;
- by the stakes of the fixture for the side, and by whether a fixture that matters more lay within four days;
- by quarter of the league season.

It SHALL report home and away sides apart wherever stakes are read.

#### Scenario: A European week
- **WHEN** a club plays league, continental and league fixtures inside eight days
- **THEN** each of the three sheets is counted in the short-rest row and in its own competition's row

#### Scenario: Stakes of the two sides
- **WHEN** a side fighting relegation visits a side with nothing to play for
- **THEN** the two sheets are counted in different stakes rows

### Requirement: The census reports how football is spread
For each club the census SHALL report how many players were given a league start, the share of league starts taken
by the eleven most-used, how many players started four league matches in five, and each planned role's share of
football against the share the plan gave it. It SHALL report starts by age band, by planned role, by transfer-listed
players, by players in the shop window, and by players named from another of the club's squads.

#### Scenario: Planned against played
- **WHEN** a season has been read
- **THEN** the report shows, for each planned role, the mean share planned and the mean share played

### Requirement: The census reports goalkeepers and benches
The census SHALL report, per club, the declared number one's share of league starts and of each other competition's
starts, how many keepers were given a league start, and every match a side started with an outfield player in goal,
with whether the club had a fit goalkeeper free that day. It SHALL report bench size by competition, the share of
benches naming no keeper, one, and two or more, the lines of the XI left without cover, and substitutes used.

#### Scenario: An outfielder in goal
- **WHEN** a side starts an outfield player in goal
- **THEN** the match is listed with the club, the date and the fit goalkeepers the club had in its other squads

### Requirement: Each figure stands beside its reference and the census compares two runs
Each headline figure SHALL be printed beside a real-football reference band and a verdict, inside or outside. Each
band SHALL carry a note of where it comes from. The census SHALL offer a comparison of two of its reports that lists
every row differing by more than the run-to-run noise, and the noise SHALL be measured from two runs of the same
build.

#### Scenario: A band is missed
- **WHEN** the mean XI change going into a domestic cup tie is 8.2 and the band is 4.0 to 6.5
- **THEN** the row is marked outside

#### Scenario: A report against itself
- **WHEN** a report is compared with itself
- **THEN** no row is listed
