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

use chrono::{Datelike, NaiveDate, Weekday};
use rayon::prelude::*;

use crate::club::player::personality::Language;
use crate::transfers::ScoutingRegion;
use crate::transfers::pipeline::breakout::{BreakoutInputs, BreakoutPerformanceSignal};
use crate::transfers::pipeline::loan_interest::ClubOpinion;
use crate::transfers::pipeline::planning::BriefTier;
use crate::transfers::pipeline::processor::{PipelineProcessor, PlayerSummary};
use crate::transfers::pipeline::standing::{StandingInputs, StandingSignal};
use crate::transfers::pipeline::trace::TransferTrace;
use crate::transfers::pipeline::{
    ClubTransferPlan, KnownPlayerMemory, ScoutMonitoringSource, ScoutPlayerMonitoring,
    WatchAvailability, WatchlistEntry,
};
use crate::{Club, Country, PlayerFieldPositionGroup};

/// One candidate, scored once per country so every club in it can be judged
/// against the same cheap arithmetic instead of recomputing the signals
/// forty times.
struct WatchCandidate<'a> {
    summary: &'a PlayerSummary,
    /// The stronger of the two discovery reads — form and standing measure
    /// the same question from opposite ends, so the honest signal is the max.
    discovery: f32,
    availability: WatchAvailability,
}

impl PipelineProcessor {
    /// Weekly, year-round refresh of every club's watchlist — the pipeline's
    /// entry point into [`MarketKnowledge`].
    ///
    /// Reads the whole world snapshot rather than the foreign view: a club's
    /// standing board includes the domestic players it would sign, and the
    /// snapshot already carries them with the same summary the cross-border
    /// side reads.
    pub fn refresh_watchlists(
        country: &mut Country,
        world_pool: &[PlayerSummary],
        date: NaiveDate,
    ) {
        MarketKnowledge::refresh(country, world_pool, date);
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

    fn covers(self, region: ScoutingRegion) -> bool {
        self.0 & Self::bit(region) != 0
    }
}

/// A bounded "best `capacity` so far" board.
///
/// The candidate pool for one shirt is the world, and only eight names ever
/// survive — so the pass keeps the eight rather than sorting sixty thousand.
/// Insertion is `O(capacity)` and the whole board fits in a cache line or
/// two, which matters because this runs per club per briefed shirt every
/// Monday.
struct TopBoard<'a> {
    capacity: usize,
    /// `(candidate, score, believed ability)`, worst-to-best is NOT
    /// maintained — only the running minimum is tracked, and the final sort
    /// is over at most `capacity` rows.
    rows: Vec<(&'a WatchCandidate<'a>, f32, f32)>,
    worst: f32,
}

impl<'a> TopBoard<'a> {
    fn new(capacity: usize) -> Self {
        TopBoard {
            capacity,
            rows: Vec::with_capacity(capacity),
            worst: f32::NEG_INFINITY,
        }
    }

    fn offer(&mut self, candidate: &'a WatchCandidate<'a>, score: f32, believed: f32) {
        if self.rows.len() < self.capacity {
            self.rows.push((candidate, score, believed));
            if self.rows.len() == self.capacity {
                self.worst = self
                    .rows
                    .iter()
                    .map(|(_, s, _)| *s)
                    .fold(f32::INFINITY, f32::min);
            }
            return;
        }
        if score <= self.worst {
            return;
        }
        // Replace the current weakest, then re-read the new minimum. Ties
        // break on the lower player id so a board is reproducible across
        // runs at the same seed.
        if let Some(slot) = self.rows.iter_mut().min_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.0.summary.player_id.cmp(&a.0.summary.player_id))
        }) {
            *slot = (candidate, score, believed);
        }
        self.worst = self
            .rows
            .iter()
            .map(|(_, s, _)| *s)
            .fold(f32::INFINITY, f32::min);
    }

    fn into_sorted(mut self) -> Vec<(&'a WatchCandidate<'a>, f32, f32)> {
        self.rows.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.summary.player_id.cmp(&b.0.summary.player_id))
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
    pub fn refresh(country: &mut Country, world_pool: &[PlayerSummary], date: NaiveDate) {
        if date.weekday() != Weekday::Mon {
            return;
        }

        // ── One country-wide candidate pass ─────────────────────────
        //
        // The two discovery signals and the availability read are pure
        // arithmetic over the summary, so they are computed once here and
        // shared by every club. Only the belief noise and the per-club gates
        // run inside the fan-out below.
        let country_reputation = country.reputation;
        let mut by_group: [Vec<WatchCandidate<'_>>; PlayerFieldPositionGroup::COUNT] =
            Default::default();
        for summary in world_pool.iter() {
            // Clubs recruit at or below their own country's standing; an
            // openly-available player is the exception that proves it, since
            // availability is precisely the signal that overrides the normal
            // direction of travel.
            if summary.country_reputation > country_reputation
                && !Self::is_openly_available(summary)
            {
                continue;
            }
            // Free agents are the free-agent market's business; a watchlist
            // is about players somebody else owns.
            if summary.contract_months_remaining <= 0 {
                continue;
            }
            let discovery = Self::discovery_score(summary);
            let availability = Self::availability_of(summary);
            let candidate = WatchCandidate {
                summary,
                discovery,
                availability,
            };
            for group in PlayerFieldPositionGroup::ALL {
                if summary.coverage.covers_group(group) {
                    by_group[group.index()].push(WatchCandidate {
                        summary: candidate.summary,
                        discovery: candidate.discovery,
                        availability: candidate.availability,
                    });
                }
            }
        }

        // ── Per-club fan-out (read only) ────────────────────────────
        let country_ref: &Country = country;
        let staged: Vec<(u32, Vec<WatchlistEntry>)> = country_ref
            .clubs
            .par_iter()
            .map(|club| (club.id, Self::for_club(country_ref, club, &by_group, date)))
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
                Self::merge(&mut club.transfer_plan, entries, date);
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
        let monitor_cap = PipelineProcessor::breakout_watch_monitor_cap(overall_score);
        let intake = PipelineProcessor::breakout_watch_per_pass(overall_score);
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

            let id = plan.next_monitoring_id();
            let mut row = ScoutPlayerMonitoring::new(
                id,
                watcher_id,
                entry.player_id,
                ScoutMonitoringSource::StaffRecommendation,
                date,
            );
            // Confidence starts low and grows with observation, exactly as a
            // demand-driven assignment's does — a name off a data board is a
            // lead, not a report.
            row.record_observation(
                entry.believed_ability,
                entry.believed_ability,
                Self::INITIAL_CONFIDENCE,
                1.0,
                entry.estimated_value,
                Vec::new(),
                date,
                false,
            );
            plan.scout_monitoring.push(row);

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
    /// Monday.
    fn for_club(
        country: &Country,
        club: &Club,
        by_group: &[Vec<WatchCandidate<'_>>; PlayerFieldPositionGroup::COUNT],
        date: NaiveDate,
    ) -> Vec<WatchlistEntry> {
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
        let reach = ReachMask::of(&PipelineProcessor::reputation_scout_regions(
            home_region,
            overall_score,
        ));
        let home_language_mask = Language::country_language_mask(&country.code);

        let mut entries: Vec<WatchlistEntry> = Vec::new();
        for slot in brief.slots.iter() {
            let pool = &by_group[slot.group.index()];
            let envelope_ceiling = slot.envelope * Self::ENVELOPE_SLACK;
            // A bounded top-`WATCH_PER_ROLE` board rather than a sorted
            // list of everyone who survived the gates. The pool is the
            // world; sorting it per slot per club every Monday is the
            // difference between a weekly pass and a weekly stall, and the
            // board only ever keeps eight names anyway.
            let mut board = TopBoard::new(Self::WATCH_PER_ROLE);
            for candidate in pool.iter() {
                let s = candidate.summary;
                if s.club_id == club.id || club.is_rival(s.club_id) {
                    continue;
                }
                if s.age < slot.age_band.0 || s.age > slot.age_band.1 {
                    continue;
                }
                // The envelope is a real constraint on a WATCHLIST, not just
                // on a bid: a department does not keep files on players its
                // owner will never fund. A tier-C slot therefore watches the
                // free and loan end of the market, which is exactly right.
                if envelope_ceiling > 0.0 && s.estimated_value > envelope_ceiling {
                    continue;
                }
                if !reach.covers(s.region) {
                    continue;
                }

                // ── Belief ──
                // The club's own read of him, then everything a scout can
                // see that is not his ability: how far he stands above his
                // own league, what his club thinks of him, whether anyone
                // could actually get him, and whether he could be spoken to
                // in the dressing room.
                let opinion = ClubOpinion::of(club.id, s.player_id, judging);
                let believed = opinion.believed_ability(s.skill_ability);
                let gain = believed - slot.incumbent_level as f32;
                if gain < slot.min_gain as f32 {
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
                let score = gain * Self::GAIN_WEIGHT
                    + candidate.discovery * Self::DISCOVERY_WEIGHT
                    + role_fit * Self::ROLE_FIT_WEIGHT
                    + candidate.availability.lift() * Self::AVAILABILITY_WEIGHT
                    + language * Self::LANGUAGE_WEIGHT;
                board.offer(candidate, score, believed);
            }

            for (candidate, score, believed) in board.into_sorted() {
                let s = candidate.summary;
                if entries.iter().any(|e| e.player_id == s.player_id) {
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
                entries.push(WatchlistEntry {
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
                });
            }
        }
        entries
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

    /// Merge this week's board into the club's plan: refresh what is still
    /// there, keep what is merely unseen this week, drop what has gone
    /// stale.
    fn merge(plan: &mut ClubTransferPlan, fresh: Vec<WatchlistEntry>, date: NaiveDate) {
        for entry in fresh {
            match plan
                .watchlist
                .iter_mut()
                .find(|e| e.player_id == entry.player_id)
            {
                Some(existing) => *existing = entry,
                None => plan.watchlist.push(entry),
            }
        }
        plan.watchlist
            .retain(|e| (date - e.last_refreshed).num_days() <= Self::ENTRY_FRESH_DAYS);
        if plan.watchlist.len() > Self::MAX_ENTRIES {
            plan.watchlist.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.player_id.cmp(&b.player_id))
            });
            plan.watchlist.truncate(Self::MAX_ENTRIES);
        }
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

    fn is_openly_available(s: &PlayerSummary) -> bool {
        s.is_listed
            || s.is_loan_listed
            || s.seller_ctx.is_transfer_requested
            || (s.contract_months_remaining > 0 && s.contract_months_remaining <= 12)
    }
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

    /// True for the transformative end of the board — the names a club
    /// would actually spend a window on.
    pub fn is_headline(&self) -> bool {
        matches!(self.slot_tier, BriefTier::A)
    }
}

#[cfg(test)]
mod watchlist_tests {
    use super::*;
    use crate::PlayerPositionType;
    use crate::transfers::pipeline::asset_ledger::{SellListEntry, SellMotive};

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
        MarketKnowledge::merge(&mut plan, vec![seen_again], Fx::date());

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
        let fresh: Vec<WatchlistEntry> = (1..=200)
            .map(|i| Fx::entry(i, PlayerFieldPositionGroup::Midfielder, i as f32))
            .collect();
        MarketKnowledge::merge(&mut plan, fresh, Fx::date());
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
                score: 0.8,
                motive: SellMotive::PeakValue,
                marked_on: Fx::date(),
            },
            // Below the bar: priced, but the club is not ready to answer.
            SellListEntry {
                player_id: 8,
                asking: 5_000_000.0,
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
