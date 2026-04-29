//! Free types shared between the various Player::on_* event handlers.
//!
//! Constructed by callers (league/match-result pipeline, transfer
//! pipeline) and handed to the player one outcome at a time.

use chrono::NaiveDate;

use crate::club::PlayerClubContract;
use crate::r#match::PlayerMatchEndStats;
use crate::TeamInfo;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MatchParticipation {
    Starter,
    Substitute,
}

/// Everything the Player needs to react to a finished match. Constructed
/// by the league/match-result pipeline and handed over one player at a
/// time; the Player owns all resulting stat bookkeeping, morale events,
/// and reputation changes.
pub struct MatchOutcome<'a> {
    pub stats: &'a PlayerMatchEndStats,
    pub effective_rating: f32,
    pub participation: MatchParticipation,
    pub is_friendly: bool,
    pub is_cup: bool,
    pub is_motm: bool,
    /// Goals scored by this player's team. Available for everyone on the
    /// matchday squad — used for decisive-goal / score-margin gating.
    pub team_goals_for: u8,
    /// Goals conceded by this player's team. Always populated for matchday
    /// squad members; emit sites apply their own role gates (GK stats,
    /// defender clean-sheet pride, etc.).
    pub team_goals_against: u8,
    pub league_weight: f32,
    pub world_weight: f32,
    /// True when the opposing club is in this player's club's rivals list.
    /// Derby results produce bigger morale swings either way.
    pub is_derby: bool,
    /// Did this player's team win the match? Derby bonus/penalty uses it.
    pub team_won: bool,
    /// Did this player's team lose the match?
    pub team_lost: bool,
}

impl<'a> MatchOutcome<'a> {
    /// Goal margin from this player's team perspective: positive when
    /// won, negative when lost, zero on a draw. Saturates to i8 to keep
    /// freak basket-scores from overflowing.
    #[inline]
    pub fn goal_margin(&self) -> i8 {
        let g = self.team_goals_for as i16 - self.team_goals_against as i16;
        g.clamp(i8::MIN as i16, i8::MAX as i16) as i8
    }
}

pub struct TransferCompletion<'a> {
    pub from: &'a TeamInfo,
    pub to: &'a TeamInfo,
    pub fee: f64,
    pub date: NaiveDate,
    pub selling_club_id: u32,
    pub buying_club_id: u32,
    /// Annual wage agreed during PersonalTerms. None = compute from context.
    pub agreed_wage: Option<u32>,
    /// Buying club's league reputation (0–10000), for fallback wage computation
    /// when `agreed_wage` is absent.
    pub buying_league_reputation: u16,
    /// Sell-on percentage pledged by the buyer to the current seller. Added
    /// to `player.sell_on_obligations` so the next sale pays the seller out.
    pub record_sell_on: Option<f32>,
}

pub struct LoanCompletion<'a> {
    pub from: &'a TeamInfo,
    pub to: &'a TeamInfo,
    pub loan_fee: f64,
    pub date: NaiveDate,
    pub loan_contract: PlayerClubContract,
    pub borrowing_club_id: u32,
}
