//! L2 — what the club knows, all year round.
//!
//! Scouting in the simulator was demand-driven: a scout was assigned to an
//! open request, watched whoever matched it, and filed a report. Close the
//! window and the department stopped existing. So a club that had no hole to
//! patch in July had no knowledge in August, and when it finally did need a
//! centre-forward it started from nothing.
//!
//! A real recruitment department carries a standing list. For each shirt on
//! the brief it keeps a handful of names within reach and within the money,
//! refreshes them every week whatever the calendar says, and sends scouts to
//! watch THOSE — so that on the first day of the window the meeting agenda is
//! already written.
//!
//! Two properties matter more than the mechanics:
//!
//! * **Lists diverge.** Reach (where a club's scouts actually go), envelope
//!   (what it can pay), role fit (what its formation asks for) and belief
//!   (`ClubOpinion`'s stable per-club misjudgement) all differ, so twenty
//!   rich clubs hold twenty different lists that overlap only on the
//!   genuinely outstanding. That is what real interest looks like, and it is
//!   the defence against the market collapsing into one auction.
//! * **Ranking reads belief, gates read truth.** Who is on the list is a
//!   judgement; whether a move is legal is a fact. The plausibility model,
//!   affordability and the seller's own protections are untouched.

use crate::transfers::MarketMap;
use crate::transfers::gate::build::Suitor;
use crate::transfers::gate::{MoveTerms, PursuitAction, TransferMovePlausibility};
use crate::transfers::scouting::ScoutingPass;
use crate::transfers::scouting::watch::FormWatch;
use crate::transfers::view::world::WorldPool;
use chrono::{Datelike, NaiveDate, Weekday};
use rayon::prelude::*;
use std::cmp::Reverse;

use crate::club::player::personality::Language;
use crate::transfers::ScoutingRegion;
use crate::transfers::loan::interest::ClubOpinion;
use crate::transfers::pipeline::processor::PlayerSummary;
use crate::transfers::pipeline::trace::{MarketSwitches, TransferTrace};
use crate::transfers::pipeline::{
    ClubTransferPlan, FileObservation, FileReading, KnownPlayerMemory, WatchAvailability,
    WatchlistEntry,
};
use crate::transfers::scouting::breakout::{BreakoutInputs, BreakoutPerformanceSignal};
use crate::transfers::scouting::recruitment::ScoutMonitoringSource;
use crate::transfers::squad::plan::BriefSlot;
use crate::transfers::squad::standing::{StandingInputs, StandingSignal};
use crate::{Club, Country, PlayerFieldPositionGroup};

/// One candidate, scored once per country so every club in it can be judged
/// against the same cheap arithmetic instead of recomputing the signals
/// forty times.
#[derive(Clone, Copy)]
struct WatchCandidate<'a> {
    summary: &'a PlayerSummary,
    /// The stronger of the two discovery reads — form and standing measure
    /// the same question from opposite ends, so the honest signal is the max.
    discovery: f32,
    availability: WatchAvailability,
    /// The facts every club tests first, kept beside the summary so the
    /// board rules most of the world out without reading it.
    player_id: u32,
    skill_ability: u8,
    age: u8,
    club_id: u32,
    estimated_value: f64,
    is_loan_listed: bool,
}

/// The country's candidates, by the shirt group they can fill and then by
/// the region they play in, so a club walks only the regions it reaches.
/// Each bucket runs strongest first, so a walk ends at the first name too
/// weak for the shirt.
struct WatchPool<'a> {
    buckets: [[Vec<WatchCandidate<'a>>; ReachMask::WIDTH]; PlayerFieldPositionGroup::COUNT],
}

impl<'a> WatchPool<'a> {
    fn of(
        candidates: impl IntoIterator<Item = (PlayerFieldPositionGroup, WatchCandidate<'a>)>,
    ) -> Self {
        let mut buckets: [[Vec<WatchCandidate<'a>>; ReachMask::WIDTH];
            PlayerFieldPositionGroup::COUNT] = Default::default();
        for (group, candidate) in candidates {
            buckets[group.index()][ReachMask::slot(candidate.summary.region)].push(candidate);
        }
        for bucket in buckets.iter_mut().flatten() {
            bucket.sort_unstable_by_key(|candidate| Reverse(candidate.skill_ability));
        }
        WatchPool { buckets }
    }

    /// The group's names in `reach`, of `weakest` ability and above.
    fn walk(
        &self,
        group: PlayerFieldPositionGroup,
        reach: ReachMask,
        weakest: u8,
    ) -> impl Iterator<Item = &WatchCandidate<'a>> {
        let buckets = &self.buckets[group.index()];
        reach.slots().flat_map(move |region| {
            buckets[region]
                .iter()
                .take_while(move |candidate| candidate.skill_ability >= weakest)
        })
    }
}

impl<'a> WatchCandidate<'a> {
    fn of(summary: &'a PlayerSummary, discovery: f32, availability: WatchAvailability) -> Self {
        WatchCandidate {
            summary,
            discovery,
            availability,
            player_id: summary.player_id,
            skill_ability: summary.skill_ability,
            age: summary.age,
            club_id: summary.club_id,
            estimated_value: summary.estimated_value,
            is_loan_listed: summary.is_loan_listed,
        }
    }
}

/// The club's standing board of names it is carrying.
pub struct Watchlist;

impl Watchlist {
    /// Weekly, year-round refresh of every club's watchlist — the pipeline's
    /// entry point into [`MarketKnowledge`].
    ///
    /// Reads the whole world snapshot rather than the foreign view: a club's
    /// standing board includes the domestic players it would sign, and the
    /// snapshot already carries them with the same summary the cross-border
    /// side reads.
    pub fn refresh_watchlists(
        country: &mut Country,
        world_pool: WorldPool<'_>,
        market_map: &MarketMap,
        date: NaiveDate,
    ) {
        MarketKnowledge::refresh(country, world_pool, market_map, date);
    }
}

/// One club's scouting reach, as a bitmask over the fifteen regions.
///
/// The reach test is the most-executed compare in the weekly pass — every
/// club runs it against a large share of the world's players — so it is the
/// one place where the difference between a hash probe and a shift is worth
/// having. Built from the same `reputation_scout_regions` ladder every other
/// pass uses, so the answer is identical, only faster to ask.
#[derive(Debug, Clone, Copy, Default)]
struct ReachMask(u32);

impl ReachMask {
    /// The enum is fieldless, so its discriminant IS the bit index. Sound
    /// while there are at most 32 regions — there are fifteen, and a
    /// sixteenth would still fit; the debug assertion below is what would
    /// catch a thirty-third.
    fn bit(region: ScoutingRegion) -> u32 {
        let index = region as u32;
        debug_assert!(index < 32, "ReachMask holds at most 32 scouting regions");
        1u32 << index
    }

    fn of(regions: &[ScoutingRegion]) -> Self {
        ReachMask(regions.iter().fold(0u32, |mask, r| mask | Self::bit(*r)))
    }

    /// One bucket per bit the mask can hold.
    const WIDTH: usize = 32;

    fn covers(self, region: ScoutingRegion) -> bool {
        self.0 & Self::bit(region) != 0
    }

    /// The bucket a region's players sit in.
    fn slot(region: ScoutingRegion) -> usize {
        region as usize
    }

    /// The buckets this mask reaches.
    fn slots(self) -> impl Iterator<Item = usize> {
        (0..Self::WIDTH).filter(move |i| self.0 & (1u32 << i) != 0)
    }

    fn union(self, other: ReachMask) -> Self {
        ReachMask(self.0 | other.0)
    }

    /// Where a club whose first team stands at `overall_score` sends its
    /// scouts.
    fn of_club(home_region: ScoutingRegion, overall_score: f32) -> Self {
        Self::of(&ScoutingPass::reputation_scout_regions(
            home_region,
            overall_score,
        ))
    }
}

/// What the country's briefed shirts could take: where their scouts go, and
/// per shirt group the weakest man any shirt could believe over its bar, the
/// widest age band and the most any shirt's money reaches. A name outside it
/// is on no board in the country, so it is never scored.
struct WatchDemand {
    reach: ReachMask,
    groups: [Option<ShirtDemand>; PlayerFieldPositionGroup::COUNT],
}

#[derive(Clone, Copy)]
struct ShirtDemand {
    weakest: u8,
    ages: (u8, u8),
    /// The dearest valuation a shirt with a fee envelope watches.
    dearest: f64,
    /// A shirt with no envelope watches who could come without a fee.
    borrows: bool,
}

impl WatchDemand {
    fn of(country: &Country) -> Self {
        let home_region = ScoutingRegion::from_country(country.continent_id, &country.code);
        let mut demand = WatchDemand {
            reach: ReachMask::default(),
            groups: [None; PlayerFieldPositionGroup::COUNT],
        };
        for club in &country.clubs {
            let Some(brief) = club.transfer_plan.brief.as_ref() else {
                continue;
            };
            if brief.slots.is_empty() {
                continue;
            }
            let Some(team) = club.teams.main().or_else(|| club.teams.teams.first()) else {
                continue;
            };
            demand.reach = demand.reach.union(ReachMask::of_club(
                home_region,
                team.reputation.overall_score(),
            ));
            let kindest = ClubOpinion::kindest(
                team.staffs
                    .resolve_for_transfers()
                    .best_scout_judging_ability(),
            );
            for slot in &brief.slots {
                let Some(weakest) = MarketKnowledge::weakest_over_bar(&kindest, slot) else {
                    continue;
                };
                let ceiling = MarketKnowledge::envelope_ceiling(slot);
                let buys = ceiling > 0.0;
                let shirt = ShirtDemand {
                    weakest,
                    ages: slot.age_band,
                    dearest: if buys { ceiling } else { f64::NEG_INFINITY },
                    borrows: !buys,
                };
                let held = &mut demand.groups[slot.group.index()];
                *held = Some(held.map_or(shirt, |held| held.widen(shirt)));
            }
        }
        demand
    }

    fn wants(
        &self,
        group: PlayerFieldPositionGroup,
        summary: &PlayerSummary,
        availability: WatchAvailability,
    ) -> bool {
        self.groups[group.index()].is_some_and(|shirt| {
            summary.skill_ability >= shirt.weakest
                && summary.age >= shirt.ages.0
                && summary.age <= shirt.ages.1
                && (summary.estimated_value <= shirt.dearest
                    || shirt.borrows
                        && MarketKnowledge::comes_without_a_fee(
                            summary.is_loan_listed,
                            availability,
                        ))
        })
    }
}

impl ShirtDemand {
    fn widen(self, other: ShirtDemand) -> Self {
        ShirtDemand {
            weakest: self.weakest.min(other.weakest),
            ages: (self.ages.0.min(other.ages.0), self.ages.1.max(other.ages.1)),
            dearest: self.dearest.max(other.dearest),
            borrows: self.borrows || other.borrows,
        }
    }
}

/// A bounded "best `capacity`" board.
///
/// The candidate pool for one shirt is the world, and only eight names ever
/// survive — so the pass keeps the eight rather than sorting sixty thousand.
/// Insertion is `O(capacity)` and the whole board fits in a cache line or
/// two, which matters because this runs per club per briefed shirt every
/// Monday. Names rank by score, then by the lower player id, so the board
/// holds the same eight in whatever order the pool is walked.
struct TopBoard<'a> {
    capacity: usize,
    /// `(candidate, score, believed ability)`, worst-to-best is NOT
    /// maintained — only the weakest is tracked, and the final sort is over
    /// at most `capacity` rows.
    rows: Vec<(&'a WatchCandidate<'a>, f32, f32, FileReading)>,
    /// `(score, player id)` of the weakest row once the board is full.
    worst: Option<(f32, u32)>,
}

impl<'a> TopBoard<'a> {
    fn new(capacity: usize) -> Self {
        TopBoard {
            capacity,
            rows: Vec::with_capacity(capacity),
            worst: None,
        }
    }

    fn outranks(a: (f32, u32), b: (f32, u32)) -> bool {
        a.0 > b.0 || (a.0 == b.0 && a.1 < b.1)
    }

    /// Whether this name would take a place on the board.
    fn admits(&self, score: f32, player_id: u32) -> bool {
        self.rows.len() < self.capacity
            || self
                .worst
                .is_some_and(|worst| Self::outranks((score, player_id), worst))
    }

    fn offer(
        &mut self,
        candidate: &'a WatchCandidate<'a>,
        score: f32,
        believed: f32,
        reading: FileReading,
    ) {
        let key = (score, candidate.player_id);
        if self.rows.len() < self.capacity {
            self.rows.push((candidate, score, believed, reading));
        } else if self.worst.is_some_and(|worst| Self::outranks(key, worst)) {
            let weakest = self
                .rows
                .iter()
                .position(|(c, s, _, _)| Some((*s, c.player_id)) == self.worst);
            if let Some(weakest) = weakest {
                self.rows[weakest] = (candidate, score, believed, reading);
            }
        } else {
            return;
        }
        if self.rows.len() == self.capacity {
            self.worst = self
                .rows
                .iter()
                .map(|(c, s, _, _)| (*s, c.player_id))
                .reduce(|w, r| if Self::outranks(w, r) { r } else { w });
        }
    }

    fn into_sorted(mut self) -> Vec<(&'a WatchCandidate<'a>, f32, f32, FileReading)> {
        self.rows.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.player_id.cmp(&b.0.player_id))
        });
        self.rows
    }
}

/// Builds and refreshes every club's standing list of names.
pub struct MarketKnowledge;

impl MarketKnowledge {
    /// Names a club carries per briefed shirt. Eight is roughly the length
    /// of a real recruitment department's per-position board: enough that
    /// losing the first choice does not end the search, short enough that
    /// the list means something.
    pub const WATCH_PER_ROLE: usize = 8;
    /// A candidate is admitted while his valuation is within this multiple
    /// of the slot's envelope. Above 1.0 because an envelope is what the
    /// club has ALLOCATED, not what it could stretch to, and because a
    /// department watches players it cannot afford today against the chance
    /// that it can in six months.
    pub const ENVELOPE_SLACK: f64 = 1.6;
    /// Longest a name stays on the board without being re-seen before it is
    /// dropped. Roughly a window plus the close season, the same staleness
    /// clock the investment watch uses on scouting memory.
    const ENTRY_FRESH_DAYS: i64 = 180;
    /// Hard ceiling on one club's whole board, across every shirt. Keeps the
    /// per-club memory bounded whatever the brief asks for.
    const MAX_ENTRIES: usize = Self::WATCH_PER_ROLE * 5;

    /// Weekly, year-round refresh of every club's watchlist.
    ///
    /// Runs on the same Monday cadence as the breakout watch, and outside
    /// the window gate for the same reason: scouts do not stop watching
    /// football in October.
    pub fn refresh(
        country: &mut Country,
        world_pool: WorldPool<'_>,
        market_map: &MarketMap,
        date: NaiveDate,
    ) {
        if date.weekday() != Weekday::Mon {
            return;
        }

        // ── One country-wide candidate pass ─────────────────────────
        //
        // The two discovery signals and the availability read are pure
        // arithmetic over the summary, so they are computed once here and
        // shared by every club. Only the belief noise and the per-club gates
        // run inside the fan-out below.
        let demand = &WatchDemand::of(country);
        let pool = WatchPool::of(
            world_pool
                .summaries()
                .iter()
                // Free agents are the free-agent market's business; a
                // watchlist is about players somebody else owns.
                .filter(|summary| {
                    summary.contract_months_remaining > 0 && demand.reach.covers(summary.region)
                })
                .flat_map(|summary| {
                    let availability = Self::availability_of(summary);
                    let mut discovery = None;
                    PlayerFieldPositionGroup::ALL
                        .into_iter()
                        .filter(move |&group| {
                            summary.coverage.covers_group(group)
                                && demand.wants(group, summary, availability)
                        })
                        .map(move |group| {
                            let discovery =
                                *discovery.get_or_insert_with(|| Self::discovery_score(summary));
                            (group, WatchCandidate::of(summary, discovery, availability))
                        })
                }),
        );

        // ── Per-club fan-out (read only) ────────────────────────────
        let country_ref: &Country = country;
        let staged: Vec<(u32, Vec<(WatchlistEntry, FileReading)>)> = country_ref
            .clubs
            .par_iter()
            .map(|club| {
                (
                    club.id,
                    Self::for_club(country_ref, club, &pool, market_map, date),
                )
            })
            .collect();

        for (club_id, entries) in staged {
            if entries.is_empty() {
                continue;
            }
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == club_id) {
                let overall_score = club
                    .teams
                    .main()
                    .or_else(|| club.teams.teams.first())
                    .map(|t| t.reputation.overall_score())
                    .unwrap_or(0.0);
                let watcher_id =
                    club.teams
                        .main()
                        .or_else(|| club.teams.teams.first())
                        .map(|team| {
                            let resolved = team.staffs.resolve_for_transfers();
                            resolved
                                .director_of_football
                                .map(|s| s.id)
                                .or_else(|| resolved.scouts.first().map(|s| s.id))
                                .unwrap_or(team.staffs.head_coach().id)
                        });
                for (entry, reading) in entries {
                    club.transfer_plan.board(reading, entry);
                }
                club.transfer_plan
                    .prune_watchlist(date, Self::ENTRY_FRESH_DAYS, Self::MAX_ENTRIES);
                if let Some(watcher_id) = watcher_id {
                    Self::follow(&mut club.transfer_plan, watcher_id, overall_score, date);
                }
            }
        }
    }

    /// Scouting follows the watchlist.
    ///
    /// A board of names is only knowledge once somebody is actually watching
    /// them: the monitoring row is what grows confidence, what puts a name on
    /// the recruitment meeting's agenda, and what leaves a dossier ready when
    /// the window opens. This opens files on the strongest names the board
    /// carries that nobody is watching yet — under exactly the caps the
    /// year-round form watch already respects, so a department's total load
    /// still scales with its reputation rather than with the length of its
    /// wish list.
    fn follow(plan: &mut ClubTransferPlan, watcher_id: u32, overall_score: f32, date: NaiveDate) {
        let monitor_cap = FormWatch::breakout_watch_monitor_cap(overall_score);
        let intake = FormWatch::breakout_watch_per_pass(overall_score);
        let mut opened = 0usize;

        let mut ordered: Vec<WatchlistEntry> = plan.watchlist.clone();
        ordered.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.player_id.cmp(&b.player_id))
        });

        for entry in ordered {
            if opened >= intake || plan.scout_monitoring.len() >= monitor_cap {
                return;
            }
            if !plan.monitorings_for_player(entry.player_id).is_empty() {
                continue;
            }
            // A meeting that has already said no blocklists him; the board
            // may keep the name, but nobody re-opens the file.
            if plan.is_rejected(entry.player_id, date) {
                continue;
            }

            let Some(reading) = plan.file_reading(entry.player_id).copied() else {
                continue;
            };
            // Confidence starts low and grows with observation, exactly as a
            // demand-driven assignment's does — a name off a data board is a
            // lead, not a report.
            if !plan.keep_file(
                reading,
                FileObservation {
                    scout_staff_id: watcher_id,
                    player_id: entry.player_id,
                    source: ScoutMonitoringSource::StaffRecommendation,
                    transfer_request_id: None,
                    origin_assignment_id: None,
                    region: None,
                    assessed_ability: entry.believed_ability,
                    assessed_potential: entry.believed_ability,
                    confidence: Self::INITIAL_CONFIDENCE,
                    role_fit: 1.0,
                    estimated_value: entry.estimated_value,
                    risk_flags: Vec::new(),
                    is_match: false,
                },
                date,
            ) {
                continue;
            }

            // For a foreigner this is the only durable record of who and
            // where he is — he sits on no roster the recommendation
            // processor can walk.
            plan.remember_known_player(KnownPlayerMemory {
                player_id: entry.player_id,
                last_known_club_id: entry.club_id,
                last_known_country_id: entry.country_id,
                position: entry.position,
                position_group: entry.position_group,
                age: entry.age,
                assessed_ability: entry.believed_ability,
                assessed_potential: entry.believed_ability,
                confidence: Self::INITIAL_CONFIDENCE,
                estimated_fee: entry.estimated_value,
                last_seen: date,
                official_appearances_seen: 0,
                friendly_appearances_seen: 0,
            });
            opened += 1;
        }
    }

    /// Confidence a name carries the day the club starts watching him. Low
    /// on purpose: the whole value of a year-round board is that the number
    /// grows before the window opens.
    const INITIAL_CONFIDENCE: f32 = 0.35;

    /// One club's board. Every gate here is cheap and ordered cheapest
    /// first — an integer compare before a float, a float before the belief
    /// hash — because this runs for every club against the whole world every
    /// Monday. A bucket is left at the first name even the department's
    /// kindest misreading would not lift over the shirt's bar; every name
    /// after him is weaker.
    fn for_club(
        country: &Country,
        club: &Club,
        pool: &WatchPool<'_>,
        market_map: &MarketMap,
        date: NaiveDate,
    ) -> Vec<(WatchlistEntry, FileReading)> {
        let plan = &club.transfer_plan;
        let Some(brief) = plan.brief.as_ref() else {
            return Vec::new();
        };
        if brief.slots.is_empty() {
            return Vec::new();
        }
        let Some(team) = club.teams.main().or_else(|| club.teams.teams.first()) else {
            return Vec::new();
        };
        let overall_score = team.reputation.overall_score();
        let judging = team
            .staffs
            .resolve_for_transfers()
            .best_scout_judging_ability();

        // Where this club's scouts actually go. The one gate that makes
        // "what a club knows" mean "as far as its network reaches".
        //
        // Held as a bitmask over the fifteen regions rather than a hash set:
        // this is the most-executed compare in the pass — every club tests
        // it against a large share of the world every Monday — and a single
        // shift-and-and is an order of magnitude cheaper than a hash probe.
        let home_region = ScoutingRegion::from_country(country.continent_id, &country.code);
        let reach = ReachMask::of_club(home_region, overall_score);
        let home_language_mask = Language::country_language_mask(&country.code);
        let suitor = Suitor::of(country, club, date);
        let kindest = ClubOpinion::kindest(judging);

        let mut entries: Vec<(WatchlistEntry, FileReading)> = Vec::new();
        for slot in brief.slots.iter() {
            let envelope_ceiling = Self::envelope_ceiling(slot);
            // A shirt with no envelope is filled on loan or on a free.
            let terms = if envelope_ceiling > 0.0 {
                MoveTerms::purchase(true)
            } else {
                MoveTerms::loan(true)
            };
            // A bounded top-`WATCH_PER_ROLE` board rather than a sorted
            // list of everyone who survived the gates. The pool is the
            // world; sorting it per slot per club every Monday is the
            // difference between a weekly pass and a weekly stall, and the
            // board only ever keeps eight names anyway.
            let Some(weakest) = Self::weakest_over_bar(&kindest, slot) else {
                continue;
            };
            let mut board = TopBoard::new(Self::WATCH_PER_ROLE);
            for candidate in pool.walk(slot.group, reach, weakest) {
                if candidate.age < slot.age_band.0 || candidate.age > slot.age_band.1 {
                    continue;
                }
                if candidate.club_id == club.id || club.is_rival(candidate.club_id) {
                    continue;
                }
                // The envelope is a real constraint on a WATCHLIST, not just
                // on a bid: a department does not keep files on players its
                // owner will never fund. A shirt with none watches the free
                // and loan end of the market: men their clubs would lend, and
                // men whose deals are running out.
                if terms.is_loan {
                    if !Self::comes_without_a_fee(candidate.is_loan_listed, candidate.availability)
                    {
                        continue;
                    }
                } else if candidate.estimated_value > envelope_ceiling {
                    continue;
                }
                // ── Belief ──
                // The club's own read of him, then everything a scout can
                // see that is not his ability: how far he stands above his
                // own league, what his club thinks of him, whether anyone
                // could actually get him, and whether he could be spoken to
                // in the dressing room.
                let opinion = ClubOpinion::of(club.id, candidate.player_id, judging);
                let believed = opinion.believed_ability(candidate.skill_ability);
                let gain = believed - slot.incumbent_level as f32;
                if gain < slot.min_gain as f32 {
                    continue;
                }
                // His record is read only for a name whose best case — the
                // shirt itself, the club's own tongue — would take a place.
                let best = Self::board_score(gain, candidate, 1.0, Self::BEST_LANGUAGE);
                if !board.admits(best, candidate.player_id) {
                    continue;
                }
                let s = candidate.summary;
                // His level and his wage against the club: two compares on
                // readings he carries.
                if !Self::fits(&suitor, s, terms) {
                    continue;
                }

                // Fit at the actual shirt, not merely somewhere in the
                // group: a winger who nominally covers centre-forward is not
                // the centre-forward the brief asked for, and must not
                // outrank one who is.
                let role_fit = if s.coverage.covers(slot.position) {
                    1.0
                } else {
                    0.6
                };
                let language = if s.country_id != country.id {
                    s.language_profile.affinity_for(home_language_mask) - 0.5
                } else {
                    0.25
                };
                let score = Self::board_score(gain, candidate, role_fit, language);
                // The full read — market reach included — only for a name
                // about to take a place.
                if !board.admits(score, candidate.player_id) {
                    continue;
                }
                let assessment = TransferMovePlausibility::read(&suitor, s, terms, market_map);
                if !assessment.reaches(PursuitAction::KeepFile.needs()) {
                    continue;
                }
                board.offer(
                    candidate,
                    score,
                    believed,
                    FileReading::of(&assessment, date),
                );
            }

            for (candidate, score, believed, reading) in board.into_sorted() {
                let s = candidate.summary;
                if entries.iter().any(|(e, _)| e.player_id == s.player_id) {
                    continue;
                }
                if TransferTrace::is(s.player_id) {
                    TransferTrace::line(
                        s.player_id,
                        "watchlist",
                        format!(
                            "club={} ({}) slot={:?} tier={:?} score={:.1} envelope={:.0} \
                             availability={:?}",
                            club.name,
                            club.id,
                            slot.position,
                            slot.tier,
                            score,
                            slot.envelope,
                            candidate.availability,
                        ),
                    );
                }
                entries.push((
                    WatchlistEntry {
                        player_id: s.player_id,
                        club_id: s.club_id,
                        country_id: s.country_id,
                        position: s.position,
                        position_group: slot.group,
                        age: s.age,
                        believed_ability: believed.round().clamp(1.0, 200.0) as u8,
                        estimated_value: s.estimated_value,
                        score,
                        availability: candidate.availability,
                        slot_position: slot.position,
                        slot_tier: slot.tier,
                        last_refreshed: date,
                        is_loan: terms.is_loan,
                    },
                    reading,
                ));
            }
        }
        entries
    }

    /// The most a shirt's valuation may reach and stay on its board. The
    /// reach, not the envelope: the transformative slot watches the whole
    /// pot's worth of names (see `BriefSlot::fee_reach`).
    fn envelope_ceiling(slot: &BriefSlot) -> f64 {
        slot.fee_reach * Self::ENVELOPE_SLACK
    }

    /// Whether a shirt with no envelope watches him: his club would lend
    /// him, or his deal is running out.
    fn comes_without_a_fee(is_loan_listed: bool, availability: WatchAvailability) -> bool {
        is_loan_listed || availability == WatchAvailability::Expiring
    }

    /// The weakest true ability the department's kindest read lifts over
    /// the shirt's bar. Belief rises with ability, so nobody below him can
    /// take a place, and a shirt with none takes nobody.
    fn weakest_over_bar(kindest: &ClubOpinion, slot: &BriefSlot) -> Option<u8> {
        let incumbent = slot.incumbent_level as f32;
        let min_gain = slot.min_gain as f32;
        (0..=u8::MAX).find(|&truth| kindest.believed_ability(truth) - incumbent >= min_gain)
    }

    /// Standing fit's level and wage reads, on the readings the summary
    /// carries — what lets the board skip a name before believing anything
    /// of him. The full read still runs for a name about to take a place.
    fn fits(suitor: &Suitor<'_>, s: &PlayerSummary, terms: MoveTerms) -> bool {
        if MarketSwitches::standing_fit_off() {
            return true;
        }
        let below = s.level_floor.below(suitor.division as f32);
        if terms.is_loan {
            return below <= 0.0 || s.seller_ctx.is_transfer_requested;
        }
        // Holding a wage holds every smaller one, so the two asks are one
        // read of the larger.
        let wage = if below < 1.0 {
            s.asking_wage
        } else {
            s.asking_wage.max(s.refusal_price)
        };
        suitor.can_hold(s, wage)
    }

    /// Weight of each axis in the belief score. Improvement dominates —
    /// a watchlist exists to find better players — but a name the club can
    /// actually get, in a role it actually plays, beats a marginally better
    /// one it cannot.
    const GAIN_WEIGHT: f32 = 2.5;
    const DISCOVERY_WEIGHT: f32 = 0.35;
    const ROLE_FIT_WEIGHT: f32 = 12.0;
    const AVAILABILITY_WEIGHT: f32 = 14.0;
    const LANGUAGE_WEIGHT: f32 = 8.0;
    /// The language read of a foreigner fluent in the club's tongue; no
    /// name reads higher.
    const BEST_LANGUAGE: f32 = 0.5;

    fn board_score(gain: f32, candidate: &WatchCandidate<'_>, role_fit: f32, language: f32) -> f32 {
        gain * Self::GAIN_WEIGHT
            + candidate.discovery * Self::DISCOVERY_WEIGHT
            + role_fit * Self::ROLE_FIT_WEIGHT
            + candidate.availability.lift() * Self::AVAILABILITY_WEIGHT
            + language * Self::LANGUAGE_WEIGHT
    }

    /// The stronger of the two discovery reads. Form travels; so does
    /// standing, and the two are structurally blind to different players —
    /// a centre-back who never scores has no breakout score and a very high
    /// standing one.
    fn discovery_score(s: &PlayerSummary) -> f32 {
        let breakout = BreakoutPerformanceSignal::compute(&BreakoutInputs {
            position_group: s.position_group,
            goals: s.goals,
            assists: s.assists,
            appearances: s.appearances,
            average_rating: s.average_rating,
            age: s.age,
            league_reputation: s.seller_ctx.league_reputation,
            is_league_top_scorer: false,
            scoring_rank: None,
            recent_award_points: 0.0,
        })
        .score;
        let standing = StandingSignal::compute(&StandingInputs {
            position_group: s.position_group,
            skill_ability: s.skill_ability,
            age: s.age,
            league_reputation: s.seller_ctx.league_reputation,
            position_group_rank: s.seller_ctx.position_group_rank,
            international_apps: s.international_apps,
            prior_season_rating: s.career_record.prior_season_rating,
            prior_season_appearances: s.career_record.prior_season_appearances,
            seasons_as_regular: s.career_record.seasons_as_regular,
        })
        .score;
        breakout.max(standing)
    }

    /// Why a club might realistically be able to get him. Read off the
    /// public signals only — a seller's own sell list is invisible from
    /// outside until the plausibility model turns it into Soft availability.
    fn availability_of(s: &PlayerSummary) -> WatchAvailability {
        if s.seller_ctx.is_transfer_requested {
            return WatchAvailability::Requested;
        }
        if s.is_listed || s.is_loan_listed {
            return WatchAvailability::Listed;
        }
        if s.contract_months_remaining > 0 && s.contract_months_remaining <= 12 {
            return WatchAvailability::Expiring;
        }
        if s.seller_ctx.in_debt {
            return WatchAvailability::SellerDistress;
        }
        if s.seller_ctx.big_stage_inclination >= Self::CIRCULATED_INCLINATION {
            return WatchAvailability::Circulated;
        }
        WatchAvailability::None
    }

    /// Pull at which a player is quietly on the market through his agent.
    /// Matches `BigStagePullConfig::inclination_bar` and the plausibility
    /// model's own agent-circulation bar, so the watchlist and the gate
    /// agree about who is gettable.
    const CIRCULATED_INCLINATION: f32 = 0.22;
}

impl WatchAvailability {
    /// How much this signal lifts a name up the board. Never a gate — the
    /// plausibility model still decides whether a move is possible — only a
    /// statement about which of two equally good players a department
    /// spends its week on.
    pub fn lift(self) -> f32 {
        match self {
            WatchAvailability::None => 0.0,
            WatchAvailability::Circulated => 0.45,
            WatchAvailability::SellerDistress => 0.5,
            WatchAvailability::Expiring => 0.7,
            WatchAvailability::Listed => 0.9,
            WatchAvailability::Requested => 1.0,
        }
    }
}

impl WatchlistEntry {
    /// True while this name still answers the shirt it was filed under.
    pub fn serves(&self, group: PlayerFieldPositionGroup) -> bool {
        self.position_group == group
    }
}

#[cfg(test)]
mod watchlist_tests {
    use super::*;
    use crate::PlayerPositionType;
    use crate::club::board::mandate::ReservationVerdict;
    use crate::transfers::gate::TransferMoveStage;
    use crate::transfers::squad::ledger::{SellListEntry, SellMotive};
    use crate::transfers::squad::plan::BriefTier;

    struct Fx;

    impl Fx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 7, 6).unwrap()
        }

        fn entry(player_id: u32, group: PlayerFieldPositionGroup, score: f32) -> WatchlistEntry {
            WatchlistEntry {
                player_id,
                club_id: 900,
                country_id: 9,
                position: PlayerPositionType::MidfielderCenter,
                position_group: group,
                age: 24,
                believed_ability: 150,
                estimated_value: 20_000_000.0,
                score,
                availability: WatchAvailability::None,
                slot_position: PlayerPositionType::MidfielderCenter,
                slot_tier: BriefTier::B,
                last_refreshed: Self::date(),
                is_loan: false,
            }
        }

        fn reading() -> FileReading {
            FileReading {
                stage: TransferMoveStage::CanCompleteMove,
                stopped_by: None,
                read_on: Self::date(),
            }
        }
    }

    #[test]
    fn a_signal_that_says_he_is_gettable_lifts_him_up_the_board() {
        // Ordered by how much it says: nothing < an agent's call < a
        // struggling seller < a running-down contract < a listing < a
        // formal request to leave.
        let ladder = [
            WatchAvailability::None,
            WatchAvailability::Circulated,
            WatchAvailability::SellerDistress,
            WatchAvailability::Expiring,
            WatchAvailability::Listed,
            WatchAvailability::Requested,
        ];
        for pair in ladder.windows(2) {
            assert!(
                pair[0].lift() < pair[1].lift(),
                "{:?} must say less than {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn the_board_is_read_back_per_shirt_strongest_belief_first() {
        let mut plan = ClubTransferPlan::new();
        plan.watchlist = vec![
            Fx::entry(1, PlayerFieldPositionGroup::Midfielder, 10.0),
            Fx::entry(2, PlayerFieldPositionGroup::Midfielder, 40.0),
            Fx::entry(3, PlayerFieldPositionGroup::Forward, 90.0),
        ];
        let midfield = plan.watchlist_for(PlayerFieldPositionGroup::Midfielder);
        assert_eq!(
            midfield.iter().map(|e| e.player_id).collect::<Vec<_>>(),
            vec![2, 1],
            "a forward never answers a midfield brief, and the stronger belief leads"
        );
    }

    #[test]
    fn a_stale_name_is_dropped_and_a_re_seen_one_is_refreshed() {
        let mut plan = ClubTransferPlan::new();
        let mut stale = Fx::entry(1, PlayerFieldPositionGroup::Midfielder, 10.0);
        stale.last_refreshed = Fx::date() - chrono::Duration::days(400);
        plan.watchlist.push(stale);

        let mut seen_again = Fx::entry(2, PlayerFieldPositionGroup::Midfielder, 10.0);
        seen_again.score = 55.0;
        plan.board(Fx::reading(), seen_again);
        plan.prune_watchlist(
            Fx::date(),
            MarketKnowledge::ENTRY_FRESH_DAYS,
            MarketKnowledge::MAX_ENTRIES,
        );

        assert_eq!(
            plan.watchlist
                .iter()
                .map(|e| e.player_id)
                .collect::<Vec<_>>(),
            vec![2],
            "a name nobody has looked at for a year is no longer knowledge"
        );
    }

    #[test]
    fn a_board_never_grows_without_bound() {
        let mut plan = ClubTransferPlan::new();
        for i in 1..=200 {
            plan.board(
                Fx::reading(),
                Fx::entry(i, PlayerFieldPositionGroup::Midfielder, i as f32),
            );
        }
        plan.prune_watchlist(
            Fx::date(),
            MarketKnowledge::ENTRY_FRESH_DAYS,
            MarketKnowledge::MAX_ENTRIES,
        );
        assert!(plan.watchlist.len() <= MarketKnowledge::MAX_ENTRIES);
        assert!(
            plan.watchlist.iter().all(|e| e.score > 100.0),
            "and it keeps the names it believes in most"
        );
    }

    #[test]
    fn a_marketed_player_is_quietly_available_and_carries_a_price() {
        let mut plan = ClubTransferPlan::new();
        plan.sell_list = vec![
            SellListEntry {
                player_id: 7,
                asking: 30_000_000.0,
                floor: 0.0,
                verdict: ReservationVerdict::Sell,
                score: 0.8,
                motive: SellMotive::PeakValue,
                marked_on: Fx::date(),
            },
            // Below the bar: priced, but the club is not ready to answer.
            SellListEntry {
                player_id: 8,
                asking: 5_000_000.0,
                floor: 0.0,
                verdict: ReservationVerdict::Sell,
                score: 0.1,
                motive: SellMotive::SurplusByPlan,
                marked_on: Fx::date(),
            },
        ];
        assert!(plan.is_marketed(7));
        assert!(!plan.is_marketed(8));
        assert_eq!(plan.asking_for(7), Some(30_000_000.0));
        assert_eq!(plan.asking_for(99), None);
    }
}

#[cfg(test)]
mod standing_board_tests {
    use super::*;
    use crate::PlayerPositionType;
    use crate::PlayerSquadStatus;
    use crate::transfers::pipeline::{TransferNeedPriority, TransferNeedReason};
    use crate::transfers::squad::plan::{BriefSlot, BriefTier, RecruitmentBrief};
    use crate::transfers::tests::kit::{TestClub, TestCountry, TestDate, TestSummary};

    struct Fx;

    impl Fx {
        fn slot(envelope: f64) -> BriefSlot {
            BriefSlot {
                position: PlayerPositionType::MidfielderCenter,
                group: PlayerFieldPositionGroup::Midfielder,
                tier: BriefTier::A,
                gap: 20,
                min_gain: 1,
                envelope,
                fee_reach: envelope,
                age_band: (16, 35),
                promised_status: PlayerSquadStatus::FirstTeamRegular,
                target_level: 110,
                baseline_level: 80,
                incumbent_level: 70,
                incumbent: None,
                reason: TransferNeedReason::QualityUpgrade,
                priority: TransferNeedPriority::Important,
            }
        }

        /// A small club in a division of 2 000 that has briefed one shirt.
        fn country(envelope: f64) -> Country {
            let mut club = TestClub::new(1).reputation(2_000).build();
            club.transfer_plan.brief = Some(RecruitmentBrief {
                slots: vec![Self::slot(envelope)],
                ..RecruitmentBrief::empty(TestDate::today())
            });
            TestCountry::new(1)
                .league_reputation(2_000)
                .clubs(vec![club])
                .build()
        }

        fn board(country: &Country, pool: &[PlayerSummary]) -> Vec<u32> {
            let pool = WatchPool::of(pool.iter().map(|summary| {
                (
                    PlayerFieldPositionGroup::Midfielder,
                    WatchCandidate::of(summary, 0.0, MarketKnowledge::availability_of(summary)),
                )
            }));
            MarketKnowledge::for_club(
                country,
                &country.clubs[0],
                &pool,
                &MarketMap::default(),
                TestDate::today(),
            )
            .into_iter()
            .map(|(e, _)| e.player_id)
            .collect()
        }
    }

    /// Improvement is ranked inside standing fit, so the man furthest above
    /// a small club is not the first name on its board.
    #[test]
    fn the_best_player_in_the_pool_is_not_the_first_name_on_a_small_clubs_board() {
        let country = Fx::country(5_000_000.0);
        let far_above = TestSummary::new(10)
            .club_id(2)
            .ability(175)
            .reputation(7_500)
            .league(8_000, 0.85)
            .salary(9_000_000)
            .value(2_000_000.0)
            .build();
        let modest = TestSummary::new(11)
            .club_id(3)
            .ability(95)
            .league(2_000, 0.2)
            .salary(80_000)
            .value(300_000.0)
            .build();
        assert_eq!(Fx::board(&country, &[far_above, modest]), vec![11]);
    }

    /// A shirt with no envelope watches the loan and free end of the market.
    #[test]
    fn a_shirt_with_no_fee_envelope_watches_only_who_could_come_without_a_fee() {
        let country = Fx::country(0.0);
        let settled = TestSummary::new(20)
            .club_id(2)
            .ability(95)
            .league(2_000, 0.2)
            .salary(80_000)
            .contract_months(36)
            .build();
        let lent = TestSummary::new(21)
            .club_id(3)
            .ability(95)
            .league(2_000, 0.2)
            .salary(80_000)
            .loan_listed()
            .build();
        assert_eq!(Fx::board(&country, &[settled, lent]), vec![21]);
    }

    /// A walk ends at the first name too weak for the shirt, in whatever
    /// order the pool was filled.
    #[test]
    fn a_walk_stops_at_the_shirts_weakest_name() {
        let summaries: Vec<PlayerSummary> = [(1, 80), (2, 140), (3, 100), (4, 60)]
            .into_iter()
            .map(|(id, ability)| TestSummary::new(id).ability(ability).build())
            .collect();
        let pool = WatchPool::of(summaries.iter().map(|summary| {
            (
                PlayerFieldPositionGroup::Midfielder,
                WatchCandidate::of(summary, 0.0, WatchAvailability::None),
            )
        }));
        let reach = ReachMask::of(&[summaries[0].region]);
        let walked: Vec<u32> = pool
            .walk(PlayerFieldPositionGroup::Midfielder, reach, 100)
            .map(|candidate| candidate.player_id)
            .collect();
        assert_eq!(walked, vec![2, 3]);
    }

    /// A name's record is opened only when his best case would take a place,
    /// so no record may score him above it.
    #[test]
    fn no_name_scores_above_his_best_case() {
        let summary = TestSummary::new(1).build();
        for availability in [WatchAvailability::None, WatchAvailability::Requested] {
            let candidate = WatchCandidate::of(&summary, 37.5, availability);
            for gain in [-3.0f32, 0.0, 12.5, 80.0] {
                let best = MarketKnowledge::board_score(
                    gain,
                    &candidate,
                    1.0,
                    MarketKnowledge::BEST_LANGUAGE,
                );
                for role_fit in [0.6, 1.0] {
                    for language in [-0.5, -0.25, 0.15, 0.25, 0.5] {
                        let score =
                            MarketKnowledge::board_score(gain, &candidate, role_fit, language);
                        assert!(score <= best, "{score} above the best case {best}");
                    }
                }
            }
        }
    }

    /// The shirt's weakest name is the first ability the kindest read lifts
    /// over its bar.
    #[test]
    fn the_weakest_name_is_the_first_the_kindest_read_lifts_over_the_bar() {
        let slot = Fx::slot(1_000_000.0);
        let kindest = ClubOpinion::kindest(10);
        let clears = |truth: u8| {
            kindest.believed_ability(truth) - slot.incumbent_level as f32 >= slot.min_gain as f32
        };
        let weakest = MarketKnowledge::weakest_over_bar(&kindest, &slot).unwrap();
        assert!(clears(weakest));
        assert!(!clears(weakest - 1));
    }
}

#[cfg(test)]
mod top_board_tests {
    use super::*;
    use crate::transfers::gate::TransferMoveStage;
    use crate::transfers::tests::kit::{TestDate, TestSummary};

    fn reading() -> FileReading {
        FileReading {
            stage: TransferMoveStage::CanCompleteMove,
            stopped_by: None,
            read_on: TestDate::today(),
        }
    }

    fn board<'a>(walk: &[(&'a WatchCandidate<'a>, f32)]) -> Vec<u32> {
        let mut board = TopBoard::new(2);
        for (candidate, score) in walk {
            if board.admits(*score, candidate.player_id) {
                board.offer(candidate, *score, 0.0, reading());
            }
        }
        board
            .into_sorted()
            .into_iter()
            .map(|(c, _, _, _)| c.player_id)
            .collect()
    }

    /// Two names level on score at the edge of the board: the lower id
    /// keeps the place, whichever the pool reached first.
    #[test]
    fn the_board_holds_the_same_names_in_any_walk_order() {
        let summaries: Vec<PlayerSummary> = [5, 3, 9]
            .into_iter()
            .map(|id| TestSummary::new(id).build())
            .collect();
        let candidates: Vec<WatchCandidate<'_>> = summaries
            .iter()
            .map(|s| WatchCandidate::of(s, 0.0, WatchAvailability::None))
            .collect();
        let (five, three, nine) = (&candidates[0], &candidates[1], &candidates[2]);

        for walk in [
            vec![(five, 10.0), (three, 10.0), (nine, 20.0)],
            vec![(three, 10.0), (five, 10.0), (nine, 20.0)],
            vec![(nine, 20.0), (five, 10.0), (three, 10.0)],
        ] {
            assert_eq!(board(&walk), vec![9, 3]);
        }
    }
}
