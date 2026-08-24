# Player situations — what the mind can hold, and what football has that it doesn't

Companion to `docs/player_mind.md` (the architecture) and
`docs/staff_mind.md` (the dugout half). Those documents describe how the
mind is *built*. This one asks a different question, from the outside:

> Take a real footballer's year. How much of it can this mind actually
> see, and what would it take to see the rest?

The short answer is that the instrument is finished and the world barely
speaks to it. Nine tenths of the work below is not new machinery — it is
connecting machinery that already exists on both ends.

> **Status.** §§1–3 are the audit as it stood before the work; they are
> left in the present tense because they are the argument for what was
> built, and because the numbers in them are the before-side of the
> measurement. Everything proposed in §4 is implemented except the two
> optional halves of Tier 4 and phase 2's beliefs — see **§8, the
> implementation log**, for what landed, what it cost, and the five
> defects the work turned up.

---

## 1. The state of the instrument

| Organ / faculty | Built | Fed by the live sim | Read by anything |
|---|---|---|---|
| `MindMemory` (episodes, convictions, ledger) | yes — ~3 000 LOC, 94 tests | **15 of ~70** episode kinds | `wants_to_leave()`, one call site |
| `GoalStack` (33 wants, five-rung ladder) | yes — ~3 000 LOC, 78 tests | yes, via 4 faculties + legacy bridge | nothing downstream |
| `CareerMind` | yes | yes | mood profile only |
| `CompetitiveMind` | yes | yes | mood profile only |
| `SocialMind` | yes | yes | mood profile only |
| `FinancialMind` | yes | yes | mood profile only |
| `ProfessionalMind` | yes | **never runs** (§2.1) | — |
| `MindBeliefs` / `SelfImage` / surprise (phase 2) | **not implemented** | — | — |
| `MindDeliberation` / `weigh` (phase 5) | **not implemented, player side** | — | — |
| Match seam (phase 6) | **not implemented** | — | — |

The mood layer next door is the opposite: **204 `HappinessEventType`
variants**, 13 calibrated factors, and a player page that renders them.
Everything the mind lacks in reach, the happiness layer already has —
including situations I would otherwise be proposing here
(`ThreatenedByNewSigning`, `RolePathBlockedAtEliteClub`,
`TrainingStandardFrustration`, `LearningFromStarTeammate`,
`PathwayBlockedByLoanSigning`, `PlayingForNewContract`).

So the gap is not *"the simulation doesn't notice football"*. It is:

> **The layer that notices football is a mood that fades in a month. The
> layer that could hold an intention for a season and a grudge for a
> decade is the one nothing talks to.**

---

## 2. Five things that are dark

### 2.1 The professional faculty has never run in a live world

`Player::mind_situation` hard-codes the manager
(`player/core/player.rs:1171`):

```rust
// The manager is resolved by the caller that knows the staff;
// `None` until an emit site supplies it …
manager: ActorRef::NONE,
```

and the only live caller (`player.rs:1255`) overrides exactly one field:

```rust
let mut situation = self.mind_situation(now.date(), country_code);
situation.club_reputation = (team_reputation / 10_000.0).clamp(0.0, 1.0);
self.mind.tick_with(&mind_ctx, &situation);
```

`ProfessionalMind::reflect` opens with `if s.manager.is_none() { return; }`.
It has therefore never executed a rule in a simulated world. What that
costs:

- `WinTheManagersTrust`, `PlayInMyBestRole` and the professional route to
  `LeaveThisClub` are **unreachable goals**.
- *"A change of manager can rescue a career"* — the headline behaviour in
  `player_mind.md`, phase 4 — has never happened.
- Being frozen out, being misused, and being written off by a specific
  man are all modelled and all inert.

### 2.2 A player cannot remember winning the league

The complete player-side emit vocabulary, from `match_play.rs`,
`player.rs::verify_promises` and `transfer.rs::complete_transfer`:

```
SeniorDebut  FirstGoalForClub  DecisiveGoal  ManOfTheMatch  CostlyError
SentOff  DerbyWin  DerbyDefeat  FansAdoration  FansHostility  MediaPraise
ManagerPromiseKept  ManagerPromiseBroken  SoldAgainstWill  SignedForClub
```

Fifteen kinds. The catalog holds about seventy. Never recorded for a
player, though every one has a gate in the sim that already fires:

- **Career** — `WonLeagueTitle`, `WonDomesticCup`, `WonContinentalTrophy`,
  `Relegated`, `Promoted`, `ReleasedByClub`, `LoanedOut`,
  `ContractRenewed`, `CaptaincyAwarded/Removed`, `ClubServantMilestone`
- **The manager** — `ManagerArrived`, `ManagerLeftClub`, `ManagerFrozenOut`,
  `ManagerSignedARival`, `ManagerPublicPraise/Criticism`,
  `ManagerPrivateBacking`
- **Selection** — `StartedBigMatch`, `LeftOutOfBigMatch`, `DroppedToBench`,
  `WonStartingPlace`, `LostStartingPlace`, `SubbedOffEarly`,
  `RoleUpgraded/Downgraded`
- **The room** — every one of the seven
- **The body** — `SeriousInjury`, `CareerThreateningInjury`,
  `ReturnedFromLongInjury`
- **International** — `FirstCap`, `NationalSnub`, `MajorTournamentSquad`,
  `NationalTeamGlory`
- **Money** — `BigPayRise`, `WageBelowPeers`, `ClubRefusedTerms`,
  `ClubBrokeWagePromise`

A career, as this mind currently records one, is: a debut, some goals, a
red card, two derbies, and the day he was sold. He remembers the fans
better than the trophies.

### 2.3 Personality decides nothing about what a man wants

`MindTickContext` carries `professionalism`, `consistency`, `temperament`
and `loyalty` — and passes all four only to the **memory** organ, where
they set the forgetting rate and the nostalgia drift. `MindSituation`
carries `ambition`, `pressure`, `adaptability`.

Grep the five faculties for the other four attributes: **zero hits**
outside test fixtures.

So two players with identical minutes, identical contracts and identical
clubs form identical wants at identical strengths — one of them a
one-club loyalist with professionalism 18, the other a mercenary with
loyalty 3. This is the single biggest variety gap in the system, and
almost every rule below fixes some of it for one clause each.

The same is true of `surprise`. `EncodingInputs::surprise` is passed as
the literal `0.5` at every emit site in the codebase, because phase 2's
beliefs were never built. The formula that was supposed to make players
*remember different things about the same season* is running with its
differentiating term pinned to a constant.

### 2.4 The mind has no output

There is no `PlayerMind::deliberate`. No player faculty implements
`weigh` — they all inherit the default `ReasonSet::new()`. The single
behavioural read of the whole system in the live simulation is:

```rust
// player/events/transfer.rs:50
|| self.mind.wants_to_leave() >= Self::MOVE_WAS_HIS_IDEA;
```

…one boolean, used to decide whether a completed sale files
`SoldAgainstWill`. Nothing else in the simulator, and **nothing at all in
the web layer**, reads a goal, a conviction or a ledger account. The
player page renders `happiness.morale` and the 13 factors; the mind is
invisible.

The staff half is well ahead here — `Staff` already calls
`self.mind.deliberate(MindOption::Resign)` and branches on
`GoalKind::GetOutOfHere`.

### 2.5 The situation is built in the wrong place, and it shows

Compare the two builders:

| | Player | Manager |
|---|---|---|
| Built in | `Player::simulate`, from `GlobalContext` | `Club::run_manager_mind`, with the whole club in hand |
| Knows the other people at the club | **no** | yes — standing, squad, board |
| Situation fields that reach a faculty | 11 of 17 | all |

`MindSituation` is 17 fields and the faculties consume eleven.
`is_on_loan` is dead. `familiar_teammates` reaches only
`is_culturally_isolated`. `manager` is never set (§2.1).

More importantly, the *missing* fields are all the ones that need the
squad, and they are exactly the football-decisive ones: where he ranks at
his position, who he is competing with, whether the club just signed
someone in his shirt, what he was bought to be, whether the man who
signed him is still here.

**The fix shape is already in the repo**: `Club::run_manager_mind` is a
weekly, club-level pass that assembles a rich situation for one man. The
player equivalent — call it `run_squad_minds` — is the enabler for most
of §4.

---

## 3. The football reading

Four things a real footballer's year contains that this model cannot
currently express, independent of the wiring gaps above.

### 3.1 A signing is a promise, and the drama is in the gap

Clubs do not sign players; they sign players *for something*. The fee
relative to what the club has ever paid, the wage relative to the
dressing room, the number on the shirt, and what the manager said at the
unveiling all set a bar. The same 6.8 average rating is a promising start
for a squad addition and a crisis for a club-record buy.

The mind's only yardstick for a new arrival is `expected_start_share` —
a lookup from `PlayerSquadStatus`. A £40m striker and a free transfer
with the same paperwork are the same man in October.

### 3.2 Long service is three different men, and this model has one

`CareerMind::reflect` gates the entire long-service branch on
`ambition >= 12.0` and then forks on club reputation:

```rust
if s.ambition < Self::AMBITIOUS { return; }
…
let goal = if s.club_reputation < 0.6 { StepUpToABiggerClub }
           else { FindANewChallenge };
```

Football has at least three, and the fork is personality × how the club
has treated him:

1. **The one-club man.** High loyalty, warm memory. Wants to stay, and
   past six or seven years wants the *recognition* — the armband, the
   testimonial, being called a club servant. Will take a pay cut. Refuses
   bigger clubs. Modelled halfway: `StayAtThisClub` forms, but only from
   `SocialMind` belonging, never from loyalty, and nothing ever wants
   recognition.
2. **The loyal-but-ambitious.** High loyalty *and* high ambition — a
   combination the current code can only resolve as "leave". In football
   this man does not leave; he asks the club to **match** him. That is
   the player-side mirror of the manager's `BeBackedInTheMarket`, and
   `GoalKind::PlayWithBetterPlayers` is already in the catalog with no
   rule that forms it. If the club sells its best players instead, *then*
   he flips to `FindANewChallenge`. The conditional flip is the story.
3. **The restless.** High ambition, low loyalty. Today's path, correct.

And the fourth case, the commonest of all: a 30-year-old of ordinary
ambition, six years at the club, who simply stays. Today that is an
absence of rules rather than a stable state.

### 3.3 Wanting to develop is not wanting a bigger club

The place where the current model is furthest from the sport.

A player who has stopped improving knows it before anyone tells him — he
feels it training against the same teammates, in a manager whose
instructions haven't changed in two years, in his own numbers. What he
wants in response is *not* a bigger badge. In order, he wants:

1. **A better teacher** — a coach who improves him. This is why
   21-year-olds leave big clubs for mid-table ones with a reputation for
   development, which the current transfer model cannot produce at all.
2. **A different role** — the No. 6 who wants to play as an 8.
3. **More responsibility** — set pieces, the armband, being the one the
   team plays through.
4. *Only then*, a bigger club.

Today all four collapse into `StepUpToABiggerClub`, gated on ambition
≥ 12, after 730 days, keyed on club reputation. The distinction matters
because the first three are **satisfiable at the club** — by a coaching
appointment, by an individual training plan, by a role change — and the
fourth is not. A model that only has the fourth turns every plateau into
a sale.

Note the constraint (`feedback_hidden_potential_ability`): the player
must not read his own CA or PA. That is not an obstacle — it is the
better model. His self-read should be built from what he can observe
(rating trend, training performance, minutes, reputation, the level of
clubs asking after him) and should therefore be *wrong* sometimes, which
is phase 2's `SelfImage` and is where "he thinks he's better than he is"
comes from.

### 3.4 Standing is positional, not squad-wide

Everything about a player's place is currently a scalar:
`starter_ratio` against `expected_start_share`. Football is a queue at a
position. Two players on identical minutes are in completely different
situations if one is 20 and behind a 32-year-old and the other is 27 and
behind an equal. A young player behind a great is patient. A peak-age
player behind a peer is not, and never has been.

The sim knows this — `ThreatenedByNewSigning`,
`enforce_position_group_minimums`, `SquadAssetContext::group_levels` all
reason positionally. The mind does not.

---

## 4. Proposals

Ordered so that each tier makes the next one cheap. Everything is
football-only; §5 says what I deliberately left out.

### Tier 0 — turn the lights on

These are not features; they are the reason the features below are small.

**0.1 · `run_squad_minds`, a club-level weekly pass.**
Mirror `Club::run_manager_mind`: assemble the player's situation where
the squad and the staff are in hand, then tick. New situation fields, all
`Copy`:

| Field | Football meaning |
|---|---|
| `manager: ActorRef` | fixes §2.1 — turns the professional faculty on |
| `pecking_rank: u8` / `rivals_at_position: u8` | the queue he is in (§3.4) |
| `top_rival: ActorRef` | the man in his shirt |
| `rival_gap: i8` | his observable level minus the incumbent's — **CA-blind**, via `AbilityEstimator::observable_level` |
| `is_captain` / `is_vice` | standing |
| `wage_rank: u8` | where he sits in the dressing room's pay order |
| `coaching_ceiling: u8` | best of `coach_best_technical/mental/fitness` — *already on `ClubContext`, no plumbing* |
| `months_to_tournament: u8`, `national_standing` | 3.1 below |

Cost: one pass, one struct. Everything else in this document reads from
it.

**0.2 · Thirty emit sites.** Every kind in §2.2 has a gate that already
exists and is already calibrated — a trophy, a relegation, a captaincy
change, an injury, a squad-registration omission, a contract renewal. The
rule from phase 1b holds: *tap the gate the happiness layer already
trusts*, never invent a second bar for "was it memorable". Priority order
by narrative weight: trophies and relegation → manager arrival/departure
→ injury → selection and role → captaincy → international → the room.

**0.3 · Personality reaches the faculties.** Put `loyalty`,
`professionalism` and `temperament` on `MindSituation` and use them.
Roughly one clause per rule, and it is what makes two players in the same
situation behave differently — the whole variety budget for the price of
a few lines.

**0.4 · Give the mind an output.** `PlayerMind::deliberate`, player-side
`weigh`, consumed at **exactly one** site first —
`negotiations.rs::resolve_personal_terms` — behind a `.dev/transfers`
acceptance-rate census, per the repo's own parallel-run discipline. The
`WeightedReason`s drop straight into `TransferReason{key,scout,rival}`
and the newspaper desks, which already exist.

### Tier 1 — the arrival

**1.1 · The mandate.** Stamp an `ArrivalMandate` at
`Player::complete_transfer`: fee relative to the buying squad's value,
wage rank, the squad status agreed, the shirt, and **`signed_by`** — the
head coach on the day. It does three things:

- raises his own expectation above what the role table says (a
  club-record buy expects to start, whatever the paperwork);
- sets the patience window as a real deadline (`MindGoal::commit_until`) —
  a marquee signing gives himself until Christmas, a squad addition until
  next season;
- becomes the yardstick for the flop arc, so the £40m striker with no
  goals by December escalates on a schedule the free transfer never does.

Feeds the existing `LosingPatienceWithSigning` / `DressingRoomStatusShock`
moods rather than duplicating them.

**1.2 · The man in my shirt.** With `top_rival` and `rival_gap` from 0.1:
displacing him files `WonStartingPlace` against a *person*, and the two
ledgers move in opposite directions. Years of that become `BadBlood` —
or, if the older man helped, `HeMentoredMe`. Failing to displace scales
`WinBackMyPlace` by the **gap**, not by absolute minutes, which is the
distinction in §3.4.

**1.3 · The manager who signed me.** The sharpest of the three, and it
runs entirely on machinery that exists. When a manager leaves and
`mandate.signed_by == him` and the player is inside ~18 months:

- he loses his sponsor — expectations are no longer underwritten, and
  `WinTheManagersTrust` forms at high strength against the successor;
- if his ledger `trust` in the departing man was strongly positive, the
  `ManagerLeftClub` episode lands hard and consolidates `HeBackedMe`;
- if the successor's `CoachDecisionState` never rated him,
  `ManagerFrozenOut` fires and — crucially — `WinBackMyPlace` is
  **blocked** (`GoalBlocker`) rather than pursued, which routes him to
  `BeAllowedToLeave`. Frozen out is not the same state as benched, and it
  should not produce the same behaviour.

*"He was the last manager's signing"* is a whole category of footballer
and it currently cannot exist.

**1.4 · The return.** Already designed (`player_mind.md` §4.6),
implemented in memory, unreachable for want of `weigh`. 0.4 makes it
real: a step down accepted, a lower wage taken, and a hard block if the
coach who broke his word is still there — with the reasons printable.

### Tier 2 — the long stay, and wanting to get better

**2.1 · The plateau he can feel.** A small `SelfImage` (phase 2, scoped
to this) fed only from observables already at the player tick: rating
trend over ~18 months, `training.training_performance` EMA, minutes,
reputation trend, and the club's `coaching_ceiling`. Then a **three-rung
escalation** where today there is one:

```
KeepImproving           latent, silent, satisfiable in place
      ↓  (club's coaching ceiling doesn't move, 2+ windows)
WorkWithABetterCoach    still satisfiable in place — by a hire
      ↓  (nothing changes)
StepUpToABiggerClub     today's only rung
```

`KeepImproving` is satisfied by a rating trend turning up, by a better
coach arriving, **or by the manager giving him an `IndividualTrainingPlan`**
— which would be the first real interaction between the coaching
machinery (wired July 2026, `training_direction.rs`) and what a player
wants.

The prize is a transfer market where **a player moves for the coaching,
not the badge** — a mid-table club with an excellent staff outbidding a
bigger one for a 21-year-old, which no detector in the repo can currently
express.

**2.2 · Three ways to be a long-serving player.** Replace the
`ambition >= 12` gate with the §3.2 fork on loyalty × ambition × club
sentiment. The new rule is the middle man: **`PlayWithBetterPlayers`**,
formed by a loyal, ambitious, long-serving player; voiced, it is a public
"we need to strengthen"; and it flips to `FindANewChallenge` **only if
the club sells instead of buys**. Conditional on the club's actual
behaviour, over a window — the kind of logic the legacy detectors cannot
hold, because they re-derive from scratch every week.

**2.3 · Recognition.** Long service should want something. Tap
`ClubServantMilestone` at the appearance milestones that already fire;
let it form `BeCaptain` for the right personality, and consolidate
`SpiritualHome`. A club servant refused the armband is a real grievance
and is currently inexpressible.

**2.4 · Succession, both ways.** The veteran watching a young replacement
arrive forks on professionalism and temperament: **mentor** (emit
`MentorSupport`, soften his own `WinBackMyPlace`, form `MoveIntoCoaching`)
or **resist** (`TeammateConflict`, `WinBackMyPlace` harder, and
`RoleDowngraded` when he loses the shirt). The youngster who received
`MentorSupport` consolidates `HeMentoredMe` — and ten years later is
measurably more likely to mentor. A generational loop, essentially free
once the ledger is fed.

### Tier 3 — variety

**3.1 · The tournament year.** The strongest variety-per-line item in the
document. The cycle maths already exists (`should_start_cycle`, tournament
year = qualifying + 2) and the competitions are real. Give the situation
`months_to_tournament` and a national standing, then let
`GetIntoTheNationalSquad` **accrue urgency** as the tournament nears. A
fringe international who is not playing will take a step down, a loan,
anything — which the resignation path today only reaches after years of
failure. Afterwards: `MajorTournamentSquad` / `NationalTeamGlory`, or
`NationalSnub` and a want that hardens for the next cycle.

Every January window in a tournament year then looks different from every
other January. That is exactly the variety being asked for, and it is
100% football.

**3.2 · The run-down.** At under twelve months, a player with a high
self-image and a warm market read may *prefer* to run the deal down and
leave free. A `weigh` on `MindOption::SignContract`, using the mandate,
the ledger's account with the club, and the market read. One of the
most-felt mechanics in the sport, and it needs nothing that 0.4 does not
already build.

**3.3 · The comeback.** Tap `process_injury`. A long absence should
temporarily lower his *own* expectation — which removes the artefact of a
player returning from six months out and immediately resenting the bench.
Repeated serious injuries consolidate `InjuriesHaveDefinedMe`, and that
conviction should re-point what he wants: security over money
(`SecureMyFuture` above `BePaidWhatImWorth`), and a lower bar for
`RetireOnMyTerms`.

**3.4 · Crossing the divide.** `rivalry_intensity` exists. A player with
`SpiritualHome` or `FansAdoredMe` at club A should refuse — or heavily
discount — a move to A's rival, as a named `MindStance::Refuse` the
newspaper can print. And a player who *did* cross carries it permanently:
the mood layer's `ColdShoulderOverRivalPast` gets a memory that never
fades.

### Tier 4 — make it visible

Without this, none of the above reaches the user.

**4.1 · A "Mind" panel on the player page**, beside the existing morale
and happiness factors:

- **What he wants.** `GoalStack::public_goals()` (Voiced and Pressing)
  to everyone; `Active` ones only through the club's own staff view or a
  scout report — a genuine information asymmetry, and the `Active` rung
  is *designed* to be the silent one. Render the status, the deadline
  ("has given the club until January") and the evidence atoms.
- **What he remembers here.** `recall(RecallCue::Club(this_club))` — the
  convictions and the two or three strongest episodes, as sentences.
  This is the ten-year payoff, finally on screen.

**4.2 · The ledger on the relations page.** `player/relations/` and the
ego-graph views already exist. The four-axis `ActorAccount` (trust,
warmth, debt, respect) is a natural second edge type: who he rates, who
he does not, and who he owes. Nearly free.

**4.3 · Reasons into the desks.** The `WeightedReason` set from 0.4 is
already shaped for `transfer_reason_localization`, the Decisions register
and `newspaper_system`. "Returning to the club where he made his name" or
"unconvinced by a manager who broke his word" prints without inventing
anything — which is the newspaper rule (never print an unsourced figure).

---

## 5. What I deliberately did not propose

The episode catalog already contains `Bereavement`, `ChildBorn`,
`FamilySettled`, `FamilyUnsettled` and the goal `SettleMyFamily`. **None
of them is emitted for a player today**, and I am not proposing to wire
them. Nothing above adds a non-football life event, an off-pitch
scandal, a hobby, or a personal-finance system.

The one edge worth a decision: *a family that will not settle abroad* is
the single commonest stated reason a foreign signing goes home, and the
sim already expresses that outcome as `GoalKind::GoHome`, driven by
language, isolation and compatriots — i.e. **the football-visible version
of it is already there without the life-sim.** The recommendation is to
leave those four kinds unemitted and let `GoHome` carry it.

---

## 6. Order of work, and how each step is proved

The repo's own method — additive, parallel-run, census-gated — applies
throughout. Nothing below deletes a legacy path.

| # | Step | Gate |
|---|---|---|
| 1 | 0.1 `run_squad_minds` + manager wired | `.dev/mind`: professional-faculty coverage > 0; world tick inside the 8 ms/day budget |
| 2 | 0.2 emit sites, in narrative-weight order | `.dev/mind`: episodes per senior, empty-minds share, conviction spread |
| 3 | 0.3 personality into the faculties | `.dev/mind`: goal-kind spread *by personality band* — the point is dispersion, not a mean |
| 4 | Tier 1 (arrival), Tier 2 (the long stay) | `.dev/transfers`: move plausibility and request volume must not move beyond the noise floor |
| 5 | 0.4 deliberation, one call site | acceptance-rate census before/after, per phase 5 |
| 6 | Tier 3, Tier 4 | i18n locale parity for every new goal / fact / reason key |

Two standing traps from the existing log, still live:

- **No situation, no thinking.** A neutral `MindSituation` reads to
  `CompetitiveMind` as a man getting his minutes, and silently satisfies
  wants. Any new field must have a "no view" value that is not "good
  news".
- **The census must print raw distributions on both sides.** A bias of
  +48 reads identically whether the mind is high or the legacy layer is
  low, and they want opposite fixes. The first census run got this wrong.

---

## 7. If only three things get built

1. **0.1 + 0.3** — the squad-level pass, and personality in the
   faculties. Turns on a whole faculty and makes every existing rule
   produce different answers for different men. Smallest change, largest
   effect.
2. **2.1** — the plateau, and wanting a better teacher rather than a
   bigger badge. The one proposal that adds a genuinely new shape to the
   transfer market.
3. **4.1** — the Mind panel. Everything else is invisible without it.

---

## 8. Implementation log

Everything in §4 below Tier 4's optional halves is built. Suite:
**3 918 core tests passing** (3 890 at the start of the work), 161 web,
`cargo check --workspace` clean, `cargo fmt` clean. No calibrated test
moved.

### Tier 0 — the lights are on

**0.1 · `SquadStandingViewBuilder`** (`club/team/squad_life/standing_view.rs`).
The sibling of `SquadSocialViewBuilder`, run from the same weekly
`Team::run_weekly_pass`, writing a `SquadStandingView` onto every player:
who picks the side, his rank in the queue at his position, the man
directly in front of him (or, for the man at the top, the one chasing
him), that man's age, the gap between them, the armband, his place in the
pay order, and the best coaching the club has for someone in his
position.

Every comparison is `AbilityEstimator::observable_level`, never the
hidden ability digit — the rule in `feedback_hidden_potential_ability`
applies to a player judging a teammate exactly as it does to a coach.
Ties break on id so the ranking is stable across ticks; a rank that
flickered would read to the mind as losing and winning his place every
week.

**`MindSituation` grew from 17 fields to 32** and, more to the point,
they are now *fed*. `manager` is no longer hard-coded to `NONE`, which
is the single line that had kept `ProfessionalMind` from executing a
rule in a live world.

**0.2 · Emit sites.** The player-side episode vocabulary went from 15
kinds to roughly 40:

| Where | Kinds |
|---|---|
| `end_of_period` (season + cup + playoff) | `WonLeagueTitle` · `WonDomesticCup` · `Promoted` · `Relegated` |
| `TeamCollection::ensure_coach_state` | `ManagerArrived` · `ManagerLeftClub` |
| `injury/processing.rs` | `SeriousInjury` · `CareerThreateningInjury` · `ReturnedFromLongInjury` |
| `events/role.rs` | `WonStartingPlace` · `LostStartingPlace` |
| `CaptaincyAssigner::set_official_captain` | `CaptaincyAwarded` · `CaptaincyRemoved` |
| `world_status.rs` | `FirstCap` · `MajorTournamentSquad` · `NationalSnub` |
| `TransferExecution::new_signing_threats` | `ManagerSignedARival` · `MentorSupport` |
| `Player::remember_recent_mood` (weekly) | 18 more, listed below |

The last row is the one worth explaining. Most of the remaining emit
sites are player-scoped and do not know which club he is at — the
mismatch phase 1b kept running into. `remember_recent_mood` sweeps the
week's `HappinessEvent` log at the weekly tick, where the club id and the
date are both in hand, and files the ones a career remembers:
`ContractRenewed`, `ReleasedByClub`, `BigPayRise`, `WageBelowPeers`,
`ClubRefusedTerms`, `ManagerPublicPraise`, `ManagerPublicCriticism`,
`ManagerPrivateBacking`, `ManagerFrozenOut`, `RoleDowngraded`,
`StartedBigMatch`, `LeftOutOfBigMatch`, `WelcomedBySquad`,
`FeltIsolated`, `TeammateBefriended`, `TeammateConflict`,
`SquadBackedHim`, `SquadTurnedOnHim`, `MediaAttack`,
`ClubServantMilestone`.

This reads the **same gate**, not a second one: an event is in the log
because the happiness layer already decided it happened, with its own
calibrated condition and its own cooldown. The window is `1..=7` days
rather than `0..=7`, and that is what stops double-filing — the weekly
think runs every seventh day, so each event lands in exactly one window.

**0.3 · Personality reaches the faculties.** `loyalty`, `professionalism`
and `temperament` are on the situation, exposed as continuous drives
(`loyalty_drive`, `diligence`, `volatility`, `thin_skinned`) so rules
scale by them instead of branching on a threshold. They now decide: how
hard a man fights for his place, how long he waits before concluding
silence means something, whether long service becomes an anchor or an
itch, whether a veteran mentors his replacement or resents him, and
whether he will run a contract down.

**0.4 · The mind answers.** `PlayerMind::deliberate(MindOption)` fans out
to all five faculties, each returning *named* reasons. The order —
competitive, professional, career, social, financial — decides who is
heard when a decision draws more than the six a `ReasonSet` holds:
for a footballer the shirt usually is the argument, and money is almost
never the loudest voice and almost never silent.

Also added: `Recall::inspect`, the read-only twin of `Recall::cue`.
Reading a man's memory on a web page must not rehearse it, or a player
who happens to be popular would never forget anything.

### Tier 1 — the arrival

- **The mandate** is `MindSituation::standing_expectation`: the greater
  of the role he was promised and where he sits in the pay order. A
  dressing room believes the wage packet, and so does he — so
  `playing_time_gap` now measures a club-record earner against a
  club-record earner's expectation, and the same eight quiet games are a
  promising start for a squad addition and a crisis for the marquee
  signing.
- **The man in his shirt** is `blocked_unfairly`, which weighs the gap
  running his way against whether waiting is worth anything. A boy of
  twenty behind a thirty-four-year-old is queuing; a man of twenty-eight
  behind his equal is being wronged. Identical minutes, opposite
  conclusions — asserted end to end.
- **The sponsor.** `ProfessionalMind::signed_by`, stamped at
  `TransferExecution::sign_into_main_team`. When that man leaves,
  `lost_his_advocate` latches and `WinTheManagersTrust` forms at 0.75
  instead of 0.45, carrying `GoalEvidence::LOST_HIS_ADVOCATE`. The
  opposite case is the happier one, and it is now separated: a player who
  was *written off* by the departing manager has the heat taken out of
  wanting away, which is the commonest way an out-of-favour career is
  rescued.

### Tier 2 — the long stay, and wanting to get better

- **The plateau he can feel.** `CareerMind` holds two EMAs of his
  observable level — one fast (~2 months), one slow (~1 year) — and the
  distance between them is whether he is improving. A career-long 7 out
  of 10 reads as standing still, which is what it is. Thirty flat weeks
  and he has concluded it.
- **Three rungs where there was one.** `KeepImproving` →
  `WorkWithABetterCoach` → `StepUpToABiggerClub`. The first two are
  `GoalDirection::Neutral` — answerable *in place*, by a coaching
  appointment or a role change — which is the whole point: a model with
  only the third turns every plateau into a sale.
  `MindSituation::coaching_shortfall` measures the bench against the
  player rather than in the abstract, so a bench of 12s is a fine
  education for a third-division full-back and a dead end for an
  international.
- **Three ways to be a long-serving player.** The `ambition >= 12` gate
  is replaced by a fork on loyalty × ambition × how the club has treated
  him. The new middle man is `PlayWithBetterPlayers` — a want that
  existed in the catalog with no rule that formed it — held for a season
  with a real deadline, and flipping to `FindANewChallenge` only if the
  club never answers.
- **Recognition.** `BecomeAClubLegend`, formed by five years' service and
  answered by the armband. The one grievance in the catalog a club cannot
  buy its way out of.
- **Succession, both ways.** A veteran with a good young player behind
  him forks on professionalism and temperament: mentor (which starts him
  thinking about `MoveIntoCoaching`) or resist (which puts a fault line
  in the room). `HoldOntoMyPlace` is the mirror of `WinBackMyPlace` —
  defending a shirt is not the same state as chasing one.

### Tier 3 — variety

- **The tournament year.** `NationalTeamCompetitions::months_to_next_tournament`
  derives the clock from the cycle arithmetic rather than from fixtures
  (the fixtures for a tournament two years out do not exist yet, and the
  players thinking about it do). The continent stamps it once per tick
  onto every country context beneath it; the player reads it beside his
  own `NationalStanding`, derived from the `Int` status and his caps.
  `tournament_pressure` is squared so the pressure genuinely belongs to
  the last year, and it sharpens `PlayFirstTeamFootball` — which is what
  empties benches every January of a World Cup year.
- **The run-down.** `RunDownMyContract` takes three things together: a
  deal genuinely running down, a reason not to re-sign, and enough
  standing in the pay order that he expects somebody to want him.
  Loyalty is the brake, and that is most of what loyalty means in a
  contract negotiation.
- **Frozen out is not benched.** `GoalBlocker::FrozenOut` holds
  `WinBackMyPlace` rather than pursuing it — there is no competition to
  win because he is not in it — and routes the escalation to
  `BeAllowedToLeave`, which is somewhere it can actually go.
- **The comeback.** A new injury is noticed by watching the counter
  `set_injury` already increments, so there is one severity model rather
  than two that can disagree. Three weeks out is a `SeriousInjury`, three
  months is `CareerThreateningInjury`, and a knock is not a memory at all.

### Tier 4 — visible

A **Mind panel** on the player's personal page:

- **What he wants** — every live goal, loudest first, with its rung on
  the ladder, its pressure, any date he has privately given it, and any
  blocker. The `Latent` and `Active` rungs render italic and muted under
  a "not said out loud" treatment, because being able to see a want a
  season before it becomes a transfer request is the point of the panel.
- **What he remembers here** — his convictions about this club, warm or
  sour, over a one-line read of how he feels about the place. Built with
  `PlayerMind::inspect`, so looking does not rehearse.

119 i18n keys × 9 locales, inserted without churning the existing
bundles; `assert_key_parity` and `assert_prose_is_translated` both green.

### The census, measured

`.dev/mind`, 21 days of the real daily tick over **41 513 senior
players**. Against the 60-day baseline recorded in `mind_census`:

| | before (60 days) | after (**21** days) |
|---|---|---|
| episodes per senior, mean | 0.85 | **4.52** |
| episodes, p90 / max | 3 / 25 | **8 / 24** |
| ledger accounts, mean | 0.29 | **3.30** |
| **empty minds** | **22.1 %** | **0.7 %** |
| faculty coverage | 0.18 | **0.50** |
| `MoodProfile` raw | mean 50.00, **max 50** | mean 47.25, p50 48, **max 54** |
| bias vs morale | −10.11 | −9.22 |

Five times the episodes on a run **less than half as long** — about a
fifteen-fold rate increase — and the empty-mind share, which is the
honest read on emit-site coverage, is effectively gone.

The line that matters most is the `MoodProfile` one. The first census
found it at **exactly 50.00 for all 42 227 seniors**: the five faculties
had never contributed a non-zero value anywhere in the simulation. It is
now a distribution. That was the finding phase 4b was stuck on, and it
was never a tuning problem.

**Read honestly, three of these numbers are run-length artefacts, not
results:**

- **Convictions are still ~0.01 per senior.** Consolidation is monthly
  and this run is 21 days; almost nothing has had a chance to
  consolidate. The 60-day comparison is the one to make, and it has not
  been run.
- **The ladder reads Latent 88.1 % / Active 11.5 % / Voiced 0.4 % /
  Pressing 0 %**, against Latent 34 % / Active 61 % / Voiced 3.9 % at 60
  days. The ladder climbs one rung per weekly review, so at three reviews
  nothing *can* be past Active. This says nothing about the shape yet.
- **The staff half reads zero episodes and zero tenure** because no
  manager has been sacked in three weeks. Pre-existing, and untouched by
  this work.

**Phase 3b is unchanged in character and slightly better:** `Req` vs
`GoalStack::is_pressing` agrees 99.2 %, and **mind-only is now 0** — the
stack demands nowhere the sim does not. A swap-over could still only
reduce transfer requests, never invent them.

**Budget.** 17.2 s/day for the whole world tick against the ~21 s/day
this harness recorded before the work, so the weekly standing pass —
one `observable_level` per player per week — is not visible above
run-to-run variation. It is the number to watch if the pass ever grows;
the ≤8 ms/day mind budget in `docs/player_mind.md` §10 still wants an
A/B against a build with the mind compiled out, which no run has done.

### Five defects the work turned up

1. **`ProfessionalMind` had never executed a rule in a live world** —
   §2.1, and the reason for `run_squad_minds`.
2. **`MindGoal::pressure` could never reach `press_at` for
   `RunDownMyContract`** as first written, because the spec test bounds
   `press_at` to 0..=1 and 1.10 was a way of saying "never". Rewritten as
   0.95: a player *does* eventually tell the club he will not re-sign,
   and pretending otherwise was the lazier model.
3. **`blocked_unfairly` had its age term inverted** — as first written it
   said a boy behind a veteran was in the *most* hopeless position rather
   than the least. Caught by the test that asserts the opposite, which is
   why that test is written as a comparison between two men rather than a
   threshold on one.
4. **`GoalEvidence` filled its `u32` exactly.** The 29 atoms it shipped
   with used all 32 bits minus three; the squad-standing, development and
   international rules each needed atoms. Widened to `u64`, which costs
   four bytes a goal and forty-eight a player — paid deliberately, and the
   `GoalStack` size guard is what stops it being paid again by accident.
5. **`months_to_next_tournament` was three years late on every call** as
   first written, because a forward-only walk over the cycle skips the
   current one: a tournament two years out had its qualifying draw two
   years *ago*. The window reaches backwards now.

### What is still open

- **Beliefs (phase 2).** `EncodingInputs::surprise` is still the literal
  `0.5` at every emit site. Everything in §2.3 about personality is
  fixed; the other half of that finding — that every player encodes the
  same event at the same depth modulo relevance — is not.
- **`DroppedToBench` / `LeftOutOfBigMatch` from the omission path.**
  `MatchSelectionContext` carries neither a club id nor a date, and the
  squad selector that builds it has neither in scope. The weekly mood
  sweep catches the big-match half through `BenchedForBigMatch`; ordinary
  rotation is still untapped.
- **Deliberation has no consumer yet.** `PlayerMind::deliberate` is built
  and tested; converting `resolve_personal_terms` to it is phase 5's
  remaining half and wants its own `.dev/transfers` acceptance-rate
  census before and after, per §6.
- **The ledger on the relations page** (4.2) and **reasons into the
  desks** (4.3) are not built.
