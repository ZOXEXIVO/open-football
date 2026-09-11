use crate::TeamType;
use crate::club::Club;
use crate::club::board::manager::candidate::{
    CandidateSource, EmployedCandidateRaw, ManagerCandidate,
};
use crate::club::board::manager::scorer::ManagerCandidateScorer;
use crate::club::staff::StaffPosition;
use crate::{SimulatorData, Staff};
use chrono::NaiveDate;
use rayon::prelude::*;

pub struct ManagerShortlist;

impl ManagerShortlist {
    /// Share of the club's wage mandate a head coach's salary may take.
    ///
    /// Nothing used to bound it at all: `target_salary` was both what the
    /// candidate wanted and what the club paid, so a relegation-threatened
    /// side would hand out a half-million deal because the reputation
    /// arithmetic said so. Real boards appoint inside the wage bill.
    pub const WAGE_SHARE_CAP: f32 = 0.06;

    /// How far an owner who is funding the club himself will stretch that
    /// ceiling. A benefactor's cheque is exactly the thing that lets a club
    /// pay a manager its revenue cannot explain.
    pub const RICH_OWNER_STRETCH: f32 = 1.5;

    /// Injection appetite past which the owner counts as writing cheques.
    const RICH_OWNER_APPETITE: f32 = 0.6;

    /// The most this club will pay a head coach, or `None` when it has no
    /// wage mandate to measure against and is therefore unconstrained.
    pub fn salary_ceiling(club: &Club) -> Option<u32> {
        let mandate = club.board.season_targets.as_ref()?.adjusted_wage_budget();
        if mandate <= 0 {
            return None;
        }
        let mut ceiling = mandate as f32 * Self::WAGE_SHARE_CAP;
        if club.board.ownership.injection_appetite() > Self::RICH_OWNER_APPETITE {
            ceiling *= Self::RICH_OWNER_STRETCH;
        }
        Some(ceiling as u32)
    }

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

    /// Walk the world ONCE and collect every poachable in-post manager
    /// with the requester-independent filters already applied: a Main
    /// team is present, the board has a season target set, the board is
    /// happy (confidence ≥ 70 ≈ the manager is over-delivering), and a
    /// Manager actually occupies the seat. The per-requester reputation
    /// ceiling and scoring are applied later in [`combined`] against
    /// this shared snapshot.
    ///
    /// Parallel read-only sweep over `data` — runs ONCE per shortlist
    /// refresh tick, replacing the previous per-club world re-walk
    /// (`O(searching_clubs × all_clubs)` → `O(all_clubs)`).
    ///
    /// Cross-border by default — a Premier League club can poach from
    /// La Liga or the Eredivisie; any continent is fair game.
    pub(crate) fn enumerate_employed_pool(data: &SimulatorData) -> Vec<EmployedCandidateRaw<'_>> {
        data.continents
            .par_iter()
            .flat_map(|c| c.countries.par_iter())
            .flat_map(|country| country.clubs.par_iter())
            .filter_map(|club| {
                let main_team = club
                    .teams
                    .iter()
                    .find(|t| matches!(t.team_type, TeamType::Main))?;
                // Performance filter: only consider managers at clubs
                // outperforming their season target. The season-target
                // presence and confidence checks are requester-
                // independent, so they belong in the one-time walk.
                club.board.season_targets.as_ref()?;
                // Overperforming proxy: confidence >= 70 means the board
                // is happy, i.e. the manager is over-delivering. Cheaper
                // than re-deriving league standings here.
                if club.board.confidence.level < 70 {
                    return None;
                }
                let manager = main_team.staffs.find_by_position(StaffPosition::Manager)?;
                Some(EmployedCandidateRaw {
                    manager,
                    club_id: club.id,
                    club_world_rep: main_team.reputation.world,
                })
            })
            .collect()
    }

    /// Build the full shortlist for a club — free agents + viable
    /// employed targets, merged and ranked. The employed candidates are
    /// filtered out of the shared `employed` snapshot built once by
    /// [`enumerate_employed_pool`]: only clubs whose reputation is at
    /// most `rep_ceiling` (i.e. strictly smaller than the requester by a
    /// margin) survive, each rescored against the requester's
    /// reputation. Cheap per-requester pass — no world walk here.
    pub(crate) fn combined(
        free_agent_staff: &[Staff],
        employed: &[EmployedCandidateRaw<'_>],
        requesting_club_id: u32,
        requesting_rep: u16,
        salary_ceiling: Option<u32>,
        today: NaiveDate,
    ) -> Vec<ManagerCandidate> {
        let mut combined: Vec<ManagerCandidate> =
            Self::from_free_agents(free_agent_staff, requesting_rep, today);

        let rep_ceiling = ((requesting_rep as f32) * 0.8) as u16;
        for raw in employed {
            // Only poach from clubs strictly smaller than us by a margin.
            if raw.club_world_rep > rep_ceiling {
                continue;
            }
            // Don't shortlist the club's own current manager (paranoia
            // — the rep-ceiling filter should already exclude self, but
            // a club at the rep boundary could match itself).
            if raw.club_id == requesting_club_id {
                continue;
            }
            let Some(score) =
                ManagerCandidateScorer::score_employed(raw.manager, requesting_rep, today)
            else {
                continue;
            };
            let target_salary =
                ManagerCandidateScorer::target_salary(raw.manager, requesting_rep, today);
            combined.push(ManagerCandidate {
                staff_id: raw.manager.id,
                fit_score: score,
                target_salary,
                source: CandidateSource::Employed {
                    current_club_id: raw.club_id,
                },
            });
        }

        // A man the club cannot pay is not a candidate. Filtered after
        // scoring rather than inside it, so the ceiling is a budget
        // decision rather than a judgement about the coach.
        if let Some(ceiling) = salary_ceiling {
            combined.retain(|c| c.target_salary <= ceiling);
        }

        combined.sort_unstable_by(|a, b| b.fit_score.cmp(&a.fit_score));
        combined.truncate(Self::MAX_LEN);
        combined
    }
}
