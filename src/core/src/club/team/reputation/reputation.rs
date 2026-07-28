use chrono::NaiveDate;
use std::collections::VecDeque;

/// Enhanced TeamReputation with dynamic updates and history tracking
#[derive(Debug, Clone)]
pub struct TeamReputation {
    /// Local/regional reputation (0-10000)
    pub home: u16,
    /// National reputation (0-10000)
    pub national: u16,
    /// International reputation (0-10000)
    pub world: u16,

    /// Momentum - how fast reputation is changing
    momentum: ReputationMomentum,

    /// Historical tracking
    history: ReputationHistory,

    /// Factors affecting reputation
    factors: ReputationFactors,

    /// Slow-moving structural standing the club reverts toward.
    ///
    /// Reputation used to be a pure ratchet: monthly decay applied to every
    /// club that wasn't actively surging, the decay rate got *faster* at
    /// each tier it dropped through, and the per-week gains were truncated
    /// to integers so anything below +0.01 of total factor rounded to zero.
    /// A club that merely finished 5th every year therefore bled reputation
    /// forever with no restoring force. The baseline is that force: it is a
    /// club's institutional standing — history, stadium, support — which
    /// moves over decades, not seasons.
    baseline: ReputationBaseline,

    /// Sub-unit reputation change carried between weekly updates so small
    /// but genuine gains accumulate instead of truncating to zero.
    residue: ReputationResidue,
}

impl TeamReputation {
    /// Create a new TeamReputation with initial values
    pub fn new(home: u16, national: u16, world: u16) -> Self {
        let home = home.min(10000);
        let national = national.min(10000);
        let world = world.min(10000);
        TeamReputation {
            home,
            national,
            world,
            momentum: ReputationMomentum::default(),
            history: ReputationHistory::new(),
            factors: ReputationFactors::default(),
            baseline: ReputationBaseline {
                home,
                national,
                world,
            },
            residue: ReputationResidue::default(),
        }
    }

    /// Get the overall reputation score (weighted average, 0.0-1.0)
    pub fn overall_score(&self) -> f32 {
        (self.home as f32 * 0.2 + self.national as f32 * 0.3 + self.world as f32 * 0.5) / 10000.0
    }

    /// Blended 0..10000 reputation score for market valuation. Same
    /// weighting as `overall_score` (home 20%, national 30%, world 50%) —
    /// world reputation drives the international market most, but a club
    /// can carry strong domestic standing even with limited continental
    /// exposure. Single source of truth for every transfer/valuation call
    /// site so they stop reading just `world` and flattening domestic
    /// strength.
    pub fn market_value_score(&self) -> u16 {
        let blended = self.home as f32 * 0.2 + self.national as f32 * 0.3 + self.world as f32 * 0.5;
        blended.round().clamp(0.0, 10000.0) as u16
    }

    /// Get reputation level category
    pub fn level(&self) -> ReputationLevel {
        match self.overall_score() {
            s if s >= 0.8 => ReputationLevel::Elite,
            s if s >= 0.65 => ReputationLevel::Continental,
            s if s >= 0.5 => ReputationLevel::National,
            s if s >= 0.3 => ReputationLevel::Regional,
            s if s >= 0.15 => ReputationLevel::Local,
            _ => ReputationLevel::Amateur,
        }
    }

    /// Process weekly reputation update based on recent performance
    pub fn process_weekly_update(
        &mut self,
        match_results: &[MatchResultInfo],
        league_position: u8,
        total_teams: u8,
        date: NaiveDate,
    ) {
        // Calculate performance factors
        let match_factor = self.calculate_match_factor(match_results);
        let position_factor = self.calculate_position_factor(league_position, total_teams);

        // Update momentum based on recent results
        self.momentum.update(match_factor + position_factor);

        // Apply reputation changes
        self.apply_reputation_changes(match_factor, position_factor);

        // Record in history
        self.history.record_snapshot(
            date,
            ReputationSnapshot {
                home: self.home,
                national: self.national,
                world: self.world,
                overall: self.overall_score(),
            },
        );

        // Decay old factors
        self.factors.decay(date);
    }

    /// Process a major achievement (trophy, promotion, etc.)
    pub fn process_achievement(&mut self, achievement: Achievement) {
        let boost = achievement.reputation_boost();

        match achievement.scope() {
            AchievementScope::Local => {
                self.home = (self.home + boost.0).min(10000);
                self.national = (self.national + boost.1 / 2).min(10000);
            }
            AchievementScope::National => {
                self.home = (self.home + boost.0).min(10000);
                self.national = (self.national + boost.1).min(10000);
                self.world = (self.world + boost.2 / 2).min(10000);
            }
            AchievementScope::Continental => {
                self.national = (self.national + boost.1).min(10000);
                self.world = (self.world + boost.2).min(10000);
            }
            AchievementScope::Global => {
                self.world = (self.world + boost.2).min(10000);
                self.national = (self.national + boost.1).min(10000);
            }
        }

        self.factors.achievements.push(achievement);
        self.momentum.boost(0.2);
    }

    /// Process a player signing that affects reputation
    pub fn process_player_signing(&mut self, player_reputation: u16, is_star_player: bool) {
        if is_star_player && player_reputation > self.world {
            // Signing a star player boosts reputation
            let boost = ((player_reputation - self.world) / 10) as u16;
            self.world = (self.world + boost).min(10000);
            self.national = (self.national + boost / 2).min(10000);

            self.factors.star_players_signed += 1;
            self.momentum.boost(0.05);
        }
    }

    /// Process manager change
    pub fn process_manager_change(&mut self, manager_reputation: u16) {
        if manager_reputation > self.national {
            let boost = ((manager_reputation - self.national) / 8) as u16;
            self.national = (self.national + boost).min(10000);
            self.world = (self.world + boost / 2).min(10000);

            self.momentum.boost(0.1);
        }
    }

    /// Monthly drift: decay for clubs that are genuinely going backwards,
    /// and reversion toward the club's structural baseline for everyone.
    ///
    /// Two faults made the old version a one-way ratchet. It decayed any
    /// club whose momentum sat below +0.1 — which includes a side winning
    /// its league comfortably, since momentum for a dominant team settles
    /// around +0.04 — and it decayed *faster* at each lower tier, so every
    /// step down accelerated the next. Decay now requires actually negative
    /// momentum, and the baseline pull below gives a club something to fall
    /// back toward instead of toward zero.
    pub fn apply_monthly_decay(&mut self) {
        if self.momentum.current < 0.0 {
            // Sustained poor form erodes standing. The rate no longer
            // steepens as the club falls; a slide should not self-amplify.
            let decay_rate = 0.994 - (self.momentum.current.abs() * 0.008);
            self.apply_decay(decay_rate.max(0.985));
        }

        self.revert_toward_baseline();
    }

    /// Pull reputation toward the structural baseline, and drag the
    /// baseline slowly toward sustained reality.
    ///
    /// The asymmetry is deliberate and is what makes the system stable:
    /// current reputation moves toward the baseline roughly twenty times
    /// faster than the baseline moves toward current reputation. A club has
    /// to be genuinely, durably diminished for a decade before its
    /// institutional standing follows — which is how football actually
    /// behaves. A club can still rise or fall; it just can't free-fall.
    fn revert_toward_baseline(&mut self) {
        const PULL: f32 = 0.020;
        const BASELINE_DRIFT: f32 = 0.001;

        self.home = Self::lerp_u16(self.home, self.baseline.home, PULL);
        self.national = Self::lerp_u16(self.national, self.baseline.national, PULL);
        self.world = Self::lerp_u16(self.world, self.baseline.world, PULL);

        self.baseline.home = Self::lerp_u16(self.baseline.home, self.home, BASELINE_DRIFT);
        self.baseline.national =
            Self::lerp_u16(self.baseline.national, self.national, BASELINE_DRIFT);
        self.baseline.world = Self::lerp_u16(self.baseline.world, self.world, BASELINE_DRIFT);
    }

    /// Move `from` a fraction `t` of the way to `to`, rounding away from
    /// zero so a small gap still closes instead of stalling on truncation.
    fn lerp_u16(from: u16, to: u16, t: f32) -> u16 {
        let delta = (to as f32 - from as f32) * t;
        if delta.abs() < 1.0 {
            // Below one unit the integer step would round to zero and the
            // club would sit fractionally off its baseline forever.
            return if delta > 0.05 {
                (from + 1).min(10000)
            } else if delta < -0.05 {
                from.saturating_sub(1)
            } else {
                from
            };
        }
        ((from as f32 + delta).round().clamp(0.0, 10000.0)) as u16
    }

    /// The club's structural standing — the level it reverts toward.
    pub fn baseline_score(&self) -> f32 {
        (self.baseline.home as f32 * 0.2
            + self.baseline.national as f32 * 0.3
            + self.baseline.world as f32 * 0.5)
            / 10000.0
    }

    /// Calculate reputation factor from match results.
    ///
    /// Both reputations are compared on the same 0..1 scale. The old code
    /// divided the opponent's 0..10000 reputation by `overall_score()`
    /// (0..1, so the `.max(100.0)` floor always won): wins and draws paid
    /// 20-90× the intended factor while losses cost ~nothing, so every
    /// club's reputation ratcheted up to Elite within a couple of seasons
    /// — and with it every `reputation.level()`-keyed income base (TV,
    /// merchandising, overheads), inflating club balances world-wide.
    fn calculate_match_factor(&self, results: &[MatchResultInfo]) -> f32 {
        if results.is_empty() {
            return 0.0;
        }

        // Floors keep the ratio sane for reputation-less amateur sides;
        // the clamp stops one giant-killing from rewriting a season.
        let self_score = self.overall_score().max(0.05);

        let mut factor = 0.0;

        for result in results {
            let opponent_score = (result.opponent_reputation as f32 / 10_000.0).max(0.05);
            // The two clamps used to be wildly asymmetric: a win over a
            // weaker side was scaled *down* toward 0.25x while a loss to one
            // was scaled *up* to 4x. A strong club in a league it mostly
            // outclasses therefore lost reputation on an ordinary season —
            // it needed a ~60% win rate merely to break even. Both
            // directions now share the same bounded band.
            let result_value = match result.outcome {
                MatchOutcome::Win => {
                    // Beating a stronger side pays more.
                    let opponent_factor = (opponent_score / self_score).clamp(0.5, 2.5);
                    0.03 * opponent_factor
                }
                MatchOutcome::Draw => {
                    // Draw vs a stronger side: slight gain; vs a weaker
                    // side: slight loss.
                    let opponent_factor = (opponent_score / self_score).clamp(0.5, 2.5);
                    0.01 * opponent_factor - 0.005
                }
                MatchOutcome::Loss => {
                    // Losing to a weaker side costs more.
                    let opponent_factor = (self_score / opponent_score).clamp(0.5, 2.5);
                    -0.02 * opponent_factor
                }
            };

            // Competition importance multiplier
            let competition_mult = match result.competition_type {
                CompetitionType::League => 1.0,
                CompetitionType::DomesticCup => 1.2,
                CompetitionType::ContinentalCup => 1.5,
                CompetitionType::WorldCup => 2.0,
            };

            factor += result_value * competition_mult;
        }

        factor / results.len() as f32
    }

    /// Calculate reputation factor from league position
    fn calculate_position_factor(&self, position: u8, total_teams: u8) -> f32 {
        let relative_position = position as f32 / total_teams as f32;

        match relative_position {
            p if p <= 0.1 => 0.03,   // Top 10%
            p if p <= 0.25 => 0.01,  // Top 25%
            p if p <= 0.5 => 0.0,    // Top 50%
            p if p <= 0.75 => -0.01, // Bottom 50%
            _ => -0.02,              // Bottom 25%
        }
    }

    /// Apply calculated reputation changes.
    ///
    /// Changes are accumulated as floats and only the whole-unit part is
    /// spent each week, with the remainder carried. The old version cast
    /// straight to `i16`: a club with a total factor of 0.005 produced a
    /// world change of `0.5 as i16` = 0, so a solid-but-unspectacular side
    /// gained *nothing* week after week while multiplicative decay took
    /// away continuously. That truncation, not any modelled decline, is why
    /// established clubs bled out of the top tier.
    fn apply_reputation_changes(&mut self, match_factor: f32, position_factor: f32) {
        let total_factor = (match_factor + position_factor) * (1.0 + self.momentum.current);

        // Different scopes affected differently
        let (home_change, national_change, world_change) = self.residue.take(
            total_factor * 200.0,
            total_factor * 150.0,
            total_factor * 100.0,
        );

        self.home = ((self.home as i32 + home_change as i32).max(0) as u16).min(10000);
        self.national = ((self.national as i32 + national_change as i32).max(0) as u16).min(10000);
        self.world = ((self.world as i32 + world_change as i32).max(0) as u16).min(10000);

        // Ensure logical ordering (world <= national <= home)
        if self.world > self.national {
            self.national = self.world;
        }
        if self.national > self.home {
            self.home = self.national;
        }
    }

    /// Apply decay to all reputation values
    fn apply_decay(&mut self, rate: f32) {
        self.home = (self.home as f32 * rate) as u16;
        self.national = (self.national as f32 * rate) as u16;
        self.world = (self.world as f32 * rate) as u16;
    }

    /// Get recent trend
    pub fn get_trend(&self) -> ReputationTrend {
        self.history.calculate_trend()
    }

    /// Check if reputation meets requirements
    pub fn meets_requirements(&self, requirements: &ReputationRequirements) -> bool {
        self.home >= requirements.min_home
            && self.national >= requirements.min_national
            && self.world >= requirements.min_world
    }

    /// Get attractiveness factor for transfers/signings
    pub fn attractiveness_factor(&self) -> f32 {
        let base = self.overall_score();
        let momentum_bonus = self.momentum.current.max(0.0) * 0.2;
        let achievement_bonus = (self.factors.achievements.len() as f32 * 0.02).min(0.2);

        (base + momentum_bonus + achievement_bonus).min(1.0)
    }
}

/// A club's slow-moving structural standing — what it reverts toward.
#[derive(Debug, Clone, Copy)]
struct ReputationBaseline {
    home: u16,
    national: u16,
    world: u16,
}

/// Fractional reputation change carried between weekly updates.
///
/// Without this, every change smaller than one unit was silently discarded
/// by the cast to an integer, so gains had a hard floor at zero while decay
/// — being multiplicative — had none.
#[derive(Debug, Clone, Copy, Default)]
struct ReputationResidue {
    home: f32,
    national: f32,
    world: f32,
}

impl ReputationResidue {
    /// Bank the incoming fractional changes and return the whole units to
    /// spend now, keeping the remainder for next week.
    fn take(&mut self, home: f32, national: f32, world: f32) -> (i16, i16, i16) {
        self.home += home;
        self.national += national;
        self.world += world;

        let home_units = self.home.trunc();
        let national_units = self.national.trunc();
        let world_units = self.world.trunc();

        self.home -= home_units;
        self.national -= national_units;
        self.world -= world_units;

        (home_units as i16, national_units as i16, world_units as i16)
    }
}

/// Reputation momentum tracking
#[derive(Debug, Clone)]
struct ReputationMomentum {
    current: f32,
    history: VecDeque<f32>,
}

impl Default for ReputationMomentum {
    fn default() -> Self {
        ReputationMomentum {
            current: 0.0,
            history: VecDeque::with_capacity(10),
        }
    }
}

impl ReputationMomentum {
    fn update(&mut self, change: f32) {
        self.history.push_back(change);
        if self.history.len() > 10 {
            self.history.pop_front();
        }

        // Calculate weighted average (recent changes matter more)
        let mut weighted_sum = 0.0;
        let mut weight_total = 0.0;

        for (i, &value) in self.history.iter().enumerate() {
            let weight = (i + 1) as f32;
            weighted_sum += value * weight;
            weight_total += weight;
        }

        self.current = if weight_total > 0.0 {
            (weighted_sum / weight_total).clamp(-0.5, 0.5)
        } else {
            0.0
        };
    }

    fn boost(&mut self, amount: f32) {
        self.current = (self.current + amount).min(0.5);
    }
}

/// Historical reputation tracking
#[derive(Debug, Clone)]
struct ReputationHistory {
    snapshots: VecDeque<(NaiveDate, ReputationSnapshot)>,
    max_snapshots: usize,
}

impl ReputationHistory {
    fn new() -> Self {
        ReputationHistory {
            snapshots: VecDeque::with_capacity(52), // Store ~1 year of weekly snapshots
            max_snapshots: 52,
        }
    }

    fn record_snapshot(&mut self, date: NaiveDate, snapshot: ReputationSnapshot) {
        self.snapshots.push_back((date, snapshot));
        if self.snapshots.len() > self.max_snapshots {
            self.snapshots.pop_front();
        }
    }

    fn calculate_trend(&self) -> ReputationTrend {
        if self.snapshots.len() < 4 {
            return ReputationTrend::Stable;
        }

        let recent_avg = self
            .snapshots
            .iter()
            .rev()
            .take(4)
            .map(|(_, s)| s.overall)
            .sum::<f32>()
            / 4.0;

        let older_avg = self
            .snapshots
            .iter()
            .rev()
            .skip(4)
            .take(4)
            .map(|(_, s)| s.overall)
            .sum::<f32>()
            / 4.0;

        let change = recent_avg - older_avg;

        match change {
            c if c > 0.05 => ReputationTrend::Rising,
            c if c < -0.05 => ReputationTrend::Falling,
            _ => ReputationTrend::Stable,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct ReputationSnapshot {
    home: u16,
    national: u16,
    world: u16,
    overall: f32,
}

/// Factors affecting reputation
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
struct ReputationFactors {
    achievements: Vec<Achievement>,
    star_players_signed: u8,
    recent_investment: f64,
    stadium_upgrade: bool,
    youth_development: u8,
}

impl ReputationFactors {
    fn decay(&mut self, today: NaiveDate) {
        // Remove old achievements
        self.achievements.retain(|a| !a.is_expired(today));

        // Decay other factors
        if self.star_players_signed > 0 {
            self.star_players_signed -= 1;
        }

        self.recent_investment *= 0.95;
    }
}

/// Reputation level categories. Declared low → high so the derived `Ord`
/// ranks `Amateur < Local < … < Elite` — callers compare tiers directly
/// (e.g. a borrower below a loanee's parent tier).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReputationLevel {
    Amateur,
    Local,
    Regional,
    National,
    Continental,
    Elite,
}

impl ReputationLevel {
    /// True if this club tier actively cycles aging players out of the
    /// squad. Elite and Continental clubs chase progress and can't afford
    /// to carry past-prime squad-average players; everyone below that
    /// keeps loyal veterans through the end of their careers.
    pub fn cycles_aging_squad(&self) -> bool {
        matches!(self, ReputationLevel::Elite | ReputationLevel::Continental)
    }

    /// Wealth-scaled "well below squad average" gap: a player whose current
    /// ability sits more than this far under the main-squad average reads
    /// as surplus at this tier. Richer clubs tolerate a bigger spread (they
    /// can afford depth); small clubs shed quickly. Consulted by the
    /// country listing sweep when deciding who to sell — and by the
    /// transfer pipeline's squad-fit gate when deciding who to buy, so a
    /// club never signs a player its own listing maths would immediately
    /// declare surplus.
    pub fn surplus_quality_gap(&self) -> i16 {
        match self {
            ReputationLevel::Elite => 25,
            ReputationLevel::Continental => 20,
            ReputationLevel::National => 15,
            ReputationLevel::Regional => 12,
            _ => 10,
        }
    }

    /// The next tier down, saturating at `Amateur`. Drives the staged
    /// loan-availability broadcast: a parent club offers a listed player
    /// to the highest realistic tier first, then widens the net one rung
    /// at a time each time the offer goes unanswered.
    pub fn next_lower(self) -> ReputationLevel {
        match self {
            ReputationLevel::Elite => ReputationLevel::Continental,
            ReputationLevel::Continental => ReputationLevel::National,
            ReputationLevel::National => ReputationLevel::Regional,
            ReputationLevel::Regional => ReputationLevel::Local,
            ReputationLevel::Local => ReputationLevel::Amateur,
            ReputationLevel::Amateur => ReputationLevel::Amateur,
        }
    }

    /// `next_lower` applied `steps` times, saturating at `Amateur`. Lets a
    /// broadcast compute its current tier reach directly from how long a
    /// player has been listed, rather than accumulating one rung per weekly
    /// tick (which lagged the reach behind the true listing age).
    pub fn step_down(self, steps: u32) -> ReputationLevel {
        let mut tier = self;
        for _ in 0..steps {
            let lower = tier.next_lower();
            if lower == tier {
                break;
            }
            tier = lower;
        }
        tier
    }

    /// True when a club of this tier runs the seller-side loan broadcast:
    /// resource-rich clubs (National and above) actively place their listed
    /// loanees instead of waiting to be scanned. Shared by the broadcast's
    /// eligibility gate and the borrower scan — which defers to the broadcast
    /// for such a club's development loanees rather than letting a lower club
    /// snatch the prospect before the parent picks the best home for him.
    pub fn runs_loan_broadcast(self) -> bool {
        matches!(
            self,
            ReputationLevel::National | ReputationLevel::Continental | ReputationLevel::Elite
        )
    }
}

/// Reputation trend
#[derive(Debug, Clone, PartialEq)]
pub enum ReputationTrend {
    Rising,
    Stable,
    Falling,
}

/// Achievement that affects reputation
#[derive(Debug, Clone)]
pub struct Achievement {
    achievement_type: AchievementType,
    date: NaiveDate,
    #[allow(dead_code)]
    importance: u8, // 1-10 scale
}

impl Achievement {
    pub fn new(achievement_type: AchievementType, date: NaiveDate, importance: u8) -> Self {
        Achievement {
            achievement_type,
            date,
            importance: importance.min(10),
        }
    }

    fn reputation_boost(&self) -> (u16, u16, u16) {
        match self.achievement_type {
            AchievementType::LeagueTitle => (500, 1000, 800),
            AchievementType::CupWin => (400, 600, 400),
            AchievementType::Promotion => (600, 400, 200),
            AchievementType::ContinentalQualification => (300, 500, 600),
            AchievementType::ContinentalTrophy => (400, 800, 1500),
            AchievementType::RecordBreaking => (200, 300, 400),
        }
    }

    fn scope(&self) -> AchievementScope {
        match self.achievement_type {
            AchievementType::Promotion | AchievementType::RecordBreaking => AchievementScope::Local,
            AchievementType::LeagueTitle | AchievementType::CupWin => AchievementScope::National,
            AchievementType::ContinentalQualification => AchievementScope::Continental,
            AchievementType::ContinentalTrophy => AchievementScope::Global,
        }
    }

    fn is_expired(&self, today: NaiveDate) -> bool {
        // Achievements expire after 2 years
        (today - self.date).num_days() > 730
    }
}

#[derive(Debug, Clone)]
pub enum AchievementType {
    LeagueTitle,
    CupWin,
    Promotion,
    ContinentalQualification,
    ContinentalTrophy,
    RecordBreaking,
}

#[derive(Debug, Clone)]
enum AchievementScope {
    Local,
    National,
    Continental,
    Global,
}

/// Match result information for reputation calculation
#[derive(Debug, Clone)]
pub struct MatchResultInfo {
    pub outcome: MatchOutcome,
    pub opponent_reputation: u16,
    pub competition_type: CompetitionType,
}

#[derive(Debug, Clone)]
pub enum MatchOutcome {
    Win,
    Draw,
    Loss,
}

#[derive(Debug, Clone)]
pub enum CompetitionType {
    League,
    DomesticCup,
    ContinentalCup,
    WorldCup,
}

/// Requirements for reputation-gated content
#[derive(Debug, Clone)]
pub struct ReputationRequirements {
    pub min_home: u16,
    pub min_national: u16,
    pub min_world: u16,
}

impl ReputationRequirements {
    pub fn new(home: u16, national: u16, world: u16) -> Self {
        ReputationRequirements {
            min_home: home,
            min_national: national,
            min_world: world,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reputation_levels() {
        let mut rep = TeamReputation::new(1000, 1000, 1000);
        assert_eq!(rep.level(), ReputationLevel::Amateur);

        rep = TeamReputation::new(3500, 3500, 3500);
        assert_eq!(rep.level(), ReputationLevel::Regional);

        rep = TeamReputation::new(8500, 8500, 8500);
        assert_eq!(rep.level(), ReputationLevel::Elite);
    }

    #[test]
    fn test_achievement_processing() {
        let mut rep = TeamReputation::new(4000, 4000, 4000);
        let initial_world = rep.world;

        let achievement = Achievement::new(
            AchievementType::LeagueTitle,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            8,
        );

        rep.process_achievement(achievement);

        assert!(rep.national > 4000);
        assert!(rep.world > initial_world);
    }

    #[test]
    fn test_match_results_processing() {
        let mut rep = TeamReputation::new(5000, 5000, 5000);

        let results = vec![
            MatchResultInfo {
                outcome: MatchOutcome::Win,
                opponent_reputation: 6000,
                competition_type: CompetitionType::League,
            },
            MatchResultInfo {
                outcome: MatchOutcome::Win,
                opponent_reputation: 7000,
                competition_type: CompetitionType::DomesticCup,
            },
        ];

        rep.process_weekly_update(
            &results,
            3,
            20,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        );

        assert!(rep.momentum.current > 0.0);
    }

    #[test]
    fn losses_actually_cost_reputation() {
        // Regression: the old match factor divided a 0..10000 opponent
        // reputation by the 0..1 overall score (always floored to 100), so
        // losses cost ~nothing and every club ratcheted up to Elite.
        let mut rep = TeamReputation::new(5000, 5000, 5000);
        let start = rep.national;

        for week in 1..=10u32 {
            let results = vec![MatchResultInfo {
                outcome: MatchOutcome::Loss,
                opponent_reputation: 3000,
                competition_type: CompetitionType::League,
            }];
            rep.process_weekly_update(
                &results,
                18,
                20,
                NaiveDate::from_ymd_opt(2024, 1, 1)
                    .unwrap()
                    .checked_add_signed(chrono::Duration::days(week as i64 * 7))
                    .unwrap(),
            );
        }

        assert!(
            rep.national < start,
            "10 straight losses near the bottom must lower reputation: {} -> {}",
            start,
            rep.national
        );
    }

    #[test]
    fn mid_table_draws_do_not_inflate_reputation() {
        // A full season of mid-table draws against equal opposition must
        // leave reputation roughly where it started — not +50 per week.
        let mut rep = TeamReputation::new(5000, 5000, 5000);
        let start = rep.home;

        for week in 1..=38u32 {
            let results = vec![MatchResultInfo {
                outcome: MatchOutcome::Draw,
                opponent_reputation: 5000,
                competition_type: CompetitionType::League,
            }];
            rep.process_weekly_update(
                &results,
                10,
                20,
                NaiveDate::from_ymd_opt(2024, 8, 1)
                    .unwrap()
                    .checked_add_signed(chrono::Duration::days(week as i64 * 7))
                    .unwrap(),
            );
        }

        let drift = (rep.home as i32 - start as i32).abs();
        assert!(
            drift < 400,
            "a season of neutral results drifted reputation by {drift} (from {start} to {})",
            rep.home
        );
    }

    #[test]
    fn beating_stronger_side_pays_more_than_beating_weaker() {
        let base = TeamReputation::new(5000, 5000, 5000);
        let date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

        let mut vs_stronger = base.clone();
        vs_stronger.process_weekly_update(
            &[MatchResultInfo {
                outcome: MatchOutcome::Win,
                opponent_reputation: 9000,
                competition_type: CompetitionType::League,
            }],
            10,
            20,
            date,
        );

        let mut vs_weaker = base.clone();
        vs_weaker.process_weekly_update(
            &[MatchResultInfo {
                outcome: MatchOutcome::Win,
                opponent_reputation: 1500,
                competition_type: CompetitionType::League,
            }],
            10,
            20,
            date,
        );

        assert!(vs_stronger.home > vs_weaker.home);
    }

    #[test]
    fn test_attractiveness_factor() {
        let mut rep = TeamReputation::new(8000, 8000, 8000);
        let base_attractiveness = rep.attractiveness_factor();

        // Add achievement
        rep.process_achievement(Achievement::new(
            AchievementType::ContinentalTrophy,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            10,
        ));

        let new_attractiveness = rep.attractiveness_factor();
        assert!(new_attractiveness > base_attractiveness);
    }

    #[test]
    fn test_reputation_decay() {
        // Decay now requires genuinely negative momentum. Drive the club
        // through a run of losses first, then confirm it erodes.
        let mut rep = TeamReputation::new(5000, 5000, 5000);
        for week in 1..=6u32 {
            rep.process_weekly_update(
                &[MatchResultInfo {
                    outcome: MatchOutcome::Loss,
                    opponent_reputation: 3000,
                    competition_type: CompetitionType::League,
                }],
                19,
                20,
                NaiveDate::from_ymd_opt(2024, 1, 1)
                    .unwrap()
                    .checked_add_signed(chrono::Duration::days(week as i64 * 7))
                    .unwrap(),
            );
        }
        assert!(rep.momentum.current < 0.0, "losing run must sour momentum");

        let before = rep.home;
        rep.apply_monthly_decay();
        assert!(rep.home < before);
    }

    #[test]
    fn established_club_holding_its_level_does_not_bleed_out() {
        // The headline regression. A club that finishes comfortably in the
        // top half every season, without winning the league, must not slide
        // out of its reputation tier. Under the old model monthly decay ran
        // regardless of results and per-week gains truncated to zero, so
        // this club lost roughly 6% of its reputation a year, compounding —
        // Elite to National inside a decade, taking 90% of its revenue with
        // it.
        let mut rep = TeamReputation::new(8600, 8500, 8400);
        let start_level = rep.level();
        let start_score = rep.overall_score();
        let mut date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();

        // Ten seasons of 38 weeks: a solid 4th-place side — more wins than
        // losses against a league it mostly outclasses.
        for _season in 0..10 {
            for week in 0..38 {
                let outcome = match week % 5 {
                    0 | 1 | 2 => MatchOutcome::Win,
                    3 => MatchOutcome::Draw,
                    _ => MatchOutcome::Loss,
                };
                rep.process_weekly_update(
                    &[MatchResultInfo {
                        outcome,
                        opponent_reputation: 5200,
                        competition_type: CompetitionType::League,
                    }],
                    4,
                    20,
                    date,
                );
                date += chrono::Duration::days(7);
                if week % 4 == 0 {
                    rep.apply_monthly_decay();
                }
            }
        }

        assert_eq!(
            rep.level(),
            start_level,
            "a consistent top-four club dropped from {:?} to {:?} ({:.3} → {:.3})",
            start_level,
            rep.level(),
            start_score,
            rep.overall_score()
        );
    }

    #[test]
    fn small_weekly_gains_are_not_truncated_away() {
        // A total factor of ~0.005 yields a world change of 0.5 per week,
        // which the old integer cast threw away entirely. Over a season
        // those halves must add up to something.
        let mut rep = TeamReputation::new(5000, 5000, 5000);
        let start = rep.world;
        let mut date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();

        for _ in 0..38 {
            // Draws against marginally stronger opposition: a small but
            // real positive factor.
            rep.process_weekly_update(
                &[MatchResultInfo {
                    outcome: MatchOutcome::Draw,
                    opponent_reputation: 6000,
                    competition_type: CompetitionType::League,
                }],
                9,
                20,
                date,
            );
            date += chrono::Duration::days(7);
        }

        assert!(
            rep.world > start,
            "sub-unit weekly gains vanished: {start} → {}",
            rep.world
        );
    }

    #[test]
    fn reputation_reverts_toward_its_structural_baseline() {
        // A collapse should be recoverable, not terminal: an idle club
        // knocked below its institutional standing drifts back up.
        let mut rep = TeamReputation::new(8000, 8000, 8000);
        rep.home = 5000;
        rep.national = 5000;
        rep.world = 5000;

        for _ in 0..24 {
            rep.apply_monthly_decay();
        }

        assert!(
            rep.world > 5000,
            "reputation did not revert toward baseline: {}",
            rep.world
        );
        assert!(rep.world <= 8000);
    }

    #[test]
    fn baseline_follows_a_genuinely_sustained_decline() {
        // Reversion must not make a club immortal. Two decades of relegation
        // form has to move the institution itself.
        let mut rep = TeamReputation::new(8000, 8000, 8000);
        let start_baseline = rep.baseline_score();
        let mut date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();

        for _ in 0..20 {
            for week in 0..38 {
                rep.process_weekly_update(
                    &[MatchResultInfo {
                        outcome: MatchOutcome::Loss,
                        opponent_reputation: 4000,
                        competition_type: CompetitionType::League,
                    }],
                    20,
                    20,
                    date,
                );
                date += chrono::Duration::days(7);
                if week % 4 == 0 {
                    rep.apply_monthly_decay();
                }
            }
        }

        assert!(
            rep.baseline_score() < start_baseline,
            "twenty years of failure left the baseline untouched"
        );
        assert!(rep.overall_score() < 0.8);
    }
}
