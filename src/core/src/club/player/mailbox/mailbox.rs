use crate::handlers::ProcessContractHandler;
use crate::{Player, PlayerMailboxResult, PlayerResult};
use chrono::NaiveDate;
use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct PlayerMessage {
    pub message_type: PlayerMessageType,
}

#[derive(Debug, Clone)]
pub enum PlayerMessageType {
    ContractProposal(PlayerContractProposal),
}

#[derive(Debug, Clone)]
pub struct PlayerContractProposal {
    pub salary: u32,
    pub years: u8,
    /// Staff negotiation skill (man_management, 0-20). Higher = more persuasive.
    pub negotiation_skill: u8,
    /// One-off signature payment. Opens doors with greedy agents.
    pub signing_bonus: u32,
    /// Yearly loyalty bonus — rewards staying. Real clubs use this on
    /// long-service renewals where the base wage has hit the cap.
    pub loyalty_bonus: u32,
    /// Negotiated release clause. Players take short deals with a release
    /// more readily than long deals without one.
    pub release_clause: Option<u32>,

    // ── Extended package (all optional; None = not offered). ──
    /// Squad role the club is promising. None = keep current.
    pub squad_status_promise: Option<crate::PlayerSquadStatus>,
    pub appearance_fee: Option<u32>,
    pub unused_sub_fee: Option<u32>,
    pub goal_bonus: Option<u32>,
    pub clean_sheet_bonus: Option<u32>,
    pub promotion_bonus: Option<u32>,
    pub avoid_relegation_bonus: Option<u32>,
    pub international_cap_bonus: Option<u32>,
    /// Threshold fee that auto-releases the player on relegation.
    pub relegation_release: Option<u32>,
    /// Threshold fee that auto-releases if the club misses promotion.
    pub non_promotion_release: Option<u32>,
    /// Yearly wage rise applied at every contract anniversary.
    /// Stored as 0-50 (percent).
    pub yearly_wage_rise_pct: Option<u8>,
    pub promotion_wage_increase_pct: Option<u8>,
    pub relegation_wage_decrease_pct: Option<u8>,
    /// Club option to extend the contract by N years.
    pub optional_extension_years: Option<u8>,
    /// Apps threshold above which a one-year extension auto-triggers in
    /// the final season.
    pub appearance_extension_threshold: Option<u16>,
    /// Wage rise after a player crosses an appearances threshold for the
    /// club in league play. Encoded as (appearances_threshold, rise_pct).
    pub wage_after_apps: Option<(u16, u8)>,
    /// Wage rise after international caps cross a threshold.
    pub wage_after_caps: Option<(u16, u8)>,
    /// True if the club promises to match the highest earner. Used very
    /// sparingly — elite players only.
    pub match_highest_earner: bool,

    /// Snapshot of the club + league reputation context the offer was
    /// built against. Stashed so the player can evaluate acceptance
    /// against the SAME elite-club / mid-tier expectations the renewal
    /// AI used — without it, the player and the club disagree on what a
    /// fair wage is and the same offer gets rejected at an elite club
    /// while sailing through at a small one. `None` for legacy callers
    /// that don't supply context (acceptance falls back to neutral 0.5
    /// / 5000 in that case).
    pub valuation_club_reputation: Option<f32>,
    pub valuation_league_reputation: Option<u16>,
    /// The expected wage the renewal AI computed for this player at this
    /// club/league/status. Used by acceptance to detect the
    /// "underpaid star" case without re-running the valuation pass.
    pub valuation_expected_wage: Option<u32>,
    pub valuation_min_acceptable: Option<u32>,
}

impl PlayerContractProposal {
    /// Minimal constructor — all extended package fields default to None.
    /// Existing renewal sites can keep using this to stay
    /// behaviourally identical until they opt in to the wider package.
    pub fn basic(
        salary: u32,
        years: u8,
        negotiation_skill: u8,
        signing_bonus: u32,
        loyalty_bonus: u32,
        release_clause: Option<u32>,
    ) -> Self {
        Self {
            salary,
            years,
            negotiation_skill,
            signing_bonus,
            loyalty_bonus,
            release_clause,
            squad_status_promise: None,
            appearance_fee: None,
            unused_sub_fee: None,
            goal_bonus: None,
            clean_sheet_bonus: None,
            promotion_bonus: None,
            avoid_relegation_bonus: None,
            international_cap_bonus: None,
            relegation_release: None,
            non_promotion_release: None,
            yearly_wage_rise_pct: None,
            promotion_wage_increase_pct: None,
            relegation_wage_decrease_pct: None,
            optional_extension_years: None,
            appearance_extension_threshold: None,
            wage_after_apps: None,
            wage_after_caps: None,
            match_highest_earner: false,
            valuation_club_reputation: None,
            valuation_league_reputation: None,
            valuation_expected_wage: None,
            valuation_min_acceptable: None,
        }
    }
}

/// The player's own side of the negotiation. Stashed on the Player when a
/// proposal is turned down so the next offer from the club can converge on
/// terms the player would actually sign, rather than guessing again.
///
/// `desired_*` fields cover the headline terms; `demanded_*` carry the
/// reason the player walked, so the AI can prioritise the right lever
/// (better release clause vs. better base wage) on the next offer.
#[derive(Debug, Clone)]
pub struct PlayerContractAsk {
    pub desired_salary: u32,
    pub desired_years: u8,
    pub recorded_on: chrono::NaiveDate,
    /// Status the player wants if role was the deal-breaker.
    pub demanded_status: Option<crate::PlayerSquadStatus>,
    /// Fee threshold the player wants on a release clause.
    pub demanded_release_clause: Option<u32>,
    /// Signing/loyalty sweetener the player wants if base wage couldn't budge.
    pub demanded_signing_bonus: Option<u32>,
    /// Why the deal fell over. Renewal AI reads this to pick the right
    /// lever on the next offer instead of just bumping the wage blindly.
    pub rejection_reason: Option<RejectionReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionReason {
    LowSalary,
    ShortContract,
    StatusBelowExpectation,
    NoReleaseClause,
    NoSweetener,
    AmbitionMismatch,
}

#[derive(Debug, Clone)]
pub struct PlayerMailbox {
    messages: VecDeque<PlayerMessage>,
}

impl PlayerMailbox {
    pub fn new() -> Self {
        PlayerMailbox {
            messages: VecDeque::new(),
        }
    }

    pub fn process(
        player: &mut Player,
        player_result: &mut PlayerResult,
        now: NaiveDate,
    ) -> PlayerMailboxResult {
        let result = PlayerMailboxResult::new();

        let messages: Vec<PlayerMessage> = player.mailbox.messages.drain(..).collect();
        for message in messages {
            match message.message_type {
                PlayerMessageType::ContractProposal(proposal) => {
                    ProcessContractHandler::process(player, proposal, now, player_result);
                }
            }
        }

        result
    }

    pub fn push(&mut self, message: PlayerMessage) {
        self.messages.push_back(message);
    }
}
