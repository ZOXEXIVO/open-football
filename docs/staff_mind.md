# StaffMind — a global mind with sub-minds, for the people in the dugout

Design + migration plan for the staff side of the same system built in
`docs/player_mind.md`.

Target: `src/core/src/club/staff/mind/`

The starting position is different, and that is the whole shape of this
plan. The player had a rich mood layer and no memory of anyone. The
staff has the opposite: a real, working **memory of players** and a
**decision engine** — cited in the player plan as the proven pattern in
this repo — and almost nothing else. A manager here can tell you what he
thinks of every player he has coached, and nothing at all about the club
that sacked him.

---

## 1. What exists today

| Layer | Where | LOC | What it does |
|---|---|---|---|
| Coach memory | `staff/coach/memory.rs` | 878 | `CoachMemory` per **player**: rating EMAs, streaks, four trust signals, sticky professionalism read, 5-flag bitset |
| Decision engine | `staff/coach/engine.rs`, `assessment.rs`, `reason.rs` | ~900 | Stateless `CoachDecisionEngine` → scored, *explained* per-player assessments for selection and substitution |
| Squad plan | `staff/coach/plan.rs` | 541 | `CoachSquadPlan` — a standing `PlannedRole` per player, revised monthly, cleared on manager change |
| Bond | `staff/coach/bond.rs` | 1052 | `CoachPlayerBond` — the coach↔player relationship, with a breakdown |
| Perception | `staff/perception/` | 2 269 | `CoachProfile`, `PerceptionLens`, `PotentialEstimator`, `PlayerImpression`, per-coach noise and bias |
| Staff entity | `staff/model/staff.rs` | 1 548 | `Staff` — attributes, fatigue, `job_satisfaction: f32`, `recent_performance`, a 20-variant event log |
| Board side | `club/board/` | 8 920 | `ManagerRelationship` (5 trust facets), `BoardPressure` (5 gauges), `PromiseLedger`, `ManagerMarket` |
| Manager-facing decisions | `team/behaviour/manager_talks.rs`, `team/tactics/`, `training_direction.rs` | ~4 600 | Talks, formation/style, retraining, credibility |

Scale to respect: `staff/` is 9 428 LOC with 72 tests; `board/` is 8 920
LOC with 120. Seventy files mention `Staff`. As with `Player`, **nothing
is serialised** — `Staff` derives only `Debug, Clone` — so adding state
costs no save migration.

The good news is how much of this is already right. `CoachMemory` is a
*coach's interpretation* of a body of work rather than a copy of
`PlayerStatistics`; `CoachDecisionEngine` returns small signed
adjustments with named reasons rather than a bare number; `CoachProfile`
gives every coach a different lens. None of that needs replacing. It
needs a mind around it.

---

## 2. The gaps

**2.1 The coach remembers players and nothing else.** `CoachMemoryStore`
is `HashMap<u32, CoachMemory>` keyed by **player id**. There is no record
of a club, a board, a chairman, a rival manager, or an agent. A manager
sacked by a club in 2029 and offered the job again in 2036 has no memory
of the place — not of the sacking, not of the promises, not of the
supporters. `candidate_accepts_terms` decides with three booleans:

```rust
// manager_market.rs — the whole of a manager's decision to take a job
salary_uplift || prestige_uplift || (ambition_bonus && req_rep > current_rep)
```

This is the same gap the player plan closed, on the other side of the
touchline.

**2.2 What `CoachMemory` holds is a form ledger, not a memory.** EMAs,
streaks and trust scalars, decayed by inactivity. There are no
*episodes* (a specific thing on a specific day), no *convictions* (what
it all added up to), no forgetting curve, and no attribution. A coach
cannot hold "this player let me down in a final" — only that his
`big_match_trust` is 0.31.

**2.3 The coach has plans for players, but no goals of his own.**
`CoachSquadPlan` is his intent *for each player* — the club-facing
output. Nothing represents what **he** wants: to keep his job, to win
something here, to be backed in the window, to get a bigger job, to be
proved right about a signing, to stop. Compare `PlayerMind`'s 33-goal
catalog against zero.

**2.4 A manager's feelings are one f32 and four nudges.**
`job_satisfaction` moves on: training effectiveness > 0.75, behaviour
Good, fatigue > 70, underpaid. Then:

```rust
// staff.rs — resignation, per tick
if rand::random::<f32>() < resignation_probability { … }
```

The player got 13 factors and 204 event types. The man in the dugout
gets four terms and a die roll.

**2.5 The board–manager relationship only exists in one direction.**
`ManagerRelationship` has five well-modelled trust facets — results,
finances, squad-building, communication, style alignment — and it lives
on the **board**. There is no corresponding read of what the *manager*
thinks of the board: whether they back him, whether they keep their
word, whether he is being set up to fail. `PromiseLedger` records what
the board promised; nothing records whether the manager believes them.

**2.6 There are two per-player memories, and one of them is homeless.**
`CoachMemoryStore` lives on `Staff` and travels with him.
`CoachDecisionState` — which holds `impressions`, `emotional_heat`,
`trigger_pressure`, `squad_satisfaction` — lives on `TeamCollection` and
is rebuilt whenever the head coach id changes. So a manager's
impressions, and two accumulators that exist precisely to have history,
are attached to the club rather than to him and are discarded when he
leaves.

**2.7 Only coaches have any interiority at all.** Scouts, physios and
directors of football have `StaffPosition`, attributes and
responsibilities. Recruitment meetings emit `TargetRecommended` /
`TargetRejected` events, but no scout holds a conviction about a player
he championed and was overruled on.

---

## 3. Target architecture

Identical in shape to `PlayerMind`, because the shape is the point:
shared organs, concrete named faculties, four verbs.

```
Staff
└── mind: StaffMind                        the global mind
    ├── organs: MindOrgans                 shared with PlayerMind (§4)
    │   ├── memory                         episodes · convictions · standing accounts
    │   ├── goals                          what he wants, and how loudly
    │   └── judgements                     his read of every player (§5) — staff-only
    └── faculties                          each: observe · reflect · appraise · weigh
        ├── ambition        where his career is going, and whether this job survives
        ├── authority       his standing with the board, the room and the stands
        ├── judgement       his read of players — the existing coach lens, re-homed
        ├── philosophy      how he believes football should be played
        └── welfare         workload, fatigue, life, and whether he still wants it
```

```
src/core/src/club/staff/mind/
├── mod.rs              StaffMind, StaffTickContext, StaffSituation, re-exports
├── ambition.rs         job security · a bigger job · winning something · stopping
├── authority.rs        board read · dressing-room standing · supporter standing
├── judgement.rs        the per-player read; absorbs CoachMemory + CoachDecisionState
├── philosophy.rs       style conviction, and how far he will bend it
├── welfare.rs          fatigue, burnout, life outside the job
└── organs/
    └── judgements/     judgement.rs, store.rs, revision.rs   (staff-only organ)
```

### 3.1 The faculties

| Faculty | Owns | Absorbs |
|---|---|---|
| **AmbitionMind** | Career stage, job security, wanting a bigger club, wanting to win something, stopping | `candidate_accepts_terms`, `wants_renewal`, `calculate_desired_salary`, `determine_resignation_reason` |
| **AuthorityMind** | His read of the board and chairman; standing in the dressing room; standing with supporters | the manager side of `ManagerRelationship` / `BoardPressure` / `PromiseLedger`, `manager_credibility.rs` |
| **JudgementMind** | His read of every player — ability, ceiling, character, trust | `CoachMemoryStore`, `CoachDecisionState.impressions`, `PerceptionLens`, `PotentialEstimator`, `CoachPlayerBond` |
| **PhilosophyMind** | How he thinks the game should be played, and how far he will compromise | `CoachingStyle`, `CoachStrategy`, `TacticalDecisionEngine`, `CoachFocus` |
| **WelfareMind** | Workload, fatigue, burnout, whether he still wants to do this | `fatigue`, `update_fatigue`, `check_resignation_triggers` |

`job_satisfaction` becomes the **output** of the five `appraise` calls
rather than a field four things nudge — the same move the player plan
makes with `PlayerHappiness`.

---

## 4. The shared organs

The memory and goal machinery built for the player is not
player-specific. Episodes, a power-law forgetting curve, consolidation
into convictions, an attribution ledger, cued recall, a goal stack with
an escalation ladder — none of that knows or cares whose mind it is in.

**Phase S0 moves it up**, path-preserving:

```
src/core/src/club/player/mind/organs/   →   src/core/src/club/mind/organs/
```

with `club::player::mind::organs` kept as a re-export, exactly as the
`engine_submodule_layout` refactor did. Mechanical, and the 250 existing
mind tests are the proof it landed clean.

**The catalogs are extended, not duplicated.** `EpisodeKind`,
`FactClaim` and `GoalKind` gain manager variants alongside the player
ones rather than being made generic. Three reasons:

- The machinery stays untouched — no generics over a catalog trait, no
  risk to a working organ.
- Much of the vocabulary is genuinely **shared**. A manager and a player
  who were both at a club the year it went down remember the same event.
  `Relegated`, `WonLeagueTitle`, `SignedForClub`, `FansHostility`,
  `Bereavement` all apply to both.
- `ActorRef` already spans `Staff · Player · Club · Board · Fans ·
  Country` in both directions. The player's memory of a manager and the
  manager's memory of that player use the same key type, pointing
  opposite ways.

### 4.1 New episode kinds (~24)

```
Career          AppointedManager · SackedByClub · ResignedFromClub
                CaretakerSpell · PromotedFromWithin · WonManagerOfTheMonth
                SurvivedARelegationFight · FailedToSurviveIt

Board           BoardKeptItsPromise · BoardBrokeItsPromise
                BoardRefusedMyTarget · BoardBackedMeInTheWindow
                BoardSoldMyBestPlayer · GivenAVoteOfConfidence
                ChairmanUndercutMePublicly

Squad           LostTheDressingRoom · SquadFoughtForMe
                PlayerRefusedToPlayForMe · PlayerRepaidMyFaith
                SignedAPlayerIWanted · SignedAPlayerIDidNotWant
                MyGambleCameOff · MyGambleBackfired

Standing        SupportersTurnedOnMe · SupportersSangMyName
                MediaWroteMeOff
```

### 4.2 New fact claims (~14)

The convictions a manager carries between jobs:

```
About a club    TheySackedMe · TheyNeverBackedMe · TheyKeptTheirWord
                IBuiltSomethingThere · TheSquadWasNeverMine
                ThatPlaceWasAGraveyard

About a board   TheirWordIsWorthless · TheyStoodByMe

About a player  HeRepaidMyFaith · HeLetMeDown · HeIsWorthBuildingAround
                IWasWrongAboutHim

About himself   IAmATeacherNotAWinner · IOnlyWorkWithMySquad
```

`IWasWrongAboutHim` is the one worth pausing on. It is the only claim in
either catalog where the subject is the holder's own past judgement, and
it is what makes a coach's perception improve over a career instead of
staying at whatever `CoachProfile` seeded.

### 4.3 The ten-year return, from the other side

Same mechanism, opposite chair. A manager offered his old club after a
decade recalls `Club(id)` and gets back: `IBuiltSomethingThere` (formed
from a title and a promotion), `TheySackedMe` (flashbulb, retained),
`TheyNeverBackedMe` (consolidated from four `BoardRefusedMyTarget`
episodes), and a `Board(id)` account still pinned at `trust −0.55` by the
conviction behind it. The `AmbitionMind` weighs the job; the
`AuthorityMind` checks whether the chairman is the same man.

---

## 5. The judgements organ — staff-only

The one organ the player does not need: a persistent, revisable read of
**other people's ability**, which can be wrong.

This is where the existing coach lens is re-homed, and where the
duplication in §2.6 is resolved. `CoachMemory` (on `Staff`) and
`CoachDecisionState.impressions` (on `TeamCollection`) become one store,
living on the manager, travelling with him.

```rust
pub struct PlayerJudgement {
    pub player: ActorRef,
    /// What he thinks the player is worth now, 0..1 of the observable band.
    assessed_level: u8,
    /// What he thinks the player will become.
    assessed_ceiling: u8,
    /// How sure he is — rises with matches observed, falls with time apart.
    confidence: u8,
    /// The four trust axes `CoachMemory` already models, kept as-is.
    tactical_trust: u8,
    big_match_trust: u8,
    training_trust: u8,
    professionalism_read: u8,
    /// Form, relative to what *this coach* expected — kept from CoachMemory.
    recent_rating_ema: f32,
    long_form_rating: f32,
    /// Accumulators that currently die with `CoachDecisionState`.
    emotional_heat: u8,
    /// Was he right? Set when the player's later career settles the
    /// question. Feeds `IWasWrongAboutHim` and slowly sharpens the lens.
    verdict: JudgementOutcome,
}
```

Two things this adds that neither existing structure has:

**Judgements survive the job.** A manager who rated a player at one club
still rates him at the next, which is how real managers sign the same
players repeatedly — and the sim currently cannot express it at all.

**Judgements can be scored.** `JudgementOutcome` closes the loop: a
coach who wrote off a player who then became very good accrues
`IWasWrongAboutHim`, which nudges his `PerceptionLens` toward patience.
A coach whose convictions keep proving right grows confident faster.
That is a coach who *learns*, and it costs one field plus a monthly
audit.

---

## 6. Goals a manager holds

New `GoalKind` rows in the shared catalog, `GoalDomain::Management`.
Same ladder — `Latent → Active → Voiced → Pressing → Satisfied ·
Frustrated · Abandoned` — with the same meaning: `Active` is the rung
where a want silently shapes every decision, and `Voiced` is where the
press hears about it.

| Goal | Points | Note |
|---|---|---|
| `KeepThisJob` | Stay | Survival. Urgency from board trust, not from the calendar |
| `WinSomethingHere` | Stay | |
| `SurviveTheSeason` | Stay | The relegation-fight variant of `KeepThisJob` |
| `GetABiggerJob` | Leave | The manager's `StepUpToABiggerClub` |
| `BeBackedInTheMarket` | Neutral | Voiced = the classic public plea for signings |
| `KeepMyBestPlayer` | Neutral | Fires when the board is listening to offers |
| `SignThePlayerIWant` | Neutral | One target, held across a window |
| `GetMyOwnSquad` | Stay | A new manager wanting to replace an inherited team |
| `ProveThemWrong` | Neutral | Formed by a sacking. Points at a specific club |
| `BeGivenTime` | Stay | |
| `RestoreOrderInTheRoom` | Stay | From `LostTheDressingRoom` |
| `GetOutOfHere` | Leave | The terminus — the manager's `LeaveThisClub` |
| `RetireFromTheGame` | Neutral | |
| `TakeANationalJob` | Leave | |

The **counterweight** matters here as much as it did for the player. Left
alone, ambition churns managers every season. `KeepThisJob`,
`WinSomethingHere` and `GetMyOwnSquad` all point Stay and compete with
`GetABiggerJob` — so a manager mid-build resists an approach he would
have taken a year earlier.

`ProveThemWrong` is the goal that only exists because memory does. It is
formed by `SackedByClub`, points at a specific `ActorRef::Club`, and
makes a fixture against that club a big match for *him*.

---

## 7. Deliberation

`MindOption` gains the decisions a manager faces, and `weigh` finally
has something to do on both sides of the touchline:

```rust
TakeTheJob(club_id)      // replaces candidate_accepts_terms' three booleans
SignThisPlayer(player_id)
SellThisPlayer(player_id)
DropThisPlayer(player_id)
ChangeTheSystem
Resign
```

`ChangeTheSystem` ships without the `TacticType` payload and
`AnswerTheBoard(BoardQuestion)` does not ship at all. Both were
speculative: the question a faculty actually answers is *whether* to
change, not *to what* — picking the formation is
`TacticalDecisionEngine`'s job and it already does it well — and no
`BoardQuestion` type exists to be answered.

Call sites that become thin wrappers: `candidate_accepts_terms`,
`wants_renewal`, `check_resignation_triggers`, `manager_talks.rs`'s
candidate scoring, `TacticalDecisionEngine::make_tactical_decisions`,
and the recruitment-meeting vote.

`CoachDecisionEngine` is **not** replaced. It already does the right
thing — scored, explained per-player assessments — and becomes the
`JudgementMind`'s `weigh` implementation for selection options. The
engine keeps its shape; the mind is what feeds it a persistent
judgement instead of a store that starts empty at every club.

---

## 8. What is genuinely different from the player mind

Worth stating plainly, because the symmetry is easy to over-read.

**A manager looks up as well as down.** The player has one manager and a
dressing room. A manager has a board above him, supporters around him,
and thirty players below. `AuthorityMind` therefore models three
standings, not one.

**A manager has an identity.** `PhilosophyMind` has no player
equivalent. A conviction about how football should be played — and how
far he will bend it under pressure — is the thing that makes two
managers with identical attributes behave differently, and it is the
axis on which `style_alignment` on the board side finally has a
counterparty.

**A manager's mind is read by far more of the simulation.** The player
mind feeds the transfer path and his own mood. The staff mind feeds
selection, substitutions, tactics, training, talks, transfers, the
recruitment meeting and the board relationship. The blast radius is
wider, which is why every phase below runs in parallel before it
switches anything over.

**The staff already has the hard part.** `CoachMemory`,
`CoachDecisionEngine` and `CoachProfile` took real work and are good.
This plan mostly gives them a home, a history, and something to want.

---

## 9. Migration — six phases

Additive throughout, old paths preserved by re-export, full suite green
at every boundary. The player-side phases proved the sequence; this one
follows it.

**Phase S0 — promote the organs.** Move `player/mind/organs/` to
`club/mind/organs/`, re-export from the old path. Extend `EpisodeKind`,
`FactClaim` and `GoalKind` with the manager rows (§4.1, §4.2, §6). No
behaviour change; 250 existing mind tests are the guard.

**Phase S1 — skeleton and memory.** `mind: StaffMind` on `Staff`,
initialised by every construction site. `StaffMind::tick` from
`Staff::simulate`. Wire the first taps — `AppointedManager`,
`SackedByClub`, `BoardKeptItsPromise` / `BoardBrokeItsPromise` from
`PromiseLedger`. Ship the manager's ten-year return as an integration
test.

**Phase S2 — the judgements organ.** Move `CoachMemory` under
`organs/judgements/`, path preserved. Fold `CoachDecisionState.impressions`
into it and move the store from `TeamCollection` onto `Staff` — this is
the phase that fixes §2.6, and it is the highest-risk one because
selection reads it. Guard with a before/after selection census on a
fixed corpus.

**Phase S3 — goals.** `GoalStack` on the staff organs. Feed it from the
existing board-relationship signals in parallel, exactly as phase 3 did
with `TransferRequestReason`: `ManagerRelationship.overall_trust` and
`BoardPressure` reinforce `KeepThisJob`; a sacking forms `ProveThemWrong`.
Nothing downstream reads it yet.

**Phase S4 — the five faculties.** `observe` / `reflect` / `appraise` /
`weigh`. `job_satisfaction` becomes a `MoodProfile` read, running
alongside the existing f32 rather than replacing it.

**Phase S5 — deliberation.** Convert the seven decision sites in §7 one
at a time, each behind its own before/after census. `candidate_accepts_terms`
is the natural first — it is three booleans, it is well-isolated, and
the manager-market census already exists to check it against.

---

## 10. Verification

- **Suite**: 3 785 core tests green at every boundary; 72 staff + 120
  board tests are the ones to watch. Re-baseline first — see
  `core_suite_rng_flakiness`.
- **`.dev/mind` census** (shared with the player work): per-manager
  episode / fact / judgement counts, goal-status distribution, memory
  footprint. Budget: **≤ 2 ms/day** on top of the current tick — staff
  are an order of magnitude fewer than players, so the ceiling is
  generous.
- **Manager-market census**: appointment rate, average tenure,
  sack-vs-resign split, before and after S5. These must not move more
  than the noise floor.
- **Selection census**: starting-XI churn and `CoachDecisionReason`
  distribution before and after S2 — the phase most able to disturb
  match results without failing a test.
- **i18n**: locale parity for every new episode / fact / goal key, per
  `i18n_sync_contract`.

## 11. Risks

| Risk | Mitigation |
|---|---|
| S2 disturbs selection | Move the store first, change nothing about how it is read; selection census on a fixed corpus |
| The organ move breaks player paths | Re-exports, and the 250 existing mind tests run unchanged |
| Catalog sprawl — one enum for two minds | Domain-tagged variants; a coverage test asserts every kind has a domain and an i18n key |
| Manager churn changes | The Stay-goals are the counterweight; manager-market census gates S5 |
| Non-coach staff left behind | Deliberately out of scope until S5; `StaffMind` works for a scout with an empty judgement store, it simply has less to say |

---

## 12. Implementation log

Phases S0–S5 are live. `cargo test -p core --lib` → **3 866 passed, 0
failed, 6 ignored** (3 785 before this work). `cargo check --workspace`
clean. No calibrated test moved in any phase.

### Phase S0 — promote the organs ✅

`player/mind/organs/` → `club/mind/organs/`, with
`club::player::mind::organs` kept as a re-export so every existing path
— and every `super::organs::…` inside a player faculty — resolves
unchanged. Mechanical; the 250 existing mind tests were the proof, and
the suite came out at exactly the count it went in at.

Two things came out with it, because they were never player-specific
either: `MoodContribution`, `ReasonSet` and `MindOption` now live in
`club/mind/verdict.rs`, and `MindOption` grew the six manager decisions
alongside the six player ones.

**The catalogs were extended, not duplicated.** 26 `EpisodeKind` rows,
14 `FactClaim` rows, 14 `GoalKind` rows, plus four staff-side
`EpisodeDomain` variants (`Management` · `Boardroom` · `Squad` ·
`Philosophy`) and five `GoalDomain` ones — one per faculty, so the mood
profile has an axis per faculty rather than one lump labelled
"management".

**One thing the plan did not anticipate.** A shared catalog runs into a
case the plan's §4.2 walks straight past: a title won at a club is
`WonEverythingHere` to the man who played in it and `IBuiltSomethingThere`
to the man who picked the side. One episode, two meanings. The fix is a
`MindHolder` parameter on `EpisodeKind::consolidates_to`, carried on
`MemoryContext` — so the reading is chosen by whose mind is doing the
consolidating rather than by a second catalog. `Relegated`,
`WonDomesticCup`, `FansHostility` and the supporter rows all divide the
same way. A test asserts the two readings never agree, because if they
did the parameter would be dead weight.

### Phase S1 — skeleton and memory ✅

- `mind: StaffMind` on `Staff`, initialised at both construction sites.
- `Staff::simulate` runs the **quiet pass** daily for every member of
  staff: the weekly goal review and the monthly consolidation. No
  situation, so no faculty reflects.
- `Club::run_manager_mind` runs the **situated pass** weekly for the
  head coach, because the club is the only place that holds the board,
  the squad and the table at once. It runs straight after
  `ensure_coach_state`, so the dressing-room reading it takes is this
  week's.
- `Staff::mind_context` / `remember` / `leave_club` are the three doors
  emit sites use, so no site builds a tick context by hand and none can
  forget the club id — which is what makes an episode cue-able by club a
  decade later.

**Taps wired (11):** `AppointedManager` (poach and free-agent
appointment), `PromotedFromWithin` (caretaker confirmed), `CaretakerSpell`,
`SackedByClub`, `BoardKeptItsPromise` / `BoardBrokeItsPromise` (from
`PromiseLedger`, via a new `BoardResult::promises_kept` counter beside
the existing `promises_broken`), `GivenAVoteOfConfidence` (the board's
public backing), `ChairmanUndercutMePublicly` (the public ultimatum),
`ContractRenewed`.

Ordering matters at exactly one of them and is asserted: the sacking is
recorded **before** `leave_club`, so it is filed against the club he was
still at.

`docs/staff_mind.md` §4.3 ships as an integration test —
`the_ten_year_return`. Thirteen simulated years, four windows of being
told no, two broken promises, a sacking, then a decade elsewhere. What
survives: `IBuiltSomethingThere`, `TheyNeverBackedMe` and `TheySackedMe`
about the club; `TheirWordIsWorthless` about the board; and a board
account still under −0.2 ten years on, because the conviction holds it
against the drift back to neutral.

### Phase S2 — the judgements organ ✅ (parallel run) 🟡 (the move)

`club/staff/mind/organs/judgements/` — `PlayerJudgement` at **28 bytes**,
`JudgementStore` 48 slots (1 348 B), fed from
`CoachMemoryObservations::apply` — the same chokepoint, the same
observations, at the same moment.

What it adds that neither existing structure has:

- **It survives the job.** `StaffMind::on_club_change` clears his
  standing with that board, room and crowd, and keeps every judgement.
  A manager who rated a player at one club still rates him at the next,
  which is how managers sign the same players repeatedly and which the
  sim could not express at all.
- **It can be scored.** `settle` compares the ceiling he held against
  what the player turned out to be, asserts `IWasWrongAboutHim`, and
  nudges his patience. A coach who has been wrong gives the next one
  longer.

Two properties worth stating because they were not obvious going in:

1. **The verdict is gated on how sure he *was*, not how sure he is now.**
   Confidence fades with time apart, and settling a question years later
   would otherwise find every old view too faint to have been wrong
   about. Whether a man was committed enough to be wrong is a fact about
   the view he held; the years since cannot un-commit him.
2. **A judgement he was never sure of teaches him nothing.** Below
   `VERDICT_CONFIDENCE` the question simply does not settle — which is
   correct, and which is why a coach's eye sharpens slowly rather than
   on every player who ever passed through.

The level he assigns is read off his own long-form rating baseline
(5.0 is a passenger, 8.0 is a very good player), so the organ is
**CA-blind by construction** — it never touches ground truth.

🟡 **What is deliberately not done.** `CoachMemoryStore` still feeds
selection, untouched, and `CoachDecisionState` still lives on
`TeamCollection`. Moving either changes match results, and the plan's
own §9 gates that on a selection census over a fixed corpus that does
not exist yet. `Club::manager_situation` *reads*
`coach_state.squad_satisfaction` rather than moving it, which gets the
signal into the mind at zero risk. §2.6 is therefore diagnosed and
half-answered: the organ that will hold it exists and is accumulating.

### Phase S3 — goals ✅

`GoalStack` on `StaffOrgans`, 14 manager rows, fed by the faculties from
the situation rather than by a bridge. Two structural properties are
asserted rather than assumed:

- **The counterweight is real.** A catalog test walks every manager goal
  and requires each Leave-goal to compete with a Stay-goal and vice
  versa. Left alone, ambition churns managers every season; this is what
  stops it, and an integration test shows a manager mid-build turning
  down an approach the same man with an inherited squad accepts.
- **Ambition alone never produces a formal demand.** The staff-side
  mirror of the player-side property. A manager over-achieving at a
  small club for three seasons is restless — `GetABiggerJob` is live —
  and has still demanded nothing.

### Phase S4 — the five faculties ✅

| Faculty | Lines | Owns |
|---|---|---|
| `ambition.rs` | 570 | job security, restlessness, investment, the sackings count |
| `judgement.rs` | 477 | his read of players, and whether it was right |
| `authority.rs` | 414 | board faith, the dressing room, the terraces |
| `welfare.rs` | 365 | strain, appetite, the end of a career |
| `philosophy.rs` | 347 | conviction, and how far he has bent from it |
| `situation.rs` | 249 | `StaffSituation` — the ground truth he cannot know from inside |

**`StaffMind` = 3 328 bytes**, entirely inline and `Copy`, asserted by a
test. Staff are an order of magnitude fewer than players.

**The one place the staff mind departs from the player mind.**
`PlayerMind::dispatch` routes each episode to exactly one faculty by
domain. `StaffMind::dispatch` fans out to all five, because a manager's
events are institutional and genuinely land on more than one of him: a
public vote of confidence is a fact about the board *and* about whether
the job survives; being sacked ends a chapter *and* takes the load off;
a title confirms his football, adds to his honours, and makes him want
another season. Every `observe` is an opt-in match, so the fan-out costs
five cheap dispatches and nothing is counted twice.

**Behaviour that only exists because these five disagree:**

- **A public vote of confidence lowers his security.** It is what a
  board says on the way to sacking someone, and a manager who has been
  sacked before reads it faster — `AmbitionMind::cynicism` rises with
  every sacking and is a real career-shaping number rather than a flavour
  field.
- **Two managers with identical attributes behave differently in the
  same crisis.** High conviction refuses to bend and takes the
  consequences; low conviction bends and ends up managing a team that is
  not his, which forms `GetMyOwnSquad` and weighs on him in a way no
  results-based reading can express. This is the axis on which the
  board's `style_alignment` finally has a counterparty.
- **The three standings move independently.** Supporters singing his
  name tells him nothing about the dressing room. Losing the room is not
  recoverable in an afternoon, and one good result does not undo it.
- **Being sacked is a rest and a blow at the same time.** Strain drops,
  appetite drops further. Strain accrues five times faster than it
  clears, so a bad season leaves a mark a quiet one does not remove —
  which is why managers take years out.
- **A career ends by degrees, not on a birthday.** `RetireFromTheGame`
  forms from age, strain and having nothing left to prove, on continuous
  curves.

`StaffMoodProfile` runs alongside `job_satisfaction` and exposes
`as_satisfaction()` on the same 0..100 scale so the parallel run can be
compared without a conversion at every call site. 🟡 Numeric parity is
the outstanding half, and it needs a population census rather than unit
tests — the same shape as the player side's phase 4b.

### Phase S5 — deliberation 🟡 (one of seven converted)

`StaffMind::deliberate` asks all five faculties and merges their
`ReasonSet`s. `candidate_accepts_terms` — three booleans, and the whole
of a manager's decision to take a job — is the first of §7's seven sites
converted.

**The conversion is conservative by construction.** The three booleans
still decide; the mind is consulted only after them, and a manager who
holds no conviction about the club produces an **empty verdict** and
gets exactly the old answer. Every manager in a fresh world is that
manager, so the manager-market baseline is preserved by construction and
the divergence grows only as careers accumulate — which is what makes
the census in §10 meaningful rather than a re-baseline. Three tests pin
it: the old answer with no history, a starved manager turning down a
doubled salary, and a club he took up being worth a flat-terms return.

🟡 The remaining six sites (`wants_renewal`,
`check_resignation_triggers`, the manager-talks candidate scoring,
`TacticalDecisionEngine::make_tactical_decisions`, the recruitment vote,
selection) are unconverted. `CoachDecisionEngine` is untouched and is
not going to be replaced — it becomes what `JudgementMind::weigh` feeds.

### Outstanding

| Item | Why it is not done |
|---|---|
| Moving `CoachMemoryStore` / `CoachDecisionState` onto the mind (S2's second half) | Changes selection; gated on a selection census over a fixed corpus |
| Numeric parity between `StaffMoodProfile` and `job_satisfaction` (S4b) | Needs a population census, not unit tests |
| The remaining six deliberation sites (S5) | Each needs its own before/after census |
| `.dev/mind` harness | Shared with the player work; both censuses above depend on it |
| Non-coach staff | `StaffMind` already works for a scout with an empty judgement store — it simply has less to say. Deliberately out of scope |
| i18n entries for the 54 new keys | Nothing renders them yet. The keys are declared and guarded (uniqueness, cross-catalog collisions, namespacing); locale rows land with the view that shows them, which is the same position the player mind is in |
| `squad_is_his` | A tenure proxy (four seasons ≈ a squad a manager built) rather than a signings count. A real per-manager signings counter belongs on the transfer pipeline |
