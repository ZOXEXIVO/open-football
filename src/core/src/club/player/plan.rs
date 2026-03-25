use chrono::NaiveDate;

/// The club's strategic intent for a signed player.
///
/// When a club buys a player, they have a plan: compete for a starting spot,
/// develop for the future, provide depth. This plan protects the player from
/// being dumped before the club has properly evaluated them — a player must
/// play enough games AND spend enough time before the club can decide they
/// don't fit.
#[derive(Debug, Clone)]
pub struct PlayerPlan {
    /// What role the club envisioned when signing this player.
    pub role: PlayerPlanRole,
    /// When the plan started (transfer date).
    pub started: NaiveDate,
    /// Minimum appearances before the club can fairly judge the player.
    pub min_games: u8,
    /// Months from `started` before the evaluation period ends.
    pub evaluation_months: u8,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlayerPlanRole {
    /// Signed to be a first-team starter right away (experienced, high fee).
    ImmediateStarter,
    /// Signed to compete for a starting spot — needs integration time.
    CompeteForStarting,
    /// Backup / rotation signing — depth for the squad.
    DepthRotation,
    /// Young player signed for long-term development.
    Development,
}

impl PlayerPlan {
    /// Create a plan based on who the player is and what the club paid.
    ///
    /// Real clubs decide the role based on fee, age, and ability:
    /// - A 19yo for 8M → development project, give 2 years
    /// - A 26yo for 20M → compete for starting spot, give 1 year
    /// - A 31yo for 5M → experienced depth, evaluate in 6 months
    /// - A free agent → short trial period
    pub fn from_signing(age: u8, fee: f64, date: NaiveDate) -> Self {
        let (role, min_games, evaluation_months) = if age <= 21 {
            // Young player: long development runway
            (PlayerPlanRole::Development, 10, 18)
        } else if age <= 23 && fee > 0.0 {
            // Young-ish paid signing: still developing but expected to contribute
            (PlayerPlanRole::CompeteForStarting, 12, 12)
        } else if age <= 29 && fee > 0.0 {
            // Prime age paid signing: expected to compete for the team
            (PlayerPlanRole::CompeteForStarting, 15, 12)
        } else if age >= 30 && fee > 0.0 {
            // Experienced paid signing: should contribute quickly
            (PlayerPlanRole::ImmediateStarter, 10, 6)
        } else {
            // Free transfer / low investment: shorter evaluation
            (PlayerPlanRole::DepthRotation, 5, 6)
        };

        PlayerPlan {
            role,
            started: date,
            min_games,
            evaluation_months,
        }
    }

    /// Has the plan's evaluation period concluded?
    ///
    /// A plan is "evaluated" only when BOTH conditions are met:
    /// 1. Enough time has passed (the club gave the player a fair window)
    /// 2. The player had enough appearances (they got a real chance)
    ///
    /// If either condition isn't met, the plan is still active and the player
    /// should not be listed for sale.
    pub fn is_evaluated(&self, current_date: NaiveDate, appearances: u16) -> bool {
        let months_elapsed = (current_date - self.started).num_days() / 30;
        let time_served = months_elapsed >= self.evaluation_months as i64;
        let games_played = appearances >= self.min_games as u16;

        time_served && games_played
    }

    /// Has enough time passed, regardless of appearances?
    /// Used as a fallback — even if a player never played, after a very long
    /// time the club should be allowed to move on.
    pub fn is_expired(&self, current_date: NaiveDate) -> bool {
        let months_elapsed = (current_date - self.started).num_days() / 30;
        // Double the evaluation period as absolute maximum
        months_elapsed >= (self.evaluation_months as i64) * 2
    }
}
