# Player ability audit and calibration

The creation path starts at `DatabaseGenerator::generate`, reaches club/squad
generation, and then chooses `PlayerGenerator::generate_from_odb` for source
records, the database senior generator for procedural players, or the core
academy generator for U18/U19 squads. The final skills are scored by
`PlayerSkills::calculate_ability_for_position`.

## Reference data and method

On 2026-09-28 the source tree at `F:/Rust/open-football-database/data` contained
62,982 player files. 39,086 had outfield attributes and 15,661 had all 36
outfield attributes. The original Rust calculator could be checked against
17,149 unique profiles, including keepers with all its scoring inputs present.
Missing attributes were bounded between 1 and 20; profiles whose missing values
could change CA were excluded, rather than filled with synthetic values.

Before hydration or rescaling, the old calculator had mean absolute error
8.41 CA points, mean signed error +0.48, and 47.8% within six points of source CA.
This was not merely a generation error: scaling real recorded attributes to
that calculator unnecessarily changed the reference profiles.

Calibration uses 17,141 profiles with the required inputs present. Player IDs
divisible by five are reserved for validation (3,498 players); 13,643 other
players fit nonnegative position-specific weights. Identity, age, reputation
and PA are not predictors. Left/right variants share weights. Wingbacks, with
only 172 examples, train with the fullbacks and then get their own table: a fit
on their records with a ridge toward the pooled weights (strength 1e3, chosen by
5-fold cross-validation), because the pooled table under-scored them by 5.7 CA
on average. Unobserved formation variants use the corresponding central/wide role.

The empirical mapping is `CA = round(19 * weighted_skill_average - 110)`,
clamped to 1..200. Weights sum to one and are normalized again after rounding
to four decimals. This gives a shared inverse `(CA + 110) / 19` for flat
profiles, used by generation and development. Role-shaping weights remain
separate: they govern the shape of a player, not the measured CA cost of skills.

| Validation group | Players | Old MAE | Calibrated MAE | Within ±6 |
| --- | ---: | ---: | ---: | ---: |
| Goalkeeper | 388 | 9.24 | 2.41 | 95.1% |
| Central defender | 498 | 8.17 | 2.41 | 94.4% |
| Fullback | 763 | 8.34 | 4.33 | 78.4% |
| Wingback | 44 | 11.41 | 4.34 | 81.8% |
| Defensive midfielder | 466 | 8.29 | 3.08 | 91.2% |
| Central midfielder | 318 | 8.17 | 3.79 | 85.8% |
| Wide midfielder | 257 | 9.93 | 3.42 | 85.6% |
| Central attacking midfielder | 46 | 7.91 | 3.30 | 89.1% |
| Wide attacker | 439 | 8.93 | 3.51 | 83.4% |
| Striker | 279 | 7.46 | 3.05 | 85.7% |
| **All** | **3,498** | **8.55** | **3.34** | **86.8%** |

This is an empirical approximation, not recovery of the external engine's
exact formula. Source attributes are integer-rounded; multiple positions and
two-footedness carry information absent from the skills-only API. A separate
exploratory fit including the weaker foot reduced validation error further,
to about 2.53 CA, but that requires a player-level model and is not incorporated
here. Position costs remain estimates, particularly for sparsely sampled roles.
The reference cohort spans CA 40..196, mostly 80..139; accuracy below 40 and
at the top extreme is not established by this dataset. Boundary tests verify
boundedness and fitting, rather than external accuracy at those extremes.
The changed scoring applies wherever current skills are evaluated, including
existing saves; stored source CA is still retained within the import tolerance.

Binned by predicted CA over all 18,041 audited profiles, the residual stays
within ±0.5 from 60 to 179 (including 80 profiles predicted 160..179), so the
linear scale holds at the elite end; a quadratic mapping does not improve the
held-out error. Below a predicted 60 the source is thin and slightly
under-scored (+1.9 at 40..59, 216 profiles).

## Other corrections

- CA fitting now bisects the actual scoring function. The old two-pass ratio
  could produce CA 29 for a requested CA of 1 and stop on rounding plateaus.
- Partial source records first fit the generated profile to the recorded CA,
  as for a procedural player, then overlay the record. Generated slots flex
  within ×0.8..1.25 to close the remaining gap; what is left belongs to the
  import tolerance. Letting them absorb the whole gap drove slots with little
  CA weight to 1 or 20 (24% of keepers' generated slots: Alisson's unrecorded
  technicals all became 1). Masking real missing patterns onto complete
  records, the missing values are reconstructed with MAE 2.36 (outfield) and
  2.56 (keepers), against 2.55 and 2.92 before; the error is flat up to a
  quarter of flex and grows beyond it. The import tolerance cannot put
  skill-derived CA above PA. Unknown CA zero is not stored as a valid ability.
- Absent (zero) attributes stay absent when fitting or clamping. Every
  recorded outfielder carries first touch and passing in the goalkeeping
  block, so a goalkeeping-average guard let his other keeper attributes be
  fitted: with zero CA weight they ran to 20 (Vinícius Júnior had 20 reflexes).
- Development ceilings are `base × role weight × maturity`, but CA prices
  skills with the calibrated weights, so the flat base left roles whose shape
  under-weights their costly attributes short of PA: fully developed at 28, a
  fullback's ceilings scored 0.82 × PA and a keeper's 1.36 × PA. The base is
  now sized per position (`PlayerSkills::shaped_skill_level`).
- Maturity is read as a share of ability: a family at maturity `m` may hold
  the level of a player of `m × PA` (`PositionalSkillCeilings::for_player`).
  Multiplying the level by `m` meant a far smaller share of ability on this
  scale, whose zero sits at level 5.8: the ceilings let a player reach only
  0.71..0.82 of PA at 20..22 and 0.84..0.88 at 25..26, below the database's
  median of those ages (0.79..0.80, 0.90..0.91). Read as ability, the same
  table sits at or just under the database's top decile (0.86, 0.92, 0.98)
  for every role and PA: 0.85..0.88 at 21½, 0.91..0.93 at 24½ and
  0.95..0.96 at 27½ for a regular, keepers ~0.05 lower until their late
  twenties. The generator keeps reading it as a share of each attribute,
  which lands generated youth at the database median.
- Rolling a negative PA band uses an independent random stream, so changing
  future potential does not reroll the current player.
- Academy profiles use exact-position weights, respect their original PA
  instead of increasing it to cover generated skills, and cap keeper skills
  as well as outfield skills. Readiness is excluded from attribute variation.
- A goalkeeper's outfield and keeper attributes use the same archetype roll.

## Balance effects against HEAD

Measured with a throwaway harness (not checked in): every second source record
hydrated by each tree's own pipeline (31,255 players), then six seasons of the
weekly development tick under fixed conditions — neutral coach, league
reputation 6000, median rolls, 300 minutes per 30 days.

- Growth is modestly faster: six-season CA gain +22.4 from ≤18, +18.0 from
  19..21 and +13.8 from 22..24, against +20.8, +14.8 and +10.8 at HEAD.
- Prospects aged 17..21 with PA − CA ≥ 15 close 56..64% of their gap in every
  role. At HEAD outfielders closed 46..55% and keepers 92%; without the
  per-position ceiling base the change spread it to 48% (fullbacks)..77%
  (strikers) and 98% (keepers).
- Decline is much steeper in CA terms: 31..33-year-olds lose 55.7 CA over six
  seasons (HEAD 21.0), 34+ lose 61.6 (HEAD 24.9). Skill-point decline is
  unchanged; the calibrated scale charges 19 CA per average point instead of
  ~10.5 and weights pace and acceleration, which decline first, heavily. If it
  is too steep, the skill-decline rates are the lever, not the scale.
- The weekly tick stores the skill-derived CA, so a real player's CA moves to
  his calibrated score at the first tick (Messi 172 → 166).
- HEAD raised academy PA to the generated CA, which hid the sampled PA: an
  average academy's 14-year-olds now show PA < 30 in 38% of intakes (HEAD
  0.2%). The PA 60+ tail is unchanged.

## Whole careers

`club/player/development/career_tests.rs` runs careers through the full daily
pipeline — match load, recovery, the club's training sessions, the off-season
camp and the weekly tick — with seeded rolls, and pins them to the shipped
database's CA/PA percentiles by age (PA ≥ 110, 36,062 records). The weekly
tick alone understates growth: training contributes about a fifth of it.

- Off-season camps used to add stamina, concentration, composure, decisions,
  first touch, passing and technique to every club player on every
  off-season day (about 1.5..2.4 per attribute per summer), past PA and the
  age ceilings. Every career reached CA = PA by 22..24 and stayed there until
  about 32, whatever the club, minutes or character. Camps now only rebuild
  sharpness (`Player::on_offseason_camp_day`).
- Minutes used to barely matter: an unused squad player ended within 0.01 of
  a regular starter. The tick separated them (0.03..0.06 of PA), but training
  gains scale with the gap to potential, so whoever fell behind trained it
  back — rates converge on the same ceiling. Minutes now act on the ceiling:
  the share of each maturation family that only matches build
  (`SkillMaturation::match_share`: mental 0.15, goalkeeping 0.12, technical
  0.08, strength/stamina 0.03, speed 0) opens with
  `PlayerLoad::match_exposure`, the last 30 days' minutes against a weekly
  starter's, friendlies and youth or reserve league games at half. A regular
  who loses his place keeps what he built. The senior-minute bands the tick
  rates were rescaled to that 30-day window (a weekly start holds ~380; the
  old adult bands began at 600 and ran to 3000), and a cup-only player's
  rating no longer reads as 0 (it cost him 15% of his growth).
- Training ran a full session on match days (the fixture window files the
  day's played match as the previous one, which the no-training rule never
  checked) and planned every ordinary week as congested (fixtures within
  ±7 days plus the last 14 days, against a threshold of 2). Congestion is
  now the competitive fixtures in the seven days around today.
- The monthly specialty-skill plan added +0.5 to its skill every month (up
  to +3, capped at 14) straight into the attribute, past PA and the
  ceilings, on top of the extra reps the daily sessions already give the
  planned skill. The monthly pass now only runs the plan; a season of the
  reps alone takes a lagging skill about 1.9 further than no plan, under the
  ceiling.
- The maturation table held each share flat across bands of two to four
  years, so a ceiling-bound career stalled and then lurched: the typical
  starter gained 0.015 of PA in the season to 22 and 0.031 in the next, 0.004
  to 26 and 0.014 to 27. `SkillMaturation::ratio` now takes fractional age
  and runs straight between each share held at the middle of its years (a
  peak across all of them), so gains fall season by season: 0.032, 0.022,
  0.019, 0.020, 0.017, 0.016 from 22 to 27. The ceiling rounds a hundredth or
  two off the old band tops near 25..29; it sits within 0.04 of the
  database's top decile there.
- CA/PA of a PA 150 player developed from 17 (mean of eight roles, median
  character unless stated):

  | age | minnow starter, lax | average starter | average youth-side regular | average, unused | elite starter, model pro | database median | database p90 |
  | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
  | 22 | 0.77 | 0.85 | 0.82 | 0.79 | 0.89 | 0.80 | 0.88 |
  | 24 | 0.82 | 0.90 | 0.86 | 0.83 | 0.93 | 0.87 | 0.92 |
  | 26 | 0.86 | 0.93 | 0.89 | 0.86 | 0.97 | 0.91 | 0.97 |
  | 28 | 0.89 | 0.96 | 0.92 | 0.89 | 0.99 | 0.93 | 0.98 |

- Outfielders peak at 29..30. Decline, as CA over the career peak, is 0.97 at
  31, 0.89 at 33 and 0.75 at 35; the world generator builds veterans at 0.97,
  0.90 and 0.80. Keepers hold 0.88 of PA at 35, outfielders 0.72.

`cargo test -p core --lib career_table -- --ignored --nocapture` prints the
full grid.

## Reproduce

Run the audit from the repository root in PowerShell. The source tree is read
only; the output file is written inside this workspace.

```powershell
New-Item -ItemType Directory -Force artifacts/attribute-audit
$env:OPEN_FOOTBALL_DATA = 'F:/Rust/open-football-database/data'
$env:ABILITY_AUDIT_OUTPUT = "$PWD/artifacts/attribute-audit/source_ability.jsonl"
cargo test -p database audit_source_ability -- --ignored --nocapture
python .dev/ability/calibrate.py artifacts/attribute-audit/source_ability.jsonl
```

The Python step requires NumPy and SciPy. It writes candidate weights,
validation metrics and fixture candidates alongside the audit, without editing
Rust. Its `before` column means the calculator used when that audit was run;
the table above uses the pre-change audit. The checked-in regression fixtures
are the 20th, 50th and 80th CA percentiles of each held-out role group, selected
without filtering on prediction error. The Rust tests score those original
profiles and also check all CA values from 1 through 200.

## Verification

The Rust audit reproduces the Python predictions exactly on all 3,674 held-out
records of the 2026-09-29 audit, wingbacks included. The script reproduces
every checked-in table from that audit (keepers within 0.003: the audit now
admits keepers missing only zero-weight attributes). The full core run passed
5,027 tests and all 54 database player-generation tests pass, including
embedded-record checks. Formatting checks pass for the changed files.

The wider database suite also contains an unrelated failing calendar test:
`the_euro_and_the_world_cup_never_open_qualifying_in_the_same_year`. Its shipped
Euro configuration opens cycles in every even year, while the test expects
2026, 2030, 2034 and 2038. No calendar code or source calendar data was changed
by this work.
