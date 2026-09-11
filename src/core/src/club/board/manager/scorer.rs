use crate::Staff;
use crate::club::board::manager::approach::ManagerApproach;
use crate::club::mind::verdict::MindOption;
use crate::utils::DateUtils;
use crate::{SimulatorData, TeamType};
use chrono::NaiveDate;

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

    /// A memory firm enough to turn down terms the numbers say yes to.
    ///
    /// A manager who was starved at a club for four windows does not
    /// walk back in because the money is 20% better.
    pub const MEMORY_VETO: f32 = 0.35;

    /// And a pull strong enough to take a job the numbers say no to —
    /// unfinished business, or a place he built something.
    pub const MEMORY_OVERRIDE: f32 = 0.55;

    /// Personal-terms acceptance check.
    ///
    /// The numbers first: he accepts if the salary is materially above
    /// his current pay, or the requesting club is materially more
    /// prestigious, or he is ambitious enough to take a smaller step up.
    ///
    /// Then what he remembers about the place. This is the first of the
    /// seven decision sites in `docs/staff_mind.md` §7 to be converted,
    /// and it is deliberately conservative: a manager who holds no
    /// conviction about the club — which is every manager in a fresh
    /// world — produces an empty verdict and gets exactly the old
    /// answer. The baseline is preserved by construction, and the
    /// divergence only grows as careers accumulate, which is what makes
    /// the manager-market census meaningful rather than a re-baseline.
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

        let numbers = salary_uplift || prestige_uplift || (ambition_bonus && req_rep > current_rep);

        // What he remembers about the place, and about the people in
        // the boardroom — judged separately, because a change of
        // chairman is a real reason to look at a club again.
        let verdict = mgr
            .mind
            .deliberate(MindOption::TakeTheJob(approach.requesting_club_id));
        if verdict.is_empty() {
            return numbers;
        }

        let net = verdict.net();
        if numbers {
            net > -Self::MEMORY_VETO
        } else {
            net > Self::MEMORY_OVERRIDE
        }
    }
}
