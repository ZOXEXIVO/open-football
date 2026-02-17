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
}

impl TeamReputation {
    /// Create a new TeamReputation with initial values
    pub fn new(home: u16, national: u16, world: u16) -> Self {
        TeamReputation {
            home: home.min(10000),
            national: national.min(10000),
            world: world.min(10000),
            momentum: ReputationMomentum::default(),
            history: ReputationHistory::new(),
            factors: ReputationFactors::default(),
        }
    }

    /// Get the overall reputation score (weighted average, 0.0-1.0)
    pub fn overall_score(&self) -> f32 {
        (self.home as f32 * 0.2 + self.national as f32 * 0.3 + self.world as f32 * 0.5) / 10000.0
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
            }
        );

        // Decay old factors
        self.factors.decay();
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

    /// Apply gradual reputation decay (called monthly)
    pub fn apply_monthly_decay(&mut self) {
        // Reputation slowly decays without achievements
        let decay_rate = match self.level() {
            ReputationLevel::Elite => 0.995,       // Slower decay for elite teams
            ReputationLevel::Continental => 0.993,
            ReputationLevel::National => 0.990,
            _ => 0.988,                           // Faster decay for lower reputation
        };

        if self.momentum.current < 0.0 {
            // Accelerated decay if momentum is negative
            let adjusted_decay = decay_rate - (self.momentum.current.abs() * 0.01);
            self.apply_decay(adjusted_decay.max(0.95));
        } else if self.momentum.current < 0.1 {
            // Normal decay if momentum is low
            self.apply_decay(decay_rate);
        }
        // No decay if momentum is high (team is performing well)
    }

    /// Calculate reputation factor from match results
    fn calculate_match_factor(&self, results: &[MatchResultInfo]) -> f32 {
        if results.is_empty() {
            return 0.0;
        }

        let mut factor = 0.0;

        for result in results {
            let result_value = match result.outcome {
                MatchOutcome::Win => {
                    let opponent_factor = result.opponent_reputation as f32 / self.overall_score().max(100.0);
                    0.03 * opponent_factor
                }
                MatchOutcome::Draw => {
                    let opponent_factor = result.opponent_reputation as f32 / self.overall_score().max(100.0);
                    0.01 * opponent_factor - 0.005
                }
                MatchOutcome::Loss => {
                    let opponent_factor = self.overall_score() / result.opponent_reputation as f32;
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
            _ => -0.02,               // Bottom 25%
        }
    }

    /// Apply calculated reputation changes
    fn apply_reputation_changes(&mut self, match_factor: f32, position_factor: f32) {
        let total_factor = (match_factor + position_factor) * (1.0 + self.momentum.current);

        // Different scopes affected differently
        let home_change = (total_factor * 200.0) as i16;
        let national_change = (total_factor * 150.0) as i16;
        let world_change = (total_factor * 100.0) as i16;

        self.home = ((self.home as i16 + home_change).max(0) as u16).min(10000);
        self.national = ((self.national as i16 + national_change).max(0) as u16).min(10000);
        self.world = ((self.world as i16 + world_change).max(0) as u16).min(10000);

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
        self.home >= requirements.min_home &&
            self.national >= requirements.min_national &&
            self.world >= requirements.min_world
    }

    /// Get attractiveness factor for transfers/signings
    pub fn attractiveness_factor(&self) -> f32 {
        let base = self.overall_score();
        let momentum_bonus = self.momentum.current.max(0.0) * 0.2;
        let achievement_bonus = (self.factors.achievements.len() as f32 * 0.02).min(0.2);

        (base + momentum_bonus + achievement_bonus).min(1.0)
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

        let recent_avg = self.snapshots.iter()
            .rev()
            .take(4)
            .map(|(_, s)| s.overall)
            .sum::<f32>() / 4.0;

        let older_avg = self.snapshots.iter()
            .rev()
            .skip(4)
            .take(4)
            .map(|(_, s)| s.overall)
            .sum::<f32>() / 4.0;

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
    fn decay(&mut self) {
        // Remove old achievements
        self.achievements.retain(|a| !a.is_expired());

        // Decay other factors
        if self.star_players_signed > 0 {
            self.star_players_signed -= 1;
        }

        self.recent_investment *= 0.95;
    }
}

/// Reputation level categories
#[derive(Debug, Clone, PartialEq)]
pub enum ReputationLevel {
    Amateur,
    Local,
    Regional,
    National,
    Continental,
    Elite,
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

    fn is_expired(&self) -> bool {
        // Achievements expire after 2 years
        let today = chrono::Local::now().date_naive();
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
            8
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

        rep.process_weekly_update(&results, 3, 20, NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());

        assert!(rep.momentum.current > 0.0);
    }

    #[test]
    fn test_attractiveness_factor() {
        let mut rep = TeamReputation::new(8000, 8000, 8000);
        let base_attractiveness = rep.attractiveness_factor();

        // Add achievement
        rep.process_achievement(Achievement::new(
            AchievementType::ContinentalTrophy,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            10
        ));

        let new_attractiveness = rep.attractiveness_factor();
        assert!(new_attractiveness > base_attractiveness);
    }

    #[test]
    fn test_reputation_decay() {
        let mut rep = TeamReputation::new(5000, 5000, 5000);
        let initial_home = rep.home;

        rep.apply_monthly_decay();

        assert!(rep.home < initial_home);
    }
}