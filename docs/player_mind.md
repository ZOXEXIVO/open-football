# PlayerMind — a global mind with sub-minds

Design + migration plan for consolidating every "what does this player
want, remember, believe and decide" behaviour into one owned system.

Target: `src/core/src/club/player/mind/`

---

## 1. What exists today

The player's psychology is real and large, but it is spread across four
layers that don't know about each other.

| Layer | Where | LOC | What it does |
|---|---|---|---|
| Mood | `player/happiness/` | ~7 500 | `PlayerHappiness` — morale, 13 factors, a decaying event log, 204 `HappinessEventType` variants with 27 structured context payloads |
| Desire | `player/transfer/processing.rs`, `big_stage_pull.rs` | ~3 800 | Detectors that emit "wants X" moods and recompute `TransferRequestReason` from scratch each tick |
| Adaptation | `player/personality/adaptation.rs` | ~4 800 | Settling, isolation, transfer shock, environment story |
| Relationship | `player/events/manager_relationship.rs`, `interaction.rs`, `rapport.rs`, `team/behaviour/` | ~15 000 | Manager arc, talks, promises, dressing room, conflicts |

Plus the pieces already shaped like a mind:

- `Staff::coach_memory` → `CoachMemory` / `CoachDecisionEngine`
  (`club/staff/coach/`) — the **proven pattern in this repo**: persistent
  per-subject memory + a stateless engine returning scored, *explained*
  assessments. `PlayerMind` should be its mirror image on the player side.
- `PsychologyState` (`match/engine/psychology/`) — in-match confidence /
  nervousness. Correctly separate; the mind feeds it, doesn't absorb it.
- `PlayerPlan` (`squad/plan.rs`) — note this is the **club's** plan for the
  player, not the player's own. Keep the name distinction sharp.

Scale to respect: `core` is 413 k LOC with 3 543 tests, 1 039 of them in
`club/player/`. 105 files mention `happiness`. Nothing is serialised —
`Player` derives only `Debug, Clone` — so **adding state costs no save
migration**. That is the single biggest de-risking fact in this plan.

---

## 2. Why it needs replacing — five structural problems

**2.1 Desires are events, not intentions.** A desire fires a
`HappinessEvent` behind a cooldown, decays over `event_decay_halflife_days
= 60`, and is dropped entirely at `event_retention_days = 365` /
`recent_events_cap = 100`. Nothing persists. `process_transfer_desire`
rebuilds its reason set from live ground truth every single week:

```rust
// transfer/processing.rs — re-derived from scratch, every tick
let mut active_reasons: Vec<TransferRequestReason> = Vec::new();
```

So a player cannot *hold* an intention. He cannot decide "I'll give it
until January". He can only re-notice the same grievance 52 times a year.

**2.2 The 41-variant desire sprawl.** `TransferRequestReason` (12) +
`CareerDesireKind` (11) + `LifeSimulationDesireKind` (18) are three
parallel enums describing the same thing — a want — each with its own
detector, cooldown and escalation rule. Adding a 42nd means touching all
three layers.

**2.3 No memory of people.** `recent_events` carries an optional
`partner_player_id` and nothing else. There is no record of *who did what
to me*. A manager who broke a promise is forgotten in 60 days. A club that
sold him against his will leaves no trace. This is the gap the user named:
**a player returning to a club after ten years must remember it.**

**2.4 Emotion comes from raw events, not from surprise.** Being left out
hurts a `KeyPlayer` more than a backup — today that is hand-coded at each
emit site (`PlayingTimeFrustrationConfig::expected_start_share`,
`min_eligible_matches_for_status`, …). There is no single place where
*expectation* lives, so every new event re-invents the comparison.

**2.5 Decisions are scattered rolls, not deliberation.** Acceptance logic
lives in `negotiations.rs` (a hard willingness floor plus a
probability roll), `mailbox/handlers/contract_proposal.rs`,
`free_agent_market.rs`, `availability_market.rs`, `stalemate.rs`. Each
re-derives the player's priorities from raw fields. None of them can
consult a goal the player actually holds.

---

## 3. Target architecture

One global mind. Named, concrete sub-minds inside it — no `dyn`, no
registry, explicit fan-out (the world tick is CPU-bound; see
`simulator_parallelization_audit` and the 162 ms/day budget).

```
Player
└── mind: PlayerMind                       // the global mind
    ├── organs: MindOrgans                 // shared state every sub-mind reads/writes
    │   ├── memory:  MindMemory            // §4 — the centrepiece
    │   ├── goals:   GoalStack             // §5
    │   ├── beliefs: MindBeliefs           // §6
    │   └── mood:    MindMood              // re-homed PlayerHappiness
    │
    ├── career:       CareerMind           // §3.2 — sub-minds
    ├── social:       SocialMind
    ├── professional: ProfessionalMind
    ├── financial:    FinancialMind
    └── competitive:  CompetitiveMind
```

```
src/core/src/club/player/mind/
├── mod.rs                  PlayerMind, MindTickContext, MindSnapshot, re-exports
├── organs/
│   ├── memory/             episode.rs, semantic.rs, ledger.rs, consolidation.rs,
│   │                       recall.rs, forgetting.rs
│   ├── goals/              goal.rs, stack.rs, catalog.rs, escalation.rs
│   ├── beliefs/            self_image.rs, club_read.rs, manager_read.rs,
│   │                       market_read.rs, standing_read.rs, surprise.rs
│   └── mood/               (PlayerHappiness moves here, path preserved by re-export)
├── career/                 ambition.rs, trajectory.rs, stage.rs
├── social/                 belonging.rs, dressing_room.rs, family.rs
├── professional/           manager_read.rs, role.rs, training_attitude.rs
├── financial/              worth.rs, envy.rs, terms.rs
├── competitive/            self_belief.rs, big_match.rs, drive.rs
└── deliberation/           engine.rs, option.rs, verdict.rs, reason.rs
```

### 3.1 The sub-mind contract

Every sub-mind implements the same four verbs. Declared as a trait for
discipline; called through concrete fields for speed.

```rust
pub trait SubMind {
    /// Interpret something that happened. Writes episodes, updates its
    /// own state, may revise beliefs. Called per event, not per tick.
    fn observe(&mut self, ep: &MindEpisode, organs: &mut MindOrgans);

    /// Periodic thinking. Forms, strengthens, satisfies and abandons
    /// goals; revises beliefs against evidence. Weekly for most,
    /// monthly for slow sub-minds.
    fn reflect(&mut self, ctx: &MindTickContext<'_>, organs: &mut MindOrgans);

    /// This sub-mind's contribution to current mood. Replaces the
    /// per-factor `calculate_*` functions in happiness/processing.rs.
    fn appraise(&self, organs: &MindOrgans) -> MoodContribution;

    /// This sub-mind's opinion on a decision the player faces.
    /// Returns weighted, named reasons — never a bare number.
    fn weigh(&self, opt: &MindOption, organs: &MindOrgans) -> ReasonSet;
}
```

`observe` / `reflect` / `appraise` / `weigh` is the whole API surface. A
new sub-mind is: one folder, one field on `PlayerMind`, four methods, and
its name added to the fan-out. That is the flexibility requirement, made
structural.

### 3.2 What each sub-mind owns

| Sub-mind | Owns | Absorbs from today |
|---|---|---|
| **CareerMind** | Ambition, trajectory, stage of life, where he wants to be in three years | desire detectors in `transfer/processing.rs`, `big_stage_pull.rs`, `lifecycle.rs::CareerStageDetector`, `happiness/expectation.rs` |
| **SocialMind** | Belonging, dressing room standing, language, culture, family | `adaptation.rs` integration/isolation, `language.rs`, `SquadSocialView`, `LifeSimulationDesireDetector`, `happiness/types/squad/` |
| **ProfessionalMind** | His read of the manager, role clarity, promises, training attitude | `events/manager_relationship.rs`, `interaction.rs`, `rapport.rs`, `verify_promises`, `calculate_role_clarity` / `_coach_credibility` / `_promise_trust` |
| **FinancialMind** | Wage expectation, sense of worth, envy, contract terms, agent greed | `calculate_salary_factor`, `contract/agent.rs`, `mailbox/handlers/contract_proposal.rs`, `pending_contract_ask`, wage envy in `team/behaviour/morale.rs` |
| **CompetitiveMind** | Self-belief, form confidence, big-match will, drive, rivalry | `personality/form.rs`, `load.rs` form, big-match decision, the `MoraleRatingShaper` inputs |

`CompetitiveMind` is also the **match seam**: it exports a
`MindSnapshot` the match engine reads to seed `PsychologyState` (initial
confidence, big-match nerve, grudge against this specific opponent) and
which absorbs the match result back as episodes. The match AI is not
touched — see §9.

---

## 4. The memory system

This is the centrepiece and the part that must be right first, because
everything else reads from it.

**Requirement:** ten years later, at the same club, he remembers the
place and the people — without storing ten years of events.

The answer is the same one the brain uses: **you do not keep the
episodes, you keep what they taught you.** Three stores, one
consolidation pass, cued retrieval.

### 4.1 Three stores

```rust
pub struct MindMemory {
    /// Specific events. Bounded, salience-ranked, forgetting curve applied.
    episodes: EpisodeStore,        // cap 32, of which 6 protected "flashbulb" slots
    /// Distilled, timeless facts. What survives the decade.
    semantic: SemanticStore,       // cap 24
    /// Running account per person and per club. Never fully forgotten.
    ledger: AttributionLedger,     // cap 16 people + 16 clubs
    /// Career landmarks. Never decay at all.
    milestones: MilestoneStore,    // cap 12
}
```

**Episodic** — "on 14 March 2029 the manager left me out for the derby
and told the press I was unfit."

```rust
pub struct MindEpisode {
    pub kind: EpisodeKind,          // closed enum, ~60 variants
    pub when: EpochDay,             // u16 days since world start
    pub who: ActorRef,              // Staff(id) | Player(id) | Club(id) | Board(id) | Fans(id) | None
    pub where_club: ClubId,         // tag for club-cued recall — the 10-year hook
    pub valence: i8,                // -100..+100
    pub encoding: u8,               // strength at the moment it happened (§4.2)
    pub last_recalled: EpochDay,    // rehearsal resets the curve
    pub recall_count: u8,
    pub flags: EpisodeFlags,        // Flashbulb | Formative | Unresolved | Verified | Betrayal
}
```

24 bytes packed (`ActorRef` keeps its own 4-byte alignment).

**Semantic** — "Ajax is where I broke through." "That manager's word is
worth nothing." "England never suited me."

```rust
pub struct SemanticFact {
    pub subject: ActorRef,
    pub claim: FactClaim,           // closed enum: BrokeThroughHere, NeverTrustedMe,
                                    // FansTurnedOnMe, WasSoldAgainstMyWill,
                                    // CountryNeverSuitedMe, WonEverythingHere, …
    pub strength: u8,               // how firmly held — grows with corroboration
    pub formed: EpochDay,
    pub support: u8,                // how many episodes fed it
}
```

16 bytes. **These do not decay on a forgetting curve.** They soften only
when contradicted. This is the ten-year mechanism.

**Ledger** — the running account with each person and club. Persists
after the episodes that created it are gone.

```rust
pub struct ActorAccount {
    pub actor: ActorRef,
    pub trust: i8,        // will he keep his word            -100..+100
    pub warmth: i8,       // do I like him
    pub debt: i8,         // did he do something for me / to me
    pub respect: i8,      // do I rate him professionally
    pub last_contact: EpochDay,
}
```

Slow linear drift toward zero (~1 point per 90 days), **floored by any
supporting `SemanticFact`**. A grudge backed by "sold me against my will"
never fully fades; a mild dislike does.

Total footprint: **1 680 bytes per player**, fixed and inline. No allocation after
construction if the stores are fixed-size arrays.

### 4.2 Encoding — how strongly it lands

Not every event is remembered equally. Encoding strength at the moment of
the event:

```
encoding = intensity × relevance × (0.5 + surprise)
```

- `intensity` — the event's own emotional weight (magnitude, already in
  `MoraleEventCatalog`).
- `relevance` — how much this touches an *active goal*. Being left out
  matters enormously to a man whose goal is `FirstTeamFootball`, barely at
  all to a settled veteran.
- `surprise` — prediction error against `MindBeliefs` (§6). Getting
  dropped when you believed the manager rated you is what you remember;
  getting dropped when you expected it is not.

This one formula fixes problem 2.4 in a single place, and it is *why*
players remember different things about the same season.

### 4.3 Forgetting — a power law, not an exponential

Deliberately **not** the current `1.0 - days/60` linear ramp to zero.

```rust
/// Wickelgren power-law retention. β is the personality-modulated
/// forgetting rate. Power law, not exponential, because it has a heavy
/// tail — which is precisely why a ten-year-old memory can still be
/// there. An exponential curve reaches zero and there is nothing left
/// to recall.
fn retention(&self, days: f32, beta: f32) -> f32 {
    self.encoding as f32 / 100.0 * (1.0 + days).powf(-beta)
}
```

- `beta` baseline ≈ 0.18. Ten years (3 650 days) retains ≈ 24 % of
  encoding strength — faded but present, which is exactly right.
- `beta` is modulated continuously by personality — higher
  `professionalism` and `consistency` slow forgetting; `temperament`
  slows the forgetting of *negative* episodes specifically (a hot-headed
  man nurses grievances).
- **Rehearsal**: every recall bumps `encoding` and resets the clock.
  Returning to a club rehearses everything tagged to it — which is why
  the memories come flooding back.
- **Flashbulb**: `EpisodeFlags::Flashbulb` (debut, first trophy,
  relegation, career-threatening injury, forced sale, title won) sets a
  retention floor of 0.55 and is exempt from eviction. Six slots.

No thresholds, no cliffs — continuous curves throughout, per
`feedback_realistic_not_hacks`.

### 4.4 Consolidation — the pass that makes ten years cheap

Runs monthly. Two jobs:

1. **Abstraction.** *n* similar episodes about the same actor collapse
   into one `SemanticFact` with `support = n`. Eight `FansHostile`
   episodes at one club become the fact `FansTurnedOnMe` — and then the
   eight episodes are free to fade. One strong episode (`WasSoldAgainstMyWill`)
   can form a fact on its own if its encoding is high enough.
2. **Eviction.** Once `episodes` is at cap, the lowest live-retention
   non-flashbulb episode is dropped — *after* consolidation, so its
   meaning has already been banked.

This is what lets a 34-year-old carry a whole career in 1 KB: he has
forgotten almost every match, and remembers what they added up to.

### 4.5 Recall — cued, not scanned

Memory is never iterated linearly by callers. It is *cued*:

```rust
pub enum RecallCue {
    Club(ClubId),           // arriving at, or facing, a club
    Person(ActorRef),       // meeting a manager / teammate again
    Country(CountryId),
    Situation(EpisodeKind),
    Anniversary(EpochDay),
}

impl MindMemory {
    /// Everything this cue brings back, strongest first, with recall
    /// bias already applied. Rehearses what it returns.
    pub fn recall(&mut self, cue: RecallCue, today: EpochDay, mood: f32)
        -> RecallResult;
}
```

`RecallResult` carries the episodes, the semantic facts, and the ledger
accounts the cue touches — that is the whole "he remembers this place"
API, and it is O(32) worst case.

Two realism multipliers, one line each:

- **Mood-congruent recall.** Low mood biases retrieval toward negative
  episodes and vice versa. A miserable player genuinely does remember the
  bad times more readily.
- **Reconsolidation drift.** Each recall nudges `valence` a little
  toward the player's disposition (`loyalty` warms it, `temperament`
  sours it). Ten years on, a loyal man's memory of a club is warmer than
  the events were — nostalgia, emergent and free.

### 4.6 Worked example: the ten-year return

1. 2026–2029 at Ajax. ~40 salient episodes accrue: debut (flashbulb),
   first goal, a title, a fallout with the manager, being sold.
2. Consolidation banks: `Ajax → BrokeThroughHere (strength 90)`,
   `Ajax → WonEverythingHere (72)`, `Coach#412 → NeverTrustedMe (61)`,
   `Ajax → WasSoldAgainstMyWill (80, flashbulb episode retained)`.
   Ledger: `Ajax { warmth +64, debt +40 }`, `Coach#412 { trust −70 }`.
3. 2029–2036 elsewhere. Ajax episodes fade to ~0.3 retention; the two
   flashbulbs hold at their floor. The **facts and the ledger do not
   move**.
4. 2036, an Ajax offer arrives. `recall(Club(ajax))` returns: warmth +58
   (drifted up — he is loyal, and he has been remembering it fondly),
   `BrokeThroughHere`, `WonEverythingHere`, the flashbulb of his debut,
   and `WasSoldAgainstMyWill`.
5. `FinancialMind` accepts a lower wage than his market rate.
   `CareerMind` scores a step *down* as acceptable. `SocialMind` skips the
   settling penalty — he knows the place. `ProfessionalMind` checks
   whether `Coach#412` is still there; if he is, `trust −70` (softened by
   seven years' drift to ≈ −42) hard-blocks the move.
6. The verdict carries all of it as named reasons, so the newspaper desk
   and the Decisions register can print *"returning to the club where he
   made his name"* without inventing anything.

None of step 5 or 6 is expressible today.

---

## 5. Goals — intentions that persist

Replaces `TransferRequestReason` (12) + `CareerDesireKind` (11) +
`LifeSimulationDesireKind` (18) + `big_stage_inclination` with one model.

```rust
pub struct MindGoal {
    pub kind: GoalKind,             // closed enum, ~30 variants
    pub origin: GoalOrigin,         // which sub-mind formed it, and why
    pub formed_on: EpochDay,
    pub strength: f32,              // 0..1 — how much he wants it
    pub urgency: f32,               // 0..1 — rises as the window closes
    pub progress: f32,              // 0..1 — how satisfied it is
    pub status: GoalStatus,
    pub deadline: Option<EpochDay>, // "by January"
    pub review_on: EpochDay,        // next deliberate re-think
    pub evidence: [GoalEvidence; 4],
    pub blocked_by: Option<GoalBlocker>,
}

pub enum GoalStatus {
    Latent,     // he feels it; says nothing; it does not yet shape decisions
    Active,     // silently shapes every decision (what big_stage_inclination does today)
    Voiced,     // a mood event fires; the manager can now talk about it
    Pressing,   // formal request / ultimatum
    Satisfied,  // achieved — banks a positive episode
    Frustrated, // deadline passed unmet — banks a negative episode, hardens beliefs
    Abandoned,  // he let it go — age, resignation, a better goal
}
```

The status ladder **is** the escalation, and it is stateful. That single
change fixes problem 2.1: a player can now hold "I'll give it until
January" as a `Goal` with `deadline` and `review_on`, behave coherently
for four months, and *then* escalate — instead of re-rolling the same
grievance weekly.

**The catalog makes it extensible.** Following the existing
`MoraleEventCatalog` precedent, each `GoalKind` gets a data row, not
control flow:

```rust
pub struct GoalSpec {
    pub kind: GoalKind,
    pub formation: FormationRule,     // what makes it appear
    pub decay_per_month: f32,
    pub voice_at: f32,                // strength needed to speak up
    pub press_at: f32,                // strength needed to demand
    pub satisfied_by: SatisfactionRule,
    pub abandon_after: Option<Months>,
    pub competes_with: GoalMask,      // mutually exclusive goals
    pub i18n_key: &'static str,
}
```

Adding a 31st goal = one row + one i18n key + one test. The
`i18n_sync_contract` locale-parity test already enforces the key exists in
every locale.

---

## 6. Beliefs — a world model that can be wrong

Today the player reads live ground truth every tick. A mind holds
*beliefs*, which may be stale or false — and falsifying one is a
**betrayal**, which is far better narrative than a factor moving.

```rust
pub struct MindBeliefs {
    pub self_image:    SelfImage,     // what level do I belong at, what am I worth
    pub club_read:     ClubRead,      // ambition, trajectory, will they back me
    pub manager_read:  ManagerRead,   // does he rate me, is his word good
    pub market_read:   MarketRead,    // am I wanted, by whom, at what level
    pub standing_read: StandingRead,  // where do I sit in this dressing room
}

pub struct Belief<T> {
    pub value: T,
    pub confidence: f32,      // 0..1
    pub updated: EpochDay,
    pub expectation: T,       // what he predicts next
}
```

Two consequences worth the whole section:

**`SelfImage` is the intelligence dial.** The player's own estimate of his
ceiling and worth — deliberately *not* his true PA (see
`feedback_hidden_potential_ability`; it is seeded from
`PotentialEstimator` and then drifts). Inflated by low `professionalism`
and high `controversy` and a good run; deflated by a long benching. One
field produces: unrealistic wage demands, refusing good moves, accepting
bad ones, and the classic "he thinks he's better than he is" character.

**Surprise drives emotion.** `surprise = |outcome − expectation| ×
confidence`. It feeds §4.2 encoding *and* mood magnitude. One
implementation, every event site benefits, and problem 2.4's hand-coded
per-site expectations all collapse into it.

---

## 7. Deliberation — decisions with reasons

Mirrors `CoachDecisionEngine` exactly: stateless engine, borrows the
organs, returns a scored and *explained* verdict.

```rust
pub enum MindOption {
    JoinClub(ClubOffer), SignContract(ContractOffer), AcceptLoan(LoanOffer),
    RequestTransfer, AcceptRole(PlayerSquadStatus),
    RespondToTalk(ManagerInteractionTopic, ManagerInteractionTone),
    Retire(RetirementReason), StayAndFight,
}

pub struct MindVerdict {
    pub stance: MindStance,           // Refuse | Reluctant | Open | Keen | Desperate
    pub score: f32,                   // -1..+1
    pub reasons: [WeightedReason; 6], // named, localisable, ranked
    pub counter: Option<MindCounterOffer>, // "I'd sign for X, with a release clause"
}
```

`MindDeliberation::weigh` fans out to all five sub-minds, folds their
`ReasonSet`s against the live `GoalStack`, applies the memory ledger for
every actor named in the option, and returns the verdict.

Call sites that become thin wrappers over it:
`negotiations.rs::resolve_personal_terms` (keeping its hard floor as a
`MindStance::Refuse` case), `mailbox/handlers/contract_proposal.rs`,
`free_agent_market.rs`, `availability_market.rs`, `stalemate.rs`,
`manager_talks.rs`.

The reasons plug straight into existing plumbing —
`transfer_reason_localization`, `decisions_register_coverage`,
`newspaper_system` — so explanations reach the UI without new rendering
work.

---

## 8. Extension points

Deliberate, so the system stays cheap to grow:

| To add… | You touch |
|---|---|
| A new want | one `GoalSpec` row + one i18n key + one test |
| A new remembered event | one `EpisodeKind` variant + its encoding weight row |
| A new long-lived belief about someone | one `FactClaim` variant |
| A new decision the player faces | one `MindOption` variant; sub-minds already answer `weigh` |
| A whole new faculty | one folder + one field on `PlayerMind` + four methods |
| A new match-side signal | one field on `MindSnapshot` |

---

## 9. Migration — six phases

Additive throughout. Old paths preserved by re-export, exactly as the
`engine_submodule_layout` refactor did. Full suite green at every phase
boundary.

**Phase 0 — Skeleton and seam.** Create `mind/` with organs and empty
sub-minds. `mind: PlayerMind` on `Player`; `PlayerMind::tick` called at
the top of the weekly block in `Player::simulate` doing nothing.
`PlayerBuilder` initialises it. Add `.dev/mind` harness (mirroring
`.dev/simulate`) that runs N seasons and dumps a mind census.
*Green: 3 543 tests, zero behaviour change.*

**Phase 1 — Memory.** Implement §4 in full. Feed it by tapping the
existing emit sites: wherever `add_event_with_context` fires, also record
a `MindEpisode`. Read by nobody yet. Ship the ten-year return as a test
(§4.6 as an integration test over a simulated career).
*Green + memory census: episodes/player, facts/player, footprint.*

**Phase 2 — Beliefs and surprise.** Implement `MindBeliefs`. Route
encoding strength through §4.2. Still additive — mood still computed the
old way; assert the new surprise term correlates with the existing
per-site expectation hacks before switching anything.

**Phase 3 — Goals.** Implement `GoalStack` + catalog. Map today's 41
desire variants onto `GoalKind` rows. Run **both** systems in parallel for
one phase, with a test asserting the goal stack reaches the same
`Req`/no-`Req` verdict as `process_transfer_desire` on a fixed corpus.
Then delete the old path and let `TransferRequestReason` become a view
over `GoalStack`.
*This is the highest-risk phase — it touches the calibrated transfer
suites. The parallel-run assertion is what makes it safe.*

**Phase 4 — Sub-minds absorb appraisal.** Move `happiness/processing.rs`'s
13 `calculate_*` factors into the matching sub-mind's `appraise`.
`PlayerHappiness` moves under `organs/mood/`, path preserved by
re-export. Morale arithmetic unchanged — same numbers, new home. Guard
with `morale_breakdown` snapshot tests before and after.

**Phase 5 — Deliberation.** Convert the six decision sites in §7 to
`MindDeliberation`, one at a time, each behind its own before/after
acceptance-rate census on `.dev/transfers`.

**Phase 6 — Match seam.** `MindSnapshot` seeds `PsychologyState`
(confidence, big-match nerve, grudge vs this opponent); match result
returns as episodes. **The match AI itself is not touched** — the
calibrated baselines in `match_engine_calibration_baseline` must not
move. Verify with `dev_match` on the standard n ≥ 400 corpus.

---

## 10. Verification

- **Suite**: 3 543 core tests green at every phase boundary. Re-baseline
  first — see `core_suite_rng_flakiness` (thread-id in seeded streams);
  capture the pre-change baseline before judging any failure.
- **`.dev/mind` census** (new): after 10 simulated seasons — episodes and
  facts per player, memory footprint, goal-status distribution, how many
  players hold ≥ 1 `Voiced` goal, ten-year-recall hit rate on returning
  transfers.
- **`.dev/transfers`**: acceptance rates, request volume, move
  plausibility before/after Phase 3 and 5. These must not move more than
  the noise floor.
- **`.dev/simulate`**: world-tick cost. Budget for the mind is **≤ 8 ms/day**
  on top of the current 162 ms/day. Fixed-size stores and cued recall are
  what buy that.
- **`dev_match`**: unchanged match statistics after Phase 6, n ≥ 400.
- **i18n**: locale parity + prose tests per `i18n_sync_contract` for every
  new goal / fact / reason key.

## 11. Risks

| Risk | Mitigation |
|---|---|
| Phase 3 breaks calibrated transfer behaviour | Parallel-run both systems with an equivalence assertion before deletion |
| Memory footprint × world population | Fixed-size arrays, packed fields, 1 680 B/player, asserted by a size test |
| World tick slows down | `reflect` is weekly/monthly not daily; recall is cued; no allocation after construction; hard 8 ms/day budget |
| 105 files touch `happiness` | Phase 4 moves the file, not the API — re-exports keep every path |
| Match calibration drifts | Phase 6 is read-only into the match engine; `dev_match` n ≥ 400 gate |
| Enum sprawl returns | Catalog tables (`GoalSpec`) make new wants data, not code |

---

## 12. Implementation log

### Phase 0 — skeleton and seam ✅

- `mind: PlayerMind` on `Player`, initialised by `PlayerBuilder` and by
  the two direct struct literals (`player/generators/generator.rs`,
  `country/national/synthetic.rs`).
- `PlayerMind::tick` called from `Player::simulate`'s weekly block,
  ahead of everything that reads the player.
- `Player::mind_context(now, club_id)` gathers the tick inputs in one
  place, so every emit site records against the same personality and the
  same club tag.
- `PlayerMind` is `Copy` — a `Player::clone` (which the simulator does
  constantly) copies the mind rather than chasing pointers.

### Phase 1 — the memory organ ✅

`src/core/src/club/player/mind/` — 5 197 lines, 94 tests.

| Module | Lines | Holds |
|---|---|---|
| `organs/memory/episode.rs` | 761 | 66-variant catalog + `EncodingInputs` |
| `organs/memory/recall.rs` | 720 | cued retrieval, mood congruence, reconsolidation |
| `organs/memory/consolidation.rs` | 663 | episode → conviction rules, the monthly pass |
| `organs/memory/semantic.rs` | 586 | 25 `FactClaim`s, corroboration / contradiction |
| `organs/memory/ledger.rs` | 542 | four-axis standing accounts, floored drift |
| `organs/memory/mod.rs` | 449 | `MindMemory` facade, `record` / `recall` / census |
| `organs/memory/forgetting.rs` | 262 | the power-law curve |
| `organs/memory/store.rs` | 253 | `FixedStore<T, N>` — inline, allocation-free |
| `organs/memory/actor.rs` | 184 | `ActorRef` |
| `organs/memory/epoch.rs` | 121 | `EpochDay` / `MindClock` |
| `mind/mod.rs` | 296 | `PlayerMind`, `MindTickContext` |
| `mind/integration.rs` | 321 | end-to-end tests over a real `Player` |

**Measured, not estimated:**

- `MindEpisode` = 24 B; `MindMemory` = **1 680 B**, asserted by a test.
- β = 0.18 baseline → ten years retains ≈24% of encoding; the
  `FAINT` line at 0.12 means a trivial event is gone inside two years
  while anything encoded above ≈0.5 survives a decade. Importance decides
  longevity, from one number.

**Two findings the tests caught, both real:**

1. `EncodingInputs::strength` used an unbounded relevance ratio that
   saturated against the 0..1 clamp, collapsing "he wanted this badly"
   and "he wanted this badly and never saw it coming" into the same
   memory. Replaced with bounded multipliers centred on 1.0.
2. `Ledger::prune` ran with no conviction floor, so a grudge that every
   *read* correctly preserved was silently deleted by the next monthly
   housekeeping pass. It now takes the semantic store. Guarded by
   `pruning_never_deletes_an_account_a_conviction_is_holding_up`.

### Phase 1b — emit-site taps 🟡

Wired:

- `Player::verify_promises` → `ManagerPromiseKept` / `ManagerPromiseBroken`
  against `made_by_staff_id`, with the promise's importance × credibility
  × public weighting carried into `relevance`. This is the betrayal path
  and the one that exercises every organ: episode → ledger → conviction →
  a grudge that outlives its evidence.

Not yet wired (each needs a club id plumbed to the site):

- `record_senior_debut` → `SeniorDebut`. `MatchOutcome` carries `date`
  and `played_for: Option<MatchTeamRef>`, but `MatchTeamRef` has no club
  id — that is the plumbing to add.
- Transfer events → `SignedForClub`, `SoldAgainstWill`, `ReleasedByClub`.
- Match events → `DerbyWin`, `CostlyError`, `ManOfTheMatch`, `SentOff`.
- Team season events → `Relegated`, `Promoted`, `WonLeagueTitle`.

Memory is strictly additive at every tap: the existing `HappinessEvent`
still fires unchanged, asserted by
`a_promise_from_nobody_in_particular_still_registers_but_files_against_no_one`.


### Phase 3 — the goals organ ✅

`mind/organs/goals/` — 2 993 lines, 78 tests. Mind module total: 8 669
lines, 172 tests.

| Module | Lines | Holds |
|---|---|---|
| `stack.rs` | 770 | `GoalStack`, the weekly review, competition |
| `goal.rs` | 574 | one intention: strength, urgency, progress, status |
| `catalog.rs` | 552 | 33 `GoalKind`s + `GoalSpec` rows |
| `escalation.rs` | 413 | the ladder, with hysteresis |
| `evidence.rs` | 316 | origin, 29 evidence atoms, blockers |
| `bridge.rs` | 312 | the parallel-run mapping to the legacy enums |

**The ladder.** `Latent → Active → Voiced → Pressing →
Satisfied · Frustrated · Abandoned`. One rung per weekly review, climbing
on the full bar and falling only after a clear drop below it. `Active` is
the rung that does not exist today for anything but
`big_stage_inclination`: a want that shapes every decision he makes while
nobody has heard him say a word.

**The 41-variant sprawl is addressed.** Twelve `TransferRequestReason`
values collapse onto ten wants, and the model is wider than the enums it
replaces — 33 kinds covering things the legacy set could not express at
all (`WinTheManagersTrust`, `SecureMyFuture`, `RetireOnMyTerms`).

**Two organs, coupled.** `MindOrgans::relevance_for` feeds the goal
stack straight into `EncodingInputs::relevance`, so what a player
currently wants decides what brands itself on him. Asserted end to end:
being left out encodes ~40% deeper for a man chasing first-team football
than for a settled one. This closes phase 2's relevance term early —
surprise still waits on beliefs, which is the honest reading, since
without an expectation there is nothing to be surprised against.

**One finding, and it was a design gap rather than a test artifact.**
Nothing ever raised `urgency`, so the timing term sat at its 0.65 floor
forever and every `press_at` bar was unreachable — no goal could have
become a formal demand. Fixed with `MindGoal::accrue_urgency`: waiting is
itself a form of pressure, rising on a saturating curve (~60% at 400 days)
and overridden by a nearer deadline. It is also the more truthful model —
a man who has wanted the same thing for two seasons is not in the same
state as one who decided last week, at identical strength.

That produced a property worth keeping: **ambition alone never produces a
formal demand.** At maximum strength and zero urgency a `SelfDrive` want
reaches `Voiced` and stops. Getting to `Pressing` takes a grievance
(`escalation_bias` 1.15) or it takes time. Both routes are tested.

### Phase 3b — the parallel run 🟡

`process_transfer_desire` now feeds the goal stack from the same
`active_reasons` it already computes, via `Player::feed_goals_from_reasons`.
The legacy path is untouched and still owns `Req`; nothing downstream
reads the stack. Two things the reason enum cannot carry are supplied at
the same site:

- **A closing window.** `PlayFirstTeamFootball` and
  `StepUpToABiggerClub` take urgency from age (nothing at 24, total by
  the mid thirties); `BePaidWhatImWorth` from the contract running down.
- **A date he gave himself.** A newly-formed first-team-football want
  commits until the next window rather than escalating on the spot.

The designed disagreement between the two models, asserted rather than
glossed: `Req` clears the day the last reason stops firing, while the
matching goal *fades* — a fortnight of quiet barely dents it, a year
resolves it. That difference is the point of the migration and the reason
the switch-over needs its own phase.

**Still to do before the old path can be deleted:** an equivalence census
over a real simulated corpus (`.dev/mind`), comparing `Req` set/clear
against `GoalStack::is_pressing` per player-week. The unit-level
equivalence is covered (`feeding_the_legacy_reasons_produces_a_pressing_want`,
`the_desire_tick_feeds_goals_without_touching_the_legacy_verdict`); the
population-level check is not.


### Phase 4 — the sub-minds ✅

The piece of the architecture that did not exist yet: five faculties
inside the global mind, each implementing the same four verbs. Mind
module total: 12 239 lines, 250 tests.

| Faculty | Lines | Owns |
|---|---|---|
| `professional.rs` | 519 | his read of the manager — a *belief*, which can be wrong |
| `career.rs` | 509 | trajectory, stage of life, and the time left |
| `social.rs` | 499 | belonging, isolation, language, the pull of home |
| `competitive.rs` | 443 | self-belief, the run of form, his place in the side |
| `financial.rs` | 423 | what he thinks he is worth, and security |
| `submind.rs` | 239 | the `SubMind` contract, `MoodContribution`, `ReasonSet` |
| `situation.rs` | 230 | `MindSituation` — the ground truth he cannot know from inside |

**The contract.** `observe` (interpret an event as it lands) · `reflect`
(the periodic think: form wants, revise the reading) · `appraise`
(contribute to how he feels) · `weigh` (opinion on a decision — default
empty until phase 5, so the trait is already final).

Concrete named fields, not `dyn` behind a registry: the set is known at
compile time and the world tick is CPU-bound.

**What this buys that phase 3 could not.** Goals were previously only
ever fed by the legacy desire bridge. The faculties now form them from
their own reading of the world — and produce behaviour the legacy
detectors cannot express:

- A benched player **tries to win his place back first**. Only once he
  has a long run out of the side *and* self-belief below zero does it
  become `PlayFirstTeamFootball` — the one want in the catalog pointing
  at a smaller club, and now genuinely earned rather than assumed.
- **A change of manager can rescue a career.** `ProfessionalMind` holds a
  read of a *specific person*; a successor inherits nothing, and the
  fresh start itself forms `WinTheManagersTrust`.
- **A grudge stays with the man, not the badge.** The memory ledger keeps
  the old manager's account; the new one starts clean.
- **Career and social faculties genuinely oppose each other.** Ambition
  forms `StepUpToABiggerClub` while long service and fond memories form
  `StayAtThisClub`, and the competition rules net them off. Without the
  social counterweight a simulated league is nothing but churn.
- **Being misused is separate from being left out.** A player starting
  every week with no idea what he is being asked to do forms
  `PlayInMyBestRole` and nothing else.

**Three defects found, all real, none cosmetic:**

1. **Decay compounded.** `MindGoal::decay` recomputed the span from
   `last_fed` on every review, so nineteen weekly reviews applied roughly
   nineteen times the intended fade and silently abandoned wants that
   should still have been there. Now takes the span since the last decay.
2. **Decay then stalled.** With that fixed, strength stored as a u8
   percentage meant a 0.95%-per-review fade rounded back to the same
   integer below ~53 — a want nothing fed for four years sat at half
   strength forever. Strength is now u16 basis points.
3. **Urgency ratcheted against a fading want.** Age-accrued urgency rose
   forever while strength fell, so a grievance nobody was feeding pressed
   harder every year. Urgency is now capped by strength: a want cannot be
   more pressing than it is wanted.

A fourth was a design trap rather than a bug: `tick()` with no situation
originally let the faculties reflect on a *neutral* one, which the
competitive mind reads as a player getting the minutes his role implies —
quietly satisfying wants the caller knew nothing about. **No situation,
no thinking**; there is a test for it.

### Phase 4b — appraisal parity 🟡

`PlayerMind::appraise` returns a `MoodProfile`: five
`MoodContribution`s, each carrying a value *and a confidence*, so
"nothing to go on" is distinguishable from "no problem" — something a
flat `morale = 50` cannot express. `coverage()` reports how much of a
player is actually being read.

**It runs alongside `PlayerHappiness`, not instead of it**, and there is
a test asserting morale is untouched. What can honestly be claimed today
is *directional* agreement. Full numeric parity with the 13 calibrated
factors in `happiness/processing.rs` is the remaining work, and it needs
a population-level census rather than unit tests — the same shape as the
phase-3b equivalence check that is still outstanding.

### Suite state

`cargo test -p core --lib` → **3 785 passed, 0 failed, 6 ignored**
(3 543 at the start of this work). `cargo check --workspace` clean. No
calibrated test moved in any phase.

### Phase S0 (staff plan) — the organs moved ✅

`player/mind/organs/` now lives at **`club/mind/organs/`**, shared with
`StaffMind`. Every path in this document still resolves:
`club::player::mind::organs` is a re-export, and so is every
`super::organs::…` inside a faculty. The 250 tests this document
describes ran unchanged across the move, which is what proved it.

Three things came out with the organs, because none of them was ever
player-specific: `MoodContribution`, `ReasonSet` and `MindOption` are now
`club/mind/verdict.rs`.

One change is visible from the player side. `EpisodeKind::consolidates_to`
takes a `MindHolder`, carried on `MemoryContext`, because the same
episode means different things to the two people it happened to — a
title is `WonEverythingHere` to the man who played in it and
`IBuiltSomethingThere` to the man who picked the side. Player behaviour
is untouched: `MindTickContext::memory()` tags `MindHolder::Player` and
every player rule is exactly the rule described above.

See `docs/staff_mind.md` for the staff side.

### The `.dev/mind` harness ✅

Phases 1b, 3b and 4b all stop at the same sentence — "needs a
population census rather than unit tests". That census now exists:
`.dev/mind`, built like `.dev/transfers`, driving the real daily tick
and then walking every player and every member of staff.

What it answers for the player side:

- **Is the memory organ actually being fed?** Episode, flashbulb,
  conviction and ledger-account spreads per senior, plus an
  **empty-minds** count. A season where a large share of seniors still
  hold nothing is phase 1b's remaining taps showing up as a number
  rather than as a TODO.
- **Phase 3b, as a confusion matrix.** `Req` against
  `GoalStack::is_pressing`, reported *both* / *legacy only* / *mind
  only* / *neither*. An agreement percentage would hide the direction of
  the disagreement, which is the whole question: `mind-only` means the
  stack would demand where the sim does not.
- **Phase 4b, as bias and error.** `MoodProfile::as_morale` against
  `PlayerHappiness::morale`, on the same 0..100 scale, split into signed
  bias, mean absolute error, and the share inside ±10 / ±20. A run that
  is systematically five points high is a constant to remove; one that
  is unbiased and noisy is a model that disagrees. `as_morale` exists
  for this and nothing in the live sim reads it.
- **The goal ladder**, so a distribution collapsed onto one rung is
  visible rather than inferred.

`docs/staff_mind.md` §12 has the staff half of the same report.

### The first census run 🔬

60 days of the real engine over 42 202 senior players. The full report
and the staff half are in `docs/staff_mind.md` §12; what it said about
this document:

**Phase 1b was worse than "some taps outstanding".** Zero episodes, zero
convictions, zero ledger accounts across the whole senior population
after two months. `PlayerMind::remember` had exactly **one** live emit
site in the simulation — `verify_promises` — and
`PlayerMind::on_club_change` had **none**, so a player carried his old
belonging and his read of the old manager into his new club. The memory
organ described above, with its forgetting curve and its consolidation
rules and its 94 tests, was doing nothing because nothing spoke to it.

Both are now wired at `Player::complete_transfer` — the one place that
holds both club ids and the date, which is the plumbing 1b was waiting
on. A move lays down `SoldAgainstWill` against the selling club when he
was not asking to go, `SignedForClub` against the buying club, and
closes the spell in between, in that order.

**Phase 3b is answered, and the answer is yes.** `Req` against
`GoalStack::is_pressing`:

| | mind pressing | mind quiet |
|---|---|---|
| **`Req` set** | 177 | 321 |
| **`Req` clear** | 40 | 41 664 |

**99.1% agreement, erring conservative eight to one.** Forty players in
forty-two thousand are the only ones where the stack would demand and
the sim does not. A swap-over could only reduce transfer requests, never
invent them.

**Phase 4b is not yet measurable.** Printing the raw distributions
beside the parity line — which the first census did not do, and which
cost a rebuild to learn — gives:

```
  …morale, raw          mean=60.06  p50=59  p90=82  max=100
  …MoodProfile, raw     mean=50.00  p50=50  p90=50  max=50
```

`max=50`. Not near 50 — **exactly 50, for all 42 227 seniors**.
`MoodProfile::net()` is identically zero across the whole population:
the five faculties have never contributed a non-zero value in the live
simulation. That follows directly from the finding above — a faculty
reflects on a situation, but what moves its internal state is episodes,
and until the transfer tap there were none. There is nothing to
calibrate against morale yet, and the thing that unblocks it is emit-site
coverage rather than tuning.

**The ladder works on a population.** Latent 34.1% · Active 61.4% ·
Voiced 3.9% · Pressing 0.5%. Most wants silent, few said out loud,
almost none demanded — the shape it was designed for, and the first
evidence of it outside a fixture.

**Footprint, measured:** `PlayerMind` 1 964 B ⇒ 112 MB across 60 000
players.
