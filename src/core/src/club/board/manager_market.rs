//! Manager market — shortlists, candidate scoring, and (in slice C)
//! poaching.
//!
//! When a club's seat opens up (board sacks the incumbent or contract
//! lapses), `ClubBoard.manager_search_since` is set to today. From that
//! tick onward the world-level manager-market phase
//! (`simulator.rs` Phase D) refreshes the club's `manager_shortlist`
//! once a week, ranking the candidates the board is willing to
//! consider.
//!
//! The module is partitioned into focused namespaces — every helper
//! lives as a method on the type that owns its concern:
//!   * `ManagerMarketTick`       — daily orchestrator + appointment phase
//!   * `ManagerSearch`           — `ClubBoard` search-state operations
//!   * `ManagerSeat`             — main-team head-coach-seat operations
//!   * `ManagerShortlist`        — shortlist building (free agents +
//!                                 employed targets) and shared limits
//!   * `ManagerCandidateScorer`  — pure scoring / acceptance predicates
//!   * inherent impl on `ManagerApproach` — poaching state machine

use crate::club::Club;
use crate::club::Team;
use crate::club::board::ClubBoard;
use crate::club::staff::contract::{StaffClubContract, StaffPosition, StaffStatus};
use crate::club::staff::free_pool;
use crate::utils::DateUtils;
use crate::{SimulatorData, Staff, TeamType};
use chrono::{Datelike, NaiveDate};
use log::{debug, info};
use rayon::prelude::*;

// ─── Shared types ──────────────────────────────────────────────────────

/// Where this candidate came from. Slice C adds `Employed` and the
/// approach pipeline that operates on it; for now only `FreeAgent`
/// is reachable, but the variant exists so callers can pattern-match
/// without breaking when slice C lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateSource {
    FreeAgent,
    Employed { current_club_id: u32 },
}

/// A ranked entry on a club's manager shortlist. `fit_score` is the
/// composite ranking value; `target_salary` is what the candidate
/// would expect to be offered (the board's actual offer may flex).
#[derive(Debug, Clone)]
pub struct ManagerCandidate {
    pub staff_id: u32,
    pub fit_score: i32,
    pub target_salary: u32,
    pub source: CandidateSource,
}

/// State machine for an in-flight approach. One day per state advance:
/// `Made` (day 0) → either `CompensationDemanded` or `Rejected` (day 1)
/// → `CompensationAgreed` or `Rejected` (day 2) → `TermsAccepted` or
/// `Rejected` (day 3) → finalized (day 4, removes the approach).
///
/// Approaches are stored on `SimulatorData.pending_manager_approaches`
/// — a global registry so cascade hires (poached source club starting
/// its own search) can see the chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApproachState {
    /// The requesting club has notified the source club. Awaiting
    /// permission-to-talk + compensation demand.
    Made,
    /// Source club agreed to release the manager for `amount` in
    /// compensation. Requesting club must now accept or walk away.
    CompensationDemanded { amount: u32 },
    /// Compensation agreed and (notionally) paid; the approach now
    /// proceeds to personal-terms negotiation with the candidate.
    CompensationAgreed,
    /// Candidate has accepted the offered terms. Next tick will
    /// finalize the move (and trigger source-club cascade).
    TermsAccepted,
    /// Approach is dead — recorded so the requesting club doesn't
    /// re-approach the same target while the entry is being cleaned
    /// up. Removed from the registry one tick after being set.
    Rejected,
}

/// One in-flight pursuit of an employed manager. Stored on
/// `SimulatorData.pending_manager_approaches` and ticked daily by the
/// world-level manager-market phase.
#[derive(Debug, Clone)]
pub struct ManagerApproach {
    pub requesting_club_id: u32,
    pub source_club_id: u32,
    pub staff_id: u32,
    pub state: ApproachState,
    /// Salary the requesting club is willing to offer the candidate.
    /// Set at approach creation; never re-negotiated within a single
    /// approach (a rejection forces the requesting club to start over
    /// with a fresh approach, possibly at a higher offer).
    pub offered_salary: u32,
    pub created_at: NaiveDate,
    /// Day the approach last transitioned. Used to enforce a one-day
    /// cooldown between state advances so the pipeline takes the
    /// realistic ~5 days from approach to signing.
    pub last_action: NaiveDate,
}

// ─── ManagerSeat — head-coach-seat operations on a main team ───────────

pub struct ManagerSeat;

impl ManagerSeat {
    /// True when the club's main-team head-coach seat is open to a
    /// permanent appointment: either no head coach at all, or only a
    /// `CaretakerManager` (interim) is sitting in the seat. A permanent
    /// `Manager` blocks any new hire — same-club replacement requires
    /// the incumbent to be sacked first (which removes the `Manager`
    /// and promotes a caretaker).
    pub fn club_has_vacancy(club: &Club) -> bool {
        let Some(main) = club.teams.main() else {
            return false;
        };
        main.staffs
            .find_by_position(StaffPosition::Manager)
            .is_none()
    }

    /// Demote any sitting `CaretakerManager` on the main team back to
    /// a generic `Coach`. Used by both the free-agent appointment path
    /// and the poach-finalize path before a permanent manager is
    /// installed, so we never end a tick with both `Manager` and
    /// `CaretakerManager` holding the head-coach seat simultaneously.
    pub fn clear_caretaker(team: &mut Team) {
        if let Some(caretaker) = team
            .staffs
            .find_mut_by_position(StaffPosition::CaretakerManager)
        {
            if let Some(c) = caretaker.contract.as_mut() {
                c.position = StaffPosition::Coach;
            }
        }
    }

    /// Promote the strongest remaining coach on `team` to
    /// `CaretakerManager` for a 60-day stint. Salary is
    /// `max(current_coach_salary, prior_salary / 2)` — the same formula
    /// the sacking path uses, so a coach who steps up after a poach is
    /// paid like one who steps up after a sacking. Returns `true` if a
    /// caretaker was installed; `false` when no coaching staff are on
    /// the books (small clubs, dev fixtures).
    pub fn promote_best_caretaker(team: &mut Team, prior_salary: u32, today: NaiveDate) -> bool {
        let caretaker_id = team.staffs.best_coach_id(|s| {
            s.staff_attributes.coaching.tactical as u32
                + s.staff_attributes.mental.man_management as u32
                + s.staff_attributes.mental.motivating as u32
                + s.staff_attributes.coaching.mental as u32
        });
        let Some(id) = caretaker_id else {
            return false;
        };
        let Some(staff) = team.staffs.find_mut(id) else {
            return false;
        };
        let current_salary = staff.contract.as_ref().map(|c| c.salary).unwrap_or(0);
        let salary = current_salary.max(prior_salary / 2);
        let expires = today
            .checked_add_signed(chrono::Duration::days(60))
            .unwrap_or_else(|| {
                NaiveDate::from_ymd_opt(today.year() + 1, today.month(), 1).unwrap()
            });
        staff.contract = Some(StaffClubContract::new(
            salary,
            expires,
            StaffPosition::CaretakerManager,
            StaffStatus::Active,
        ));
        info!("Promoted staff {} to caretaker manager", id);
        true
    }

    /// Build a Manager contract for a freshly-signed candidate. Three-year
    /// term, salary as agreed. Status is `Active`.
    pub fn build_manager_contract(salary: u32, today: NaiveDate) -> StaffClubContract {
        let expires = today.with_year(today.year() + 3).unwrap_or(today);
        StaffClubContract::new(salary, expires, StaffPosition::Manager, StaffStatus::Active)
    }
}

// ─── ManagerSearch — ClubBoard search-state operations ─────────────────

pub struct ManagerSearch;

impl ManagerSearch {
    /// Initialise search state on a fresh sacking. Records the start
    /// day and locks in the search-window length based on club rep —
    /// kept on the board so we don't recompute from scratch every tick.
    pub fn open(board: &mut ClubBoard, today: NaiveDate, club_rep: u16) {
        board.manager_search_since = Some(today);
        board.search_window_days = ManagerCandidateScorer::search_window_days(club_rep);
        board.manager_shortlist.clear();
        board.shortlist_built_at = None;
    }

    /// Wipe shortlist state once a hire is finalised. Called from
    /// `result.rs` on confirm_new_manager whether the hire succeeded
    /// or fell back to the caretaker.
    pub fn clear(board: &mut ClubBoard) {
        board.manager_shortlist.clear();
        board.shortlist_built_at = None;
        board.manager_search_since = None;
        board.search_window_days = 0;
    }

    /// Pull the top free-agent candidate off a board's shortlist and
    /// remove the matching `Staff` from the global pool. Returns the
    /// owned staff member ready to be assigned to the team's roster,
    /// or `None` if the shortlist is empty / the candidate has already
    /// been signed by someone else (race during a daily tick).
    pub fn take_top_free_agent(
        board: &mut ClubBoard,
        pool: &mut Vec<Staff>,
    ) -> Option<(Staff, u32)> {
        while let Some(candidate) = board.manager_shortlist.first().cloned() {
            // Drop this entry up-front — even if we fail to find the
            // staff (signed elsewhere already), the entry was stale
            // either way.
            board.manager_shortlist.remove(0);
            if !matches!(candidate.source, CandidateSource::FreeAgent) {
                // Slice B only handles free-agent path; employed
                // candidates are handled via the approach pipeline in
                // slice C.
                continue;
            }
            let idx = pool.iter().position(|s| s.id == candidate.staff_id)?;
            let staff = pool.remove(idx);
            return Some((staff, candidate.target_salary));
        }
        None
    }
}

// ─── ManagerCandidateScorer — pure scoring / predicates ────────────────

pub struct ManagerCandidateScorer;

impl ManagerCandidateScorer {
    /// Days the search may run before the board confirms a hire (or
    /// falls back to the caretaker). Top clubs hunt longer because
    /// they're chasing big names; smaller clubs move faster because
    /// their pool of realistic targets is shallower and the season
    /// won't wait.
    pub fn search_window_days(world_rep: u16) -> u16 {
        if world_rep >= 8000 {
            60
        } else if world_rep >= 5000 {
            45
        } else if world_rep >= 2500 {
            30
        } else {
            21
        }
    }

    /// Score a free-agent candidate against a club's profile. Higher
    /// is a better fit. Returns `None` if the candidate is fundamentally
    /// inappropriate (wrong age band, no contract history, etc.).
    ///
    /// Composite of:
    ///   - Coaching skill (caretaker-style score: tactical,
    ///     man_management, motivating, mental).
    ///   - Reputation tier match: penalty when the candidate is wildly
    ///     out of the club's league (Pep at a relegation side; or a
    ///     journeyman at a CL contender).
    ///   - Age fit: 38-58 is the sweet spot for most clubs; reckless
    ///     boards tolerate younger and older outliers.
    ///   - Personal traits via `attributes.ambition`/`loyalty` —
    ///     high-ambition coaches favour upwardly-mobile clubs.
    ///
    /// Score is rough — what matters is the *ordering*, not absolute
    /// values. Tweaks to weights here only change which candidate
    /// floats to #1.
    pub fn score_free_agent(staff: &Staff, club_rep: u16, today: NaiveDate) -> Option<i32> {
        // Skill base — same components the caretaker scorer uses,
        // scaled up so candidate ranking dominates over the secondary
        // factors.
        let skill = staff.staff_attributes.coaching.tactical as i32
            + staff.staff_attributes.mental.man_management as i32
            + staff.staff_attributes.mental.motivating as i32
            + staff.staff_attributes.coaching.mental as i32
            + staff.staff_attributes.knowledge.tactical_knowledge as i32;
        let mut score = skill * 4; // 0..400

        // Reputation tier match — we infer the candidate's tier from
        // their composite skill since `Staff` doesn't carry an explicit
        // reputation field. Wide miss in either direction drops score.
        let candidate_tier = (skill * 100).min(10000) as u16;
        let gap = (candidate_tier as i32 - club_rep as i32).abs();
        score -= gap / 50; // a 1000-pt mismatch costs 20 points

        // Age fit — heavy fade outside 35-60 band, soft fade beyond 55.
        let age = DateUtils::age(staff.birth_date, today) as i32;
        let age_drag = if age < 32 {
            (32 - age) * 6
        } else if age > 60 {
            (age - 60) * 8
        } else if age > 55 {
            (age - 55) * 2
        } else {
            0
        };
        score -= age_drag;

        // Personal trait bonus — ambition pulls a candidate toward
        // high-rep clubs, loyalty rewards continuity.
        let ambition_bias = (staff.attributes.ambition * (club_rep as f32 / 1000.0)) as i32;
        score += ambition_bias;

        // Hard floor: no negative-skill candidates ever.
        if skill < 20 {
            return None;
        }

        Some(score)
    }

    /// Score an employed candidate. Currently uses the same
    /// skill-based scoring as free agents but with a small "approach
    /// friction" penalty so the board prefers a free agent of
    /// equivalent quality (cheaper, no compensation, no friction).
    /// Slice D could refine with style fit.
    pub fn score_employed(staff: &Staff, requesting_rep: u16, today: NaiveDate) -> Option<i32> {
        let base = Self::score_free_agent(staff, requesting_rep, today)?;
        // Friction tax: -25 for the trouble. A clearly-better candidate
        // still wins; a marginal upgrade no longer beats the in-house
        // promotion.
        Some(base - 25)
    }

    /// Salary the candidate expects for a job at a club of the given
    /// rep. Composite of: club tier base salary, candidate skill
    /// markup, age experience markup. Used as the offer the board
    /// makes; board may flex this in slice C's negotiation.
    pub fn target_salary(staff: &Staff, club_rep: u16, today: NaiveDate) -> u32 {
        let rep_tier = club_rep as u32;
        let base = 30_000 + rep_tier * 50; // 30k..530k by rep alone

        let skill = (staff.staff_attributes.coaching.tactical as u32
            + staff.staff_attributes.mental.man_management as u32
            + staff.staff_attributes.mental.motivating as u32
            + staff.staff_attributes.coaching.mental as u32) as f32
            / 80.0;
        let skill_mult = 0.6 + skill; // 0.6..1.6

        let age = DateUtils::age(staff.birth_date, today) as u32;
        let exp_mult = if age >= 50 {
            1.20
        } else if age >= 40 {
            1.10
        } else {
            1.0
        };

        ((base as f32) * skill_mult * exp_mult) as u32
    }

    /// Multiplier on (annual salary × remaining contract years) the
    /// source club demands as compensation. Higher tiers gouge more.
    pub fn compensation_multiplier(source_world_rep: u16) -> f32 {
        if source_world_rep >= 7000 {
            1.5
        } else if source_world_rep >= 4000 {
            1.2
        } else {
            1.0
        }
    }

    /// Whether the source club refuses to even talk. Reads source-club
    /// confidence and form: clubs whose manager is over-delivering
    /// protect their guy harder.
    pub fn source_refuses_outright(source_confidence: i32, source_overperforming: bool) -> bool {
        // Strong confidence + overperforming = ironclad refusal.
        // Otherwise they'll engage and try to extract compensation.
        source_confidence >= 80 && source_overperforming
    }

    /// Personal-terms acceptance check. The candidate accepts if
    /// EITHER the offered salary is materially above their current pay
    /// OR the requesting club is materially more prestigious.
    pub fn candidate_accepts_terms(data: &SimulatorData, approach: &ManagerApproach) -> bool {
        let Some(src) = data.club(approach.source_club_id) else {
            return false;
        };
        let Some(main) = src
            .teams
            .iter()
            .find(|t| matches!(t.team_type, TeamType::Main))
        else {
            return false;
        };
        let Some(mgr) = main.staffs.find(approach.staff_id) else {
            return false;
        };
        let current_salary = mgr.contract.as_ref().map(|c| c.salary).unwrap_or(0);
        let current_rep = main.reputation.world;
        let ambition = mgr.attributes.ambition; // 0..20

        let req_rep = data
            .club(approach.requesting_club_id)
            .and_then(|c| {
                c.teams
                    .iter()
                    .find(|t| matches!(t.team_type, TeamType::Main))
            })
            .map(|t| t.reputation.world)
            .unwrap_or(0);

        let salary_uplift = (approach.offered_salary as f32) >= (current_salary as f32) * 1.20;
        let prestige_uplift = (req_rep as f32) >= (current_rep as f32) * 1.30;

        // Ambitious coaches accept smaller prestige gaps; loyal
        // coaches demand bigger ones.
        let ambition_bonus = ambition >= 14.0;

        salary_uplift || prestige_uplift || (ambition_bonus && req_rep > current_rep)
    }
}

// ─── ManagerShortlist — building free / combined shortlists ────────────

pub struct ManagerShortlist;

impl ManagerShortlist {
    /// Maximum candidates kept on a club's shortlist. Five is enough
    /// to model "first-choice falls through, board moves to backup"
    /// without blowing memory on every club every day.
    pub const MAX_LEN: usize = 5;

    /// How often the shortlist is rebuilt while a search is open. Pool
    /// turnover (new free agents from rival sackings) is slow enough
    /// that daily refreshes are wasted work; weekly is plenty.
    pub const REFRESH_DAYS: i64 = 7;

    /// Top N free-agent candidates for a given club. Reads the global
    /// pool, scores each entry against the club's reputation, and
    /// returns the best-fit ranking. O(N log N) over the pool — pool
    /// size is in the low thousands at most, so this is cheap.
    pub fn from_free_agents(
        pool: &[Staff],
        club_rep: u16,
        today: NaiveDate,
    ) -> Vec<ManagerCandidate> {
        let mut scored: Vec<ManagerCandidate> = pool
            .iter()
            .filter_map(|s| {
                let fit = ManagerCandidateScorer::score_free_agent(s, club_rep, today)?;
                let target_salary = ManagerCandidateScorer::target_salary(s, club_rep, today);
                Some(ManagerCandidate {
                    staff_id: s.id,
                    fit_score: fit,
                    target_salary,
                    source: CandidateSource::FreeAgent,
                })
            })
            .collect();

        scored.sort_unstable_by(|a, b| b.fit_score.cmp(&a.fit_score));
        scored.truncate(Self::MAX_LEN);
        scored
    }

    /// Enumerate currently-employed managers across the world that a
    /// requesting club might shortlist. Returns one `ManagerCandidate`
    /// per hireable head coach at a club whose reputation is at most
    /// `rep_ceiling` (i.e. clubs strictly smaller than the requesting
    /// club, by a margin) AND whose manager is performing above the
    /// board's expectations.
    ///
    /// Cross-border by default — a Premier League club can poach from
    /// La Liga or the Eredivisie. Slice D can add language/work-permit
    /// modifiers; for now any continent is fair game.
    pub fn enumerate_employed(
        data: &SimulatorData,
        requesting_rep: u16,
        today: NaiveDate,
    ) -> Vec<ManagerCandidate> {
        let rep_ceiling = ((requesting_rep as f32) * 0.8) as u16;
        let mut out: Vec<ManagerCandidate> = Vec::new();

        for continent in &data.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    let main_team = match club
                        .teams
                        .iter()
                        .find(|t| matches!(t.team_type, TeamType::Main))
                    {
                        Some(t) => t,
                        None => continue,
                    };
                    if main_team.reputation.world > rep_ceiling {
                        continue;
                    }
                    // Performance filter: only consider managers at
                    // clubs outperforming their season target. Avoid
                    // poaching someone who's already failing.
                    let Some(targets) = &club.board.season_targets else {
                        continue;
                    };
                    // Overperforming proxy: confidence >= 70 means the
                    // board is happy, i.e. the manager is over-delivering.
                    // Cheaper than re-deriving league standings here.
                    if club.board.confidence.level < 70 {
                        continue;
                    }
                    let _ = targets; // future: finer filtering

                    let Some(manager) = main_team.staffs.find_by_position(StaffPosition::Manager)
                    else {
                        continue;
                    };
                    let Some(score) =
                        ManagerCandidateScorer::score_employed(manager, requesting_rep, today)
                    else {
                        continue;
                    };
                    let target_salary =
                        ManagerCandidateScorer::target_salary(manager, requesting_rep, today);
                    out.push(ManagerCandidate {
                        staff_id: manager.id,
                        fit_score: score,
                        target_salary,
                        source: CandidateSource::Employed {
                            current_club_id: club.id,
                        },
                    });
                }
            }
        }
        out
    }

    /// Build the full shortlist for a club — free agents + viable
    /// employed targets, merged and ranked. Replaces the
    /// free-agent-only shortlist for clubs that have committed to
    /// slice C.
    pub fn combined(
        data: &SimulatorData,
        requesting_club_id: u32,
        requesting_rep: u16,
        today: NaiveDate,
    ) -> Vec<ManagerCandidate> {
        let mut combined: Vec<ManagerCandidate> =
            Self::from_free_agents(&data.free_agent_staff, requesting_rep, today);

        let employed = Self::enumerate_employed(data, requesting_rep, today);
        for c in employed {
            // Don't shortlist the club's own current manager (paranoia
            // — the rep-ceiling filter should already exclude self,
            // but a club at the rep boundary could match itself).
            if let CandidateSource::Employed { current_club_id } = c.source {
                if current_club_id == requesting_club_id {
                    continue;
                }
            }
            combined.push(c);
        }

        combined.sort_unstable_by(|a, b| b.fit_score.cmp(&a.fit_score));
        combined.truncate(Self::MAX_LEN);
        combined
    }
}

// ─── ManagerMarketTick — daily orchestrator + appointment phase ────────

/// Outcome of a single club's pass through `refresh_shortlists` —
/// either the club's stale search must be cleared, or it needs a fresh
/// candidate shortlist. Carrying both decisions in one parallel sweep
/// keeps the world walk to a single pass.
enum ShortlistDecision {
    Refresh(u32, u16),
    Clear(u32),
}

pub struct ManagerMarketTick;

impl ManagerMarketTick {
    /// Run one daily tick of the world-level manager market in the
    /// canonical order: harvest expired contracts → age the pool →
    /// refresh shortlists → initiate fresh approaches → advance
    /// in-flight approaches.
    ///
    /// The order is load-bearing:
    ///   1. Sacked / contract-lapsed staff must hit the free-agent
    ///      pool *before* shortlists refresh, otherwise the freshly-
    ///      vacated seats look like they have no candidates.
    ///   2. Pool aging (satisfaction decay, retirement) runs before
    ///      shortlists so retiring coaches don't appear as candidates
    ///      this tick.
    ///   3. Shortlists must exist before fresh approaches initiate
    ///      (an approach picks from the shortlist).
    ///   4. Approach ticks must run *after* fresh initiation so a
    ///      brand-new approach starts at state 0 and doesn't get
    ///      advanced on the same tick.
    ///
    /// Wrapping these in one function localises the contract — adding
    /// a sixth step now means editing one place rather than three call
    /// sites scattered around the orchestrator.
    pub fn run(data: &mut SimulatorData, today: NaiveDate) {
        free_pool::harvest_expired_staff(data, today);
        free_pool::tick_free_agent_staff_pool(&mut data.free_agent_staff, today);
        Self::refresh_shortlists(data);
        Self::initiate_approaches(data);
        Self::tick_approaches(data);
    }

    /// World-level pass: for every club in active manager search,
    /// refresh its `manager_shortlist` if stale. Stale searches whose
    /// permanent manager is still in post are wiped instead — keeps
    /// the manager market from generating phantom approaches against
    /// clubs that aren't really hiring.
    pub fn refresh_shortlists(data: &mut SimulatorData) {
        let today = data.date.date();

        // Snapshot pool by reference — we only need to read it. We
        // collect (club_id, club_rep) first to avoid holding an iter+mut
        // borrow across the rebuild closure. The world walk runs across
        // rayon so single-continent saves still light up every core.
        let decisions: Vec<ShortlistDecision> = data
            .continents
            .par_iter()
            .flat_map(|c| c.countries.par_iter())
            .flat_map(|country| country.clubs.par_iter())
            .filter_map(|club| {
                club.board.manager_search_since?;
                if !ManagerSeat::club_has_vacancy(club) {
                    debug!(
                        "Manager market: clearing stale search at club {} (permanent manager in post)",
                        club.id
                    );
                    return Some(ShortlistDecision::Clear(club.id));
                }
                let stale = club
                    .board
                    .shortlist_built_at
                    .map(|d| (today - d).num_days() >= ManagerShortlist::REFRESH_DAYS)
                    .unwrap_or(true);
                if !stale {
                    return None;
                }
                let club_rep = club
                    .teams
                    .iter()
                    .find(|t| matches!(t.team_type, TeamType::Main))
                    .map(|t| t.reputation.world)
                    .unwrap_or(0);
                Some(ShortlistDecision::Refresh(club.id, club_rep))
            })
            .collect();

        let mut to_refresh: Vec<(u32, u16)> = Vec::new();
        let mut stale_to_clear: Vec<u32> = Vec::new();
        for d in decisions {
            match d {
                ShortlistDecision::Refresh(id, rep) => to_refresh.push((id, rep)),
                ShortlistDecision::Clear(id) => stale_to_clear.push(id),
            }
        }

        for club_id in stale_to_clear {
            if let Some(club) = data.club_mut(club_id) {
                ManagerSearch::clear(&mut club.board);
            }
        }

        if to_refresh.is_empty() {
            return;
        }

        // Build candidate lists outside the mutable-club borrow, then
        // write back. Reads from the pool AND from every other club's
        // main team (for employed-target enumeration in slice C) — so
        // this is a read-only sweep over `data` before the write phase.
        // Each per-club shortlist build is independent → parallel.
        let updates: Vec<(u32, Vec<ManagerCandidate>)> = to_refresh
            .par_iter()
            .map(|&(club_id, club_rep)| {
                let shortlist = ManagerShortlist::combined(data, club_id, club_rep, today);
                (club_id, shortlist)
            })
            .collect();

        for (club_id, shortlist) in updates {
            if let Some(club) = data.club_mut(club_id) {
                debug!(
                    "Manager market: refreshed shortlist for club id {} ({} candidates)",
                    club_id,
                    shortlist.len()
                );
                club.board.manager_shortlist = shortlist;
                club.board.shortlist_built_at = Some(today);
            }
        }
    }

    /// Look at every club in active manager search and create a fresh
    /// `ManagerApproach` for the top employed candidate on their
    /// shortlist that doesn't already have one in flight. One approach
    /// per requesting club per tick — keeps the pace realistic and
    /// avoids flood-spam of identical approaches.
    pub fn initiate_approaches(data: &mut SimulatorData) {
        let today = data.date.date();

        let new_approaches: Vec<ManagerApproach> = data
            .continents
            .par_iter()
            .flat_map(|c| c.countries.par_iter())
            .flat_map(|country| country.clubs.par_iter())
            .filter_map(|club| {
                if club.board.manager_search_since.is_none() {
                    return None;
                }
                // Vacancy invariant: never start an approach for a
                // club whose permanent manager is still in post
                // (stale search state). `refresh_shortlists` clears
                // such state on its next pass; until then, just skip.
                if !ManagerSeat::club_has_vacancy(club) {
                    return None;
                }
                // Pick the top-ranked Employed candidate that isn't
                // already in an in-flight approach for this club.
                let already_pursuing: Vec<u32> = data
                    .pending_manager_approaches
                    .iter()
                    .filter(|a| a.requesting_club_id == club.id)
                    .map(|a| a.staff_id)
                    .collect();
                let pick = club.board.manager_shortlist.iter().find(|c| {
                    matches!(c.source, CandidateSource::Employed { .. })
                        && !already_pursuing.contains(&c.staff_id)
                })?;
                let CandidateSource::Employed { current_club_id } = pick.source else {
                    return None;
                };
                Some(ManagerApproach {
                    requesting_club_id: club.id,
                    source_club_id: current_club_id,
                    staff_id: pick.staff_id,
                    state: ApproachState::Made,
                    offered_salary: pick.target_salary,
                    created_at: today,
                    last_action: today,
                })
            })
            .collect();

        for a in new_approaches {
            debug!(
                "Manager market: approach made — club {} → manager {} (at club {})",
                a.requesting_club_id, a.staff_id, a.source_club_id
            );
            data.pending_manager_approaches.push(a);
        }
    }

    /// Advance every pending approach one tick. State transitions are
    /// gated to one-per-day (via `last_action`) so an approach takes
    /// the full ~5 days from inception to signing.
    ///
    /// Successful approaches finalize here: the staff member is moved
    /// from the source club's roster to the requesting club's, the
    /// requesting club's search state is cleared, and the source club
    /// opens its OWN search (the "cascade").
    pub fn tick_approaches(data: &mut SimulatorData) {
        let today = data.date.date();
        if data.pending_manager_approaches.is_empty() {
            return;
        }

        // Collect all approach indices to process this tick.
        let indices: Vec<usize> = data
            .pending_manager_approaches
            .iter()
            .enumerate()
            .filter(|(_, a)| (today - a.last_action).num_days() >= 1)
            .map(|(i, _)| i)
            .collect();

        for i in indices {
            let approach = data.pending_manager_approaches[i].clone();
            let next: Option<ApproachState> = approach.advance(data, today);
            if let Some(next_state) = next {
                data.pending_manager_approaches[i].state = next_state;
                data.pending_manager_approaches[i].last_action = today;
            }
        }

        // Reap rejected approaches (one-tick cleanup so the requesting
        // club won't immediately re-pursue the same target on the next
        // shortlist refresh).
        data.pending_manager_approaches
            .retain(|a| !matches!(a.state, ApproachState::Rejected));
    }

    /// Execute the permanent appointment for a club whose search
    /// window has elapsed. Owns the multi-step borrow choreography:
    /// peek the shortlist (needs club mut), withdraw the candidate
    /// from the global pool (needs data mut, club borrow dropped),
    /// then install them on the team (needs club mut again).
    ///
    /// Falls back to permanently promoting the sitting caretaker when
    /// the shortlist has no viable free-agent candidate — the common
    /// path for small clubs whose pool offerings are slim.
    pub fn execute_appointment(data: &mut SimulatorData, club_id: u32, today: NaiveDate) {
        if club_id == 0 {
            return;
        }

        // Vacancy invariant: never install a second permanent manager
        // on top of an existing one. If `manager_search_since` is stale
        // (a poach already filled the seat, or the search state was
        // never cleared after a renewal), wipe the search and bail
        // out — no sacking, no candidate consumed.
        {
            let Some(club) = data.club(club_id) else {
                return;
            };
            if !ManagerSeat::club_has_vacancy(club) {
                debug!(
                    "Manager market: refusing appointment at club {} — permanent manager still in post",
                    club_id
                );
                if let Some(club_mut) = data.club_mut(club_id) {
                    ManagerSearch::clear(&mut club_mut.board);
                }
                return;
            }
        }

        // Step 1: Strip the caretaker tag. The caretaker is demoted
        // back to a generic Coach role so the seat is empty when we
        // push the new permanent appointment. We don't try to remember
        // the coach's pre-promotion role — simplification doesn't
        // materially affect simulation depth.
        let club_name: String;
        {
            let Some(club) = data.club_mut(club_id) else {
                return;
            };
            club_name = club.name.clone();
            if let Some(main_team) = club.teams.main_mut() {
                ManagerSeat::clear_caretaker(main_team);
            }
        }

        // Step 2: Identify the top free-agent candidate (id + agreed
        // salary) by reading the shortlist. We don't pop yet — the
        // staff might no longer be in the pool (signed by another club
        // this tick); only pop after we confirm the move.
        let top_candidate: Option<(u32, u32)> = {
            let Some(club) = data.club(club_id) else {
                return;
            };
            club.board
                .manager_shortlist
                .iter()
                .find(|c| matches!(c.source, CandidateSource::FreeAgent))
                .map(|c| (c.staff_id, c.target_salary))
        };

        // Step 3: Try to take the candidate from the pool. If the slot
        // has been signed already, we drop the entry and fall through
        // to the caretaker-promotion path. Pool access requires no
        // club borrow.
        let signed: Option<(Staff, u32)> = if let Some((staff_id, salary)) = top_candidate {
            let pool = &mut data.free_agent_staff;
            let removed = pool
                .iter()
                .position(|s| s.id == staff_id)
                .map(|idx| pool.remove(idx));
            removed.map(|staff| (staff, salary))
        } else {
            None
        };

        // Step 4: Install the new manager (or fallback). All work back
        // inside the club borrow.
        {
            let Some(club) = data.club_mut(club_id) else {
                return;
            };

            if let Some((mut new_manager, salary)) = signed {
                let new_id = new_manager.id;
                new_manager.contract = Some(ManagerSeat::build_manager_contract(salary, today));
                new_manager.job_satisfaction = 70.0; // Fresh start: optimistic.
                if let Some(main_team) = club.teams.main_mut() {
                    main_team.staffs.push(new_manager);
                    debug!(
                        "Free-agent signed: staff {} appointed manager at {} ({}/y)",
                        new_id, club_name, salary
                    );
                }
            } else if let Some(main_team) = club.teams.main_mut() {
                // Fallback: ex-caretaker (now Coach) → permanent
                // Manager on a 3-year deal at their existing salary.
                // The board takes the conservative option when no
                // realistic free agent stepped up during the search
                // window.
                if let Some(staff) = main_team.staffs.find_mut_by_position(StaffPosition::Coach) {
                    let salary = staff.contract.as_ref().map(|c| c.salary).unwrap_or(0);
                    let id = staff.id;
                    staff.contract = Some(ManagerSeat::build_manager_contract(salary, today));
                    debug!(
                        "Caretaker {} confirmed as permanent manager at {} (no free-agent shortlist)",
                        id, club_name
                    );
                }
            }

            // Fresh appointment — wipe chairman loyalty toward the
            // predecessor, clear all search state.
            club.board.chairman.manager_loyalty = 50;
            ManagerSearch::clear(&mut club.board);
        }
    }
}

// ─── ManagerApproach — poaching state machine ──────────────────────────

impl ManagerApproach {
    /// True when this approach's `staff_id` is still the permanent
    /// `Manager` at `source_club_id`. Used as a guard at every state
    /// transition that would otherwise disturb the source club — a
    /// 5-day approach window leaves room for the target to be sacked,
    /// poached out from underneath, or have their contract lapse, so
    /// we re-verify each time we touch the source.
    fn target_is_source_manager(&self, data: &SimulatorData) -> bool {
        let Some(src) = data.club(self.source_club_id) else {
            return false;
        };
        let Some(main) = src.teams.main() else {
            return false;
        };
        let Some(mgr) = main.staffs.find_by_position(StaffPosition::Manager) else {
            return false;
        };
        if mgr.id != self.staff_id {
            return false;
        }
        mgr.contract.is_some()
    }

    /// Decide the next state for this approach based on the current
    /// state + the live state of the involved clubs. Side effects
    /// (moving staff, cascading search) are applied here too — the
    /// state-machine and effects are interleaved deliberately so a
    /// successful TermsAccepted transition immediately performs the
    /// move.
    fn advance(&self, data: &mut SimulatorData, today: NaiveDate) -> Option<ApproachState> {
        use ApproachState::*;

        match self.state {
            Made => {
                // Target sanity: the staff must still be the source
                // club's permanent manager. If they were sacked,
                // demoted, or moved elsewhere since the approach was
                // queued, drop it now — before the source club is ever
                // asked to entertain talks.
                if !self.target_is_source_manager(data) {
                    debug!(
                        "Approach rejected: staff {} no longer manager at source club {}",
                        self.staff_id, self.source_club_id
                    );
                    return Some(Rejected);
                }

                // Source club decides whether to entertain the approach.
                let (source_conf, source_overperforming, source_rep) = {
                    let Some(src) = data.club(self.source_club_id) else {
                        return Some(Rejected);
                    };
                    let conf = src.board.confidence.level;
                    let overperf = src.board.confidence.level >= 70;
                    let rep = src
                        .teams
                        .iter()
                        .find(|t| matches!(t.team_type, TeamType::Main))
                        .map(|t| t.reputation.world)
                        .unwrap_or(0);
                    (conf, overperf, rep)
                };
                if ManagerCandidateScorer::source_refuses_outright(
                    source_conf,
                    source_overperforming,
                ) {
                    debug!(
                        "Approach rejected: source club {} won't release manager",
                        self.source_club_id
                    );
                    return Some(Rejected);
                }

                // Look up the manager's current contract to compute
                // compensation. The previous guard already confirmed
                // the target still holds Manager + has a contract; the
                // let-else ladder is kept defensive against concurrent
                // mutation.
                let (current_salary, days_left) = {
                    let Some(src) = data.club(self.source_club_id) else {
                        return Some(Rejected);
                    };
                    let Some(main) = src
                        .teams
                        .iter()
                        .find(|t| matches!(t.team_type, TeamType::Main))
                    else {
                        return Some(Rejected);
                    };
                    let Some(mgr) = main.staffs.find_by_position(StaffPosition::Manager) else {
                        return Some(Rejected);
                    };
                    let Some(contract) = mgr.contract.as_ref() else {
                        return Some(Rejected);
                    };
                    let days_left = (contract.expired - today).num_days().max(30);
                    (contract.salary, days_left)
                };

                let years_left = (days_left as f32 / 365.0).max(0.5);
                let mult = ManagerCandidateScorer::compensation_multiplier(source_rep);
                let demand = ((current_salary as f32) * years_left * mult) as u32;
                Some(CompensationDemanded { amount: demand })
            }

            CompensationDemanded { amount } => {
                // Vacancy re-check before money changes hands. If the
                // requesting club already filled the seat (parallel
                // free-agent hire, caretaker confirmation, or another
                // poach finalising on this same tick), reject without
                // paying — compensation is wasted spend on an approach
                // that finalize would refuse anyway.
                let requesting_vacant = data
                    .club(self.requesting_club_id)
                    .map(|c| ManagerSeat::club_has_vacancy(c))
                    .unwrap_or(false);
                if !requesting_vacant {
                    debug!(
                        "Approach rejected: club {} already filled the seat — no compensation paid",
                        self.requesting_club_id
                    );
                    return Some(Rejected);
                }
                // Target re-check: the source manager may have moved
                // on since the demand was issued. Don't pay
                // compensation for someone who is no longer there.
                if !self.target_is_source_manager(data) {
                    debug!(
                        "Approach rejected: source target {} no longer Manager — no compensation paid",
                        self.staff_id
                    );
                    return Some(Rejected);
                }
                // Requesting club checks whether they can stomach the
                // compensation. Cap: 30% of cash balance, with a hard
                // floor of 200k so smaller clubs can still poach.
                let can_pay = {
                    let Some(req) = data.club(self.requesting_club_id) else {
                        return Some(Rejected);
                    };
                    let cap = ((req.finance.balance.balance as f32) * 0.30) as i64;
                    let cap = cap.max(200_000);
                    (amount as i64) <= cap
                };
                if !can_pay {
                    debug!(
                        "Approach rejected: club {} can't afford {} compensation for staff {}",
                        self.requesting_club_id, amount, self.staff_id
                    );
                    return Some(Rejected);
                }
                // Pay it. We model the outflow on the requesting club
                // — source club's books aren't credited here because
                // the existing finance model is a single balance, and
                // we're not introducing transfer-fee accounting for
                // staff in this slice. Posted as staff-wages expense
                // so it shows up in the same line as the new manager's
                // salary.
                if let Some(req) = data.club_mut(self.requesting_club_id) {
                    req.finance.balance.push_expense_staff_wages(amount as i64);
                }
                Some(CompensationAgreed)
            }

            CompensationAgreed => {
                // Final sanity check before personal-terms negotiation
                // — the candidate cannot accept terms from a club they
                // no longer work for, or for a seat that's already
                // filled.
                if !self.target_is_source_manager(data) {
                    debug!(
                        "Approach rejected: source target {} no longer Manager pre-terms",
                        self.staff_id
                    );
                    return Some(Rejected);
                }
                // Personal terms: candidate compares offered_salary +
                // requesting-club prestige against current package.
                let accepted = ManagerCandidateScorer::candidate_accepts_terms(data, self);
                if accepted {
                    Some(TermsAccepted)
                } else {
                    debug!(
                        "Approach rejected: candidate {} rejected personal terms from club {}",
                        self.staff_id, self.requesting_club_id
                    );
                    Some(Rejected)
                }
            }

            TermsAccepted => {
                // Finalize: move staff from source to requesting;
                // cascade source search; clear requesting club's
                // search state.
                self.finalize(data, today);
                // Mark Rejected so the registry cleanup pass removes
                // the entry next tick. (We could add a `Signed`
                // terminal state, but the cleanup behaviour is
                // identical.)
                Some(Rejected)
            }

            Rejected => None,
        }
    }

    /// Move the staff member from source to requesting club, install
    /// them as the new manager, clear the requesting club's search
    /// state, and open a fresh manager search on the source club
    /// (cascade).
    fn finalize(&self, data: &mut SimulatorData, today: NaiveDate) {
        // Vacancy re-check: an approach takes ~5 days to mature, and
        // the requesting club's seat may have been filled in the
        // meantime (free-agent hire, caretaker confirmation, parallel
        // poach). If it has, abort without disturbing the source club
        // — compensation has already been paid, but we won't move the
        // manager and we won't leak a duplicate. The pending approach
        // is reaped via `Rejected` in the caller.
        let requesting_vacant = data
            .club(self.requesting_club_id)
            .map(|c| ManagerSeat::club_has_vacancy(c))
            .unwrap_or(false);
        if !requesting_vacant {
            info!(
                "Approach aborted: club {} already filled the seat before staff {} could be poached",
                self.requesting_club_id, self.staff_id
            );
            return;
        }

        // Source target re-check: only poach a staff member who is
        // still the source club's permanent `Manager`. If the target
        // has been sacked, demoted, or already poached elsewhere since
        // the approach began, bail out — `take_by_id` would otherwise
        // move a Coach or Assistant who happens to share the id.
        if !self.target_is_source_manager(data) {
            info!(
                "Approach aborted: staff {} is no longer the manager at source club {}",
                self.staff_id, self.source_club_id
            );
            return;
        }

        // Step 1: take the staff out of the source club's main team.
        // We pull by position (verified above to be the same id) so a
        // stale id can't inadvertently relocate a non-manager. Capture
        // the departing salary while we still hold the borrow — the
        // source club's caretaker promotion uses it as the salary
        // floor below.
        let mut staff: Option<Staff> = None;
        let mut source_salary: u32 = 0;
        if let Some(src) = data.club_mut(self.source_club_id) {
            if let Some(main) = src.teams.main_mut() {
                if let Some(taken) = main.staffs.take_by_position(StaffPosition::Manager) {
                    source_salary = taken.contract.as_ref().map(|c| c.salary).unwrap_or(0);
                    staff = Some(taken);
                }
            }
        }

        let Some(mut staff) = staff else {
            return;
        };

        // Step 2: install on requesting club. Demote any sitting
        // caretaker first so the head-coach seat is unique. Reset
        // relations so the new manager doesn't carry stale player
        // rapport from the old squad.
        let new_id = staff.id;
        staff.contract = Some(ManagerSeat::build_manager_contract(
            self.offered_salary,
            today,
        ));
        staff.relations = crate::Relations::new();
        staff.fatigue = 0.0;
        staff.job_satisfaction = 75.0; // Fresh job: optimistic.

        let mut signed = false;
        if let Some(req) = data.club_mut(self.requesting_club_id) {
            if let Some(main) = req.teams.main_mut() {
                ManagerSeat::clear_caretaker(main);
                main.staffs.push(staff);
                signed = true;
                info!(
                    "Manager poached: staff {} → club {} (compensation paid, terms agreed)",
                    new_id, self.requesting_club_id
                );
            }
            // Clear requesting club's search state — the seat is filled.
            req.board.chairman.manager_loyalty = 50;
            ManagerSearch::clear(&mut req.board);
        }

        if !signed {
            // Defensive: requesting club lookup failed during the
            // install step (only reachable if the requesting club was
            // deleted mid-tick — not a path the simulator currently
            // exercises). The staff is already taken from source;
            // nothing left to do for them here, log so we'd notice if
            // this ever fires.
            log::warn!(
                "Manager market: lost staff {} mid-finalize for club {}",
                new_id,
                self.requesting_club_id
            );
            return;
        }

        // Step 3: cascade — source club's seat is now vacant. Open a
        // fresh search on them with their rep-scaled window AND
        // promote an interim caretaker so match/training code keeps a
        // head_coach() fallback. This chain reaction makes the world
        // feel alive: one Real Madrid hire of a Bayer Leverkusen coach
        // forces Leverkusen into their own search, which in turn might
        // poach from a Bundesliga rival, and so on.
        if let Some(src) = data.club_mut(self.source_club_id) {
            let src_rep = src
                .teams
                .iter()
                .find(|t| matches!(t.team_type, TeamType::Main))
                .map(|t| t.reputation.world)
                .unwrap_or(0);
            if let Some(main) = src.teams.main_mut() {
                ManagerSeat::promote_best_caretaker(main, source_salary, today);
            }
            ManagerSearch::open(&mut src.board, today, src_rep);
            // Reset confidence so the new search starts on neutral
            // footing — the departing manager's good results don't
            // become a head-wind against finding a successor.
            src.board.confidence.level = 50;
            src.board.poor_mood_months = 0;
            info!(
                "Cascade: club {} enters manager search after losing staff {}",
                self.source_club_id, new_id
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::StaffStub;

    fn coach(id: u32, age: u8, today: NaiveDate, skill: u8) -> Staff {
        let mut s = StaffStub::default();
        s.id = id;
        s.birth_date = NaiveDate::from_ymd_opt(today.year() - age as i32, 1, 1).unwrap();
        s.staff_attributes.coaching.tactical = skill;
        s.staff_attributes.coaching.mental = skill;
        s.staff_attributes.mental.man_management = skill;
        s.staff_attributes.mental.motivating = skill;
        s.staff_attributes.knowledge.tactical_knowledge = skill;
        s
    }

    #[test]
    fn search_window_scales_with_rep() {
        assert!(
            ManagerCandidateScorer::search_window_days(9000)
                > ManagerCandidateScorer::search_window_days(3000)
        );
        assert!(
            ManagerCandidateScorer::search_window_days(3000)
                > ManagerCandidateScorer::search_window_days(500)
        );
    }

    #[test]
    fn higher_skill_scores_higher_at_matched_rep() {
        let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
        let weak = coach(1, 45, today, 8);
        let strong = coach(2, 45, today, 16);

        let weak_score = ManagerCandidateScorer::score_free_agent(&weak, 5000, today).unwrap();
        let strong_score = ManagerCandidateScorer::score_free_agent(&strong, 5000, today).unwrap();

        assert!(strong_score > weak_score);
    }

    #[test]
    fn very_old_candidate_takes_age_penalty() {
        let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
        let young = coach(1, 45, today, 14);
        let old = coach(2, 68, today, 14);

        let ys = ManagerCandidateScorer::score_free_agent(&young, 5000, today).unwrap();
        let os = ManagerCandidateScorer::score_free_agent(&old, 5000, today).unwrap();
        assert!(ys > os);
    }

    #[test]
    fn shortlist_returns_top_n_sorted() {
        let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
        let pool: Vec<Staff> = (1..=10).map(|i| coach(i, 45, today, i as u8)).collect();

        let shortlist = ManagerShortlist::from_free_agents(&pool, 6000, today);
        assert_eq!(shortlist.len(), ManagerShortlist::MAX_LEN);
        for w in shortlist.windows(2) {
            assert!(w[0].fit_score >= w[1].fit_score);
        }
        assert_eq!(shortlist[0].staff_id, 10);
    }

    #[test]
    fn target_salary_grows_with_rep() {
        let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
        let s = coach(1, 45, today, 14);
        let small = ManagerCandidateScorer::target_salary(&s, 1500, today);
        let big = ManagerCandidateScorer::target_salary(&s, 8500, today);
        assert!(big > small * 3);
    }

    #[test]
    fn take_top_free_agent_drains_pool_and_shortlist() {
        let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
        let pool_staff = coach(42, 45, today, 14);
        let mut pool = vec![pool_staff];

        let mut board = ClubBoard::new();
        board.manager_shortlist = vec![ManagerCandidate {
            staff_id: 42,
            fit_score: 100,
            target_salary: 250_000,
            source: CandidateSource::FreeAgent,
        }];

        let result = ManagerSearch::take_top_free_agent(&mut board, &mut pool);
        assert!(result.is_some());
        let (staff, salary) = result.unwrap();
        assert_eq!(staff.id, 42);
        assert_eq!(salary, 250_000);
        assert!(pool.is_empty());
        assert!(board.manager_shortlist.is_empty());
    }

    #[test]
    fn build_manager_contract_runs_three_years() {
        let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
        let c: StaffClubContract = ManagerSeat::build_manager_contract(200_000, today);
        assert_eq!(c.salary, 200_000);
        assert_eq!(c.position, StaffPosition::Manager);
        assert_eq!(c.status, StaffStatus::Active);
        assert_eq!(c.expired.year(), today.year() + 3);
    }

    // ─── Slice C: poaching state-machine tests ───────────────────────────

    #[test]
    fn confident_overperforming_source_refuses() {
        assert!(ManagerCandidateScorer::source_refuses_outright(85, true));
        assert!(!ManagerCandidateScorer::source_refuses_outright(60, true));
        assert!(!ManagerCandidateScorer::source_refuses_outright(85, false));
    }

    #[test]
    fn compensation_multiplier_scales_with_rep() {
        assert!(
            ManagerCandidateScorer::compensation_multiplier(8000)
                > ManagerCandidateScorer::compensation_multiplier(5000)
        );
        assert!(
            ManagerCandidateScorer::compensation_multiplier(5000)
                > ManagerCandidateScorer::compensation_multiplier(1000)
        );
    }

    #[test]
    fn employed_candidate_takes_friction_penalty() {
        let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
        let s = coach(1, 45, today, 14);
        let free = ManagerCandidateScorer::score_free_agent(&s, 6000, today).unwrap();
        let employed = ManagerCandidateScorer::score_employed(&s, 6000, today).unwrap();
        assert!(employed < free);
    }

    // ─── Manager-seat invariant tests ────────────────────────────────────

    use crate::academy::ClubAcademy;
    use crate::competitions::GlobalCompetitions;
    use crate::continent::Continent;
    use crate::league::LeagueCollection;
    use crate::shared::Location;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, Country, PlayerCollection,
        StaffCollection, TeamBuilder, TeamCollection, TeamReputation, TrainingSchedule,
    };

    fn make_training_schedule() -> TrainingSchedule {
        use chrono::NaiveTime;
        TrainingSchedule::new(
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
        )
    }

    fn manager_contract(
        salary: u32,
        today: NaiveDate,
        position: StaffPosition,
    ) -> StaffClubContract {
        let expires = today.with_year(today.year() + 2).unwrap_or(today);
        StaffClubContract::new(salary, expires, position, StaffStatus::Active)
    }

    fn coach_with_contract(
        id: u32,
        today: NaiveDate,
        position: StaffPosition,
        salary: u32,
    ) -> Staff {
        let mut s = coach(id, 45, today, 14);
        s.contract = Some(manager_contract(salary, today, position));
        s
    }

    fn make_main_team(team_id: u32, club_id: u32, staffs: Vec<Staff>) -> crate::Team {
        TeamBuilder::new()
            .id(team_id)
            .league_id(Some(1))
            .club_id(club_id)
            .name(format!("Team{}", team_id))
            .slug(format!("team{}", team_id))
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(Vec::new()))
            .staffs(StaffCollection::new(staffs))
            .reputation(TeamReputation::new(3000, 3000, 3000))
            .training_schedule(make_training_schedule())
            .build()
            .unwrap()
    }

    fn make_club_with_main(id: u32, staffs: Vec<Staff>) -> crate::Club {
        let team = make_main_team(id * 10, id, staffs);
        crate::Club::new(
            id,
            format!("Club{}", id),
            Location::new(1),
            ClubFinances::new(10_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(vec![team]),
            ClubFacilities::default(),
        )
    }

    fn make_data(today: NaiveDate, clubs: Vec<crate::Club>) -> SimulatorData {
        let country = Country::builder()
            .id(1)
            .code("EN".to_string())
            .slug("england".to_string())
            .name("England".to_string())
            .continent_id(1)
            .leagues(LeagueCollection::new(Vec::new()))
            .clubs(clubs)
            .build()
            .unwrap();
        let continent = Continent::new(1, "Europe".to_string(), vec![country], Vec::new());
        SimulatorData::new(
            today.and_hms_opt(12, 0, 0).unwrap(),
            vec![continent],
            GlobalCompetitions::new(Vec::new()),
        )
    }

    fn count_managers(data: &SimulatorData, club_id: u32) -> usize {
        let club = data.club(club_id).unwrap();
        let main = club.teams.main().unwrap();
        main.staffs
            .iter()
            .filter(|s| {
                s.contract
                    .as_ref()
                    .map(|c| matches!(c.position, StaffPosition::Manager))
                    .unwrap_or(false)
            })
            .count()
    }

    fn count_caretakers(data: &SimulatorData, club_id: u32) -> usize {
        let club = data.club(club_id).unwrap();
        let main = club.teams.main().unwrap();
        main.staffs
            .iter()
            .filter(|s| {
                s.contract
                    .as_ref()
                    .map(|c| matches!(c.position, StaffPosition::CaretakerManager))
                    .unwrap_or(false)
            })
            .count()
    }

    /// Combined head-coach seat count — anything filling the role,
    /// permanent or interim. The seat is unique by definition, so this
    /// must always be 0 or 1.
    fn count_head_coaches(data: &SimulatorData, club_id: u32) -> usize {
        count_managers(data, club_id) + count_caretakers(data, club_id)
    }

    #[test]
    fn execute_appointment_does_not_add_second_manager_when_permanent_manager_exists() {
        let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
        let incumbent = coach_with_contract(100, today, StaffPosition::Manager, 200_000);
        let club = make_club_with_main(1, vec![incumbent]);
        let mut data = make_data(today, vec![club]);

        let candidate = coach(42, 45, today, 14);
        data.free_agent_staff.push(candidate);

        if let Some(club) = data.club_mut(1) {
            club.board.manager_search_since = Some(NaiveDate::from_ymd_opt(2030, 4, 1).unwrap());
            club.board.search_window_days = 30;
            club.board.manager_shortlist = vec![ManagerCandidate {
                staff_id: 42,
                fit_score: 100,
                target_salary: 250_000,
                source: CandidateSource::FreeAgent,
            }];
        }

        ManagerMarketTick::execute_appointment(&mut data, 1, today);

        assert_eq!(
            count_managers(&data, 1),
            1,
            "must still have exactly one permanent manager"
        );
        assert_eq!(count_head_coaches(&data, 1), 1, "head-coach seat is unique");
        assert!(
            data.free_agent_staff.iter().any(|s| s.id == 42),
            "candidate should remain in the pool"
        );
        let club = data.club(1).unwrap();
        assert!(club.board.manager_search_since.is_none());
        assert!(club.board.manager_shortlist.is_empty());
    }

    #[test]
    fn execute_appointment_replaces_caretaker_with_permanent_manager() {
        let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
        let caretaker = coach_with_contract(50, today, StaffPosition::CaretakerManager, 80_000);
        let club = make_club_with_main(1, vec![caretaker]);
        let mut data = make_data(today, vec![club]);

        let candidate = coach(42, 45, today, 14);
        data.free_agent_staff.push(candidate);

        if let Some(club) = data.club_mut(1) {
            club.board.manager_search_since = Some(NaiveDate::from_ymd_opt(2030, 4, 1).unwrap());
            club.board.search_window_days = 30;
            club.board.manager_shortlist = vec![ManagerCandidate {
                staff_id: 42,
                fit_score: 100,
                target_salary: 250_000,
                source: CandidateSource::FreeAgent,
            }];
        }

        ManagerMarketTick::execute_appointment(&mut data, 1, today);

        assert_eq!(count_managers(&data, 1), 1);
        assert_eq!(count_caretakers(&data, 1), 0);
        assert_eq!(count_head_coaches(&data, 1), 1);
        assert!(
            data.free_agent_staff.iter().all(|s| s.id != 42),
            "candidate should be drawn from the pool"
        );
        let club = data.club(1).unwrap();
        let main = club.teams.main().unwrap();
        assert!(main.staffs.iter().any(|s| s.id == 42));
        assert!(club.board.manager_search_since.is_none());
    }

    #[test]
    fn finalize_approach_does_not_poach_into_filled_seat() {
        let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
        let source_mgr = coach_with_contract(200, today, StaffPosition::Manager, 300_000);
        let source = make_club_with_main(2, vec![source_mgr]);
        let req_mgr = coach_with_contract(100, today, StaffPosition::Manager, 250_000);
        let requesting = make_club_with_main(1, vec![req_mgr]);
        let mut data = make_data(today, vec![requesting, source]);

        let approach = ManagerApproach {
            requesting_club_id: 1,
            source_club_id: 2,
            staff_id: 200,
            state: ApproachState::TermsAccepted,
            offered_salary: 350_000,
            created_at: today - chrono::Duration::days(5),
            last_action: today - chrono::Duration::days(1),
        };
        data.pending_manager_approaches.push(approach);

        ManagerMarketTick::tick_approaches(&mut data);

        assert_eq!(
            count_managers(&data, 2),
            1,
            "source manager must not be removed"
        );
        let src_club = data.club(2).unwrap();
        let src_main = src_club.teams.main().unwrap();
        assert!(
            src_main.staffs.iter().any(|s| s.id == 200),
            "target staff must remain on source roster"
        );
        assert_eq!(count_managers(&data, 1), 1);
        assert_eq!(count_head_coaches(&data, 1), 1);
        assert!(src_club.board.manager_search_since.is_none());
        assert!(data.pending_manager_approaches.is_empty());
    }

    #[test]
    fn finalize_approach_opens_source_search_after_successful_poach() {
        let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
        let source_mgr = coach_with_contract(200, today, StaffPosition::Manager, 300_000);
        let source_coach = coach_with_contract(201, today, StaffPosition::Coach, 60_000);
        let source = make_club_with_main(2, vec![source_mgr, source_coach]);
        let caretaker = coach_with_contract(50, today, StaffPosition::CaretakerManager, 80_000);
        let requesting = make_club_with_main(1, vec![caretaker]);
        let mut data = make_data(today, vec![requesting, source]);

        if let Some(req) = data.club_mut(1) {
            req.board.manager_search_since = Some(NaiveDate::from_ymd_opt(2030, 5, 1).unwrap());
            req.board.search_window_days = 30;
        }

        let approach = ManagerApproach {
            requesting_club_id: 1,
            source_club_id: 2,
            staff_id: 200,
            state: ApproachState::TermsAccepted,
            offered_salary: 350_000,
            created_at: today - chrono::Duration::days(5),
            last_action: today - chrono::Duration::days(1),
        };
        data.pending_manager_approaches.push(approach);

        ManagerMarketTick::tick_approaches(&mut data);

        assert_eq!(count_managers(&data, 1), 1);
        assert_eq!(
            count_caretakers(&data, 1),
            0,
            "old caretaker must be demoted"
        );
        assert_eq!(count_head_coaches(&data, 1), 1);
        let req_club = data.club(1).unwrap();
        assert!(req_club.board.manager_search_since.is_none());
        assert!(
            req_club
                .teams
                .main()
                .unwrap()
                .staffs
                .iter()
                .any(|s| s.id == 200)
        );

        let src_club = data.club(2).unwrap();
        assert_eq!(src_club.board.manager_search_since, Some(today));
        assert!(src_club.board.search_window_days > 0);
        assert_eq!(count_managers(&data, 2), 0);
        assert_eq!(
            count_caretakers(&data, 2),
            1,
            "source must get an interim caretaker"
        );
        assert_eq!(count_head_coaches(&data, 2), 1);
        assert!(
            src_club
                .teams
                .main()
                .unwrap()
                .staffs
                .iter()
                .all(|s| s.id != 200)
        );
    }

    #[test]
    fn refresh_shortlists_clears_stale_search_when_permanent_manager_in_post() {
        let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
        let incumbent = coach_with_contract(100, today, StaffPosition::Manager, 200_000);
        let club = make_club_with_main(1, vec![incumbent]);
        let mut data = make_data(today, vec![club]);

        if let Some(club) = data.club_mut(1) {
            club.board.manager_search_since = Some(today - chrono::Duration::days(20));
            club.board.search_window_days = 30;
            club.board.manager_shortlist = vec![ManagerCandidate {
                staff_id: 999,
                fit_score: 50,
                target_salary: 100_000,
                source: CandidateSource::FreeAgent,
            }];
            club.board.shortlist_built_at = Some(today - chrono::Duration::days(20));
        }

        ManagerMarketTick::refresh_shortlists(&mut data);

        let club = data.club(1).unwrap();
        assert!(
            club.board.manager_search_since.is_none(),
            "stale search must be cleared when permanent manager is still in post"
        );
        assert!(club.board.manager_shortlist.is_empty());
    }

    #[test]
    fn compensation_not_paid_when_requesting_seat_filled() {
        let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
        let source_mgr = coach_with_contract(200, today, StaffPosition::Manager, 300_000);
        let source = make_club_with_main(2, vec![source_mgr]);
        let req_mgr = coach_with_contract(100, today, StaffPosition::Manager, 250_000);
        let requesting = make_club_with_main(1, vec![req_mgr]);
        let mut data = make_data(today, vec![requesting, source]);

        let starting_balance = data.club(1).unwrap().finance.balance.balance;
        let starting_outcome = data.club(1).unwrap().finance.balance.outcome;

        let approach = ManagerApproach {
            requesting_club_id: 1,
            source_club_id: 2,
            staff_id: 200,
            state: ApproachState::CompensationDemanded { amount: 500_000 },
            offered_salary: 350_000,
            created_at: today - chrono::Duration::days(3),
            last_action: today - chrono::Duration::days(1),
        };
        data.pending_manager_approaches.push(approach);

        ManagerMarketTick::tick_approaches(&mut data);

        let req = data.club(1).unwrap();
        assert_eq!(req.finance.balance.balance, starting_balance);
        assert_eq!(req.finance.balance.outcome, starting_outcome);
        assert_eq!(count_managers(&data, 2), 1);
        assert!(data.pending_manager_approaches.is_empty());
    }

    #[test]
    fn approach_rejects_when_source_target_no_longer_manager() {
        let today = NaiveDate::from_ymd_opt(2030, 6, 1).unwrap();
        let demoted = coach_with_contract(200, today, StaffPosition::Coach, 60_000);
        let real_mgr = coach_with_contract(201, today, StaffPosition::Manager, 280_000);
        let source = make_club_with_main(2, vec![demoted, real_mgr]);
        let caretaker = coach_with_contract(50, today, StaffPosition::CaretakerManager, 80_000);
        let requesting = make_club_with_main(1, vec![caretaker]);
        let mut data = make_data(today, vec![requesting, source]);

        if let Some(req) = data.club_mut(1) {
            req.board.manager_search_since = Some(NaiveDate::from_ymd_opt(2030, 5, 1).unwrap());
            req.board.search_window_days = 30;
        }

        let starting_balance = data.club(1).unwrap().finance.balance.balance;

        let approach = ManagerApproach {
            requesting_club_id: 1,
            source_club_id: 2,
            staff_id: 200,
            state: ApproachState::Made,
            offered_salary: 350_000,
            created_at: today - chrono::Duration::days(1),
            last_action: today - chrono::Duration::days(1),
        };
        data.pending_manager_approaches.push(approach);

        ManagerMarketTick::tick_approaches(&mut data);

        assert_eq!(count_managers(&data, 2), 1);
        let src_club = data.club(2).unwrap();
        assert!(
            src_club.board.manager_search_since.is_none(),
            "no cascade for a rejected approach"
        );
        let src_main = src_club.teams.main().unwrap();
        assert!(src_main.staffs.iter().any(|s| s.id == 200));
        assert!(src_main.staffs.iter().any(|s| s.id == 201));
        assert_eq!(count_managers(&data, 1), 0);
        assert_eq!(count_caretakers(&data, 1), 1);
        assert_eq!(
            data.club(1).unwrap().finance.balance.balance,
            starting_balance
        );
        assert!(data.pending_manager_approaches.is_empty());
    }
}
