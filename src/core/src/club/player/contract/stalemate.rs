use crate::club::player::mailbox::{PlayerContractAsk, RejectionReason};
use crate::{Player, PlayerSquadStatus, PlayerStatusType};
use chrono::NaiveDate;

/// Renewal-history labels emitted by the renewal pipeline. Mirrored here
/// so the assessment is self-contained and the labels can be searched
/// without pulling in the renewal manager.
pub const RENEWAL_OFFERED_LABEL: &str = "dec_contract_renewal_offered";
pub const RENEWAL_REJECTED_LABEL: &str = "dec_contract_renewal_rejected";

/// Rolling window over which renewal offers / rejections are counted.
pub const STALEMATE_WINDOW_DAYS: i64 = 365;

/// A rejection is "recent" — i.e. still actively driving the stalemate —
/// when it happened within this many days. Older rejections still count
/// toward the rolling total but no longer escalate severity on their own.
const RECENT_REJECTION_DAYS: i64 = 120;

/// Below this many days to expiry the club is meaningfully under
/// Bosman / free-agency pressure; combined with rejections it accelerates
/// the stalemate.
const EXPIRY_PRESSURE_DAYS: i64 = 180;

/// Final-month pressure: at this point every unsuccessful talk is the
/// last one before the player walks for free.
const EXPIRY_CRITICAL_DAYS: i64 = 60;

/// A pending ask within this percentage of the player's current salary is
/// considered "manageable" and the next offer should converge on it
/// rather than escalating to a listing.
const REASONABLE_ASK_HEADROOM_PCT: u32 = 25;

/// Rejection-count thresholds that trigger escalation. Three rejections
/// inside a rolling year is the established cap used by
/// `ContractRenewalManager` for proactive attempts, so we reuse it as the
/// "exhausted" threshold here.
const REJECTIONS_FOR_SEVERE: u32 = 2;
const REJECTIONS_FOR_EXHAUSTED: u32 = 3;

/// Severity ladder consumed by `handle_unresolved_salary`, the country
/// listing pipeline, and the transfer-list AI prompt. Each consumer maps
/// it onto its own action (skip, escalate offer, list).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StalemateLevel {
    /// No failed renewal history worth acting on.
    None,
    /// At least one rejection but the negotiation is still alive — keep
    /// offering, do not list.
    Emerging,
    /// Repeat rejections or rejections combined with expiry pressure /
    /// unaffordable demands. Listing is allowed for fringe / surplus
    /// profiles; senior squad players still get one more attempt.
    Severe,
    /// Negotiations are clearly impossible: cap of rejections hit, or
    /// rejections plus critical expiry, or persistent unaffordable
    /// demands. Listing is permitted across the board.
    Exhausted,
}

impl StalemateLevel {
    pub fn as_key(&self) -> &'static str {
        match self {
            StalemateLevel::None => "none",
            StalemateLevel::Emerging => "emerging",
            StalemateLevel::Severe => "severe",
            StalemateLevel::Exhausted => "exhausted",
        }
    }
}

/// Snapshot of contract-renewal state for a single player. Built once by
/// any system that needs to reason about whether to keep negotiating,
/// improve an offer, or escalate to a transfer listing. The fields are
/// the inputs the listing AI prompt also surfaces, so the club AI and
/// the deterministic country pipeline agree on the same numbers.
#[derive(Debug, Clone)]
pub struct ContractStalemate {
    pub offers_12m: u32,
    pub rejections_12m: u32,
    pub last_rejection_days_ago: Option<i64>,
    pub days_to_expiry: Option<i64>,
    pub squad_status: PlayerSquadStatus,
    pub has_market_interest: bool,
    pub is_unrest: bool,
    pub pending_ask: Option<PlayerContractAsk>,
    pub ask_affordable: Option<bool>,
    pub level: StalemateLevel,
}

/// Affordability evidence supplied by the caller. The renewal pipeline
/// and `handle_unresolved_salary` have the wage budget on hand; the
/// country pipeline computes per-club headroom from the season targets.
/// When the headroom isn't known (e.g. unit tests, board not set) the
/// assessment leaves `ask_affordable` as `None` and falls back to the
/// generic rules — it never treats "unknown" as "unaffordable" since
/// that would over-escalate.
#[derive(Debug, Clone, Copy)]
pub struct AffordabilityInput {
    pub wage_budget_headroom: Option<u32>,
    pub current_salary: u32,
}

impl ContractStalemate {
    pub fn assess(player: &Player, today: NaiveDate, affordability: AffordabilityInput) -> Self {
        let cutoff = today - chrono::Duration::days(STALEMATE_WINDOW_DAYS);
        let mut offers = 0u32;
        let mut rejections = 0u32;
        let mut last_reject: Option<NaiveDate> = None;
        for d in &player.decision_history.items {
            if d.date < cutoff {
                continue;
            }
            if d.decision == RENEWAL_OFFERED_LABEL {
                offers += 1;
            } else if d.decision == RENEWAL_REJECTED_LABEL {
                rejections += 1;
                if last_reject.map_or(true, |prev| d.date > prev) {
                    last_reject = Some(d.date);
                }
            }
        }
        let last_rejection_days_ago = last_reject.map(|d| (today - d).num_days());

        let days_to_expiry = player
            .contract
            .as_ref()
            .map(|c| (c.expiration - today).num_days());

        let squad_status = player
            .contract
            .as_ref()
            .map(|c| c.squad_status.clone())
            .unwrap_or(PlayerSquadStatus::FirstTeamRegular);

        let statuses = player.statuses.get();
        let has_market_interest = statuses.iter().any(|s| {
            matches!(
                s,
                PlayerStatusType::Wnt | PlayerStatusType::Enq | PlayerStatusType::Bid
            )
        });
        let is_unrest = statuses
            .iter()
            .any(|s| matches!(s, PlayerStatusType::Req | PlayerStatusType::Unh));

        let pending_ask = player.pending_contract_ask.clone();
        let ask_affordable =
            pending_ask
                .as_ref()
                .and_then(|ask| match affordability.wage_budget_headroom {
                    Some(headroom) => {
                        let delta = ask
                            .desired_salary
                            .saturating_sub(affordability.current_salary);
                        Some(delta <= headroom)
                    }
                    None => None,
                });

        let level = compute_level(
            rejections,
            last_rejection_days_ago,
            days_to_expiry,
            &pending_ask,
            ask_affordable,
            is_unrest,
            &squad_status,
            affordability.current_salary,
        );

        ContractStalemate {
            offers_12m: offers,
            rejections_12m: rejections,
            last_rejection_days_ago,
            days_to_expiry,
            squad_status,
            has_market_interest,
            is_unrest,
            pending_ask,
            ask_affordable,
            level,
        }
    }

    /// Whether the club can list a player at this stalemate level given
    /// their squad status. Listing requires the bar set by the *more*
    /// protected of the two — KeyPlayer needs Exhausted, NotNeeded can
    /// list as soon as the assessment leaves None. Force-selected
    /// players and loaned players still have to be filtered separately
    /// by the caller — that's a structural restriction, not a stalemate
    /// one.
    pub fn permits_listing(&self) -> bool {
        let needed = required_level_for_status(&self.squad_status);
        rank(self.level) >= rank(needed)
    }

    /// True when the next sensible move is "offer again, this time
    /// matching the ask" — i.e. the player has a pending ask, it's
    /// affordable, and the situation hasn't degraded to Exhausted.
    pub fn should_improve_offer(&self) -> bool {
        if matches!(self.level, StalemateLevel::Exhausted) {
            return false;
        }
        matches!(self.ask_affordable, Some(true))
    }

    /// Convenience: did a rejection happen in the last RECENT window?
    pub fn has_recent_rejection(&self) -> bool {
        self.last_rejection_days_ago
            .map(|d| d <= RECENT_REJECTION_DAYS)
            .unwrap_or(false)
    }
}

fn rank(level: StalemateLevel) -> u8 {
    match level {
        StalemateLevel::None => 0,
        StalemateLevel::Emerging => 1,
        StalemateLevel::Severe => 2,
        StalemateLevel::Exhausted => 3,
    }
}

/// Minimum stalemate level required to list a player of this squad
/// status. KeyPlayer / FirstTeamRegular / HotProspect need Exhausted —
/// the club won't give up on a starter after one or two rejections.
/// Fringe profiles can be listed at Severe; surplus (NotNeeded) leaves
/// negotiation as soon as it has visibly failed once.
pub fn required_level_for_status(status: &PlayerSquadStatus) -> StalemateLevel {
    match status {
        PlayerSquadStatus::KeyPlayer
        | PlayerSquadStatus::FirstTeamRegular
        | PlayerSquadStatus::HotProspectForTheFuture => StalemateLevel::Exhausted,
        PlayerSquadStatus::FirstTeamSquadRotation
        | PlayerSquadStatus::MainBackupPlayer
        | PlayerSquadStatus::DecentYoungster => StalemateLevel::Severe,
        PlayerSquadStatus::NotNeeded => StalemateLevel::Emerging,
        _ => StalemateLevel::Severe,
    }
}

fn compute_level(
    rejections: u32,
    last_rejection_days_ago: Option<i64>,
    days_to_expiry: Option<i64>,
    pending_ask: &Option<PlayerContractAsk>,
    ask_affordable: Option<bool>,
    is_unrest: bool,
    squad_status: &PlayerSquadStatus,
    current_salary: u32,
) -> StalemateLevel {
    if rejections == 0 {
        // No rejections — bare expiry pressure alone doesn't constitute
        // a stalemate. ContractRenewalManager will still try.
        return StalemateLevel::None;
    }

    let recent = last_rejection_days_ago
        .map(|d| d <= RECENT_REJECTION_DAYS)
        .unwrap_or(false);

    let near_expiry = days_to_expiry
        .map(|d| d > 0 && d <= EXPIRY_PRESSURE_DAYS)
        .unwrap_or(false);

    let critical_expiry = days_to_expiry
        .map(|d| d > 0 && d <= EXPIRY_CRITICAL_DAYS)
        .unwrap_or(false);

    let ask_clearly_unaffordable = matches!(ask_affordable, Some(false));
    let ask_clearly_affordable = matches!(ask_affordable, Some(true));
    // "Unreasonable" = the player asks for far more than what they're on
    // today. The renewal pipeline can sometimes match this; budget gates
    // it elsewhere. Used as a soft signal for severity when affordability
    // is unknown (no budget supplied).
    let ask_unreasonable = pending_ask
        .as_ref()
        .map(|ask| {
            if current_salary == 0 {
                return false;
            }
            let cap = current_salary
                .saturating_add(current_salary.saturating_mul(REASONABLE_ASK_HEADROOM_PCT) / 100);
            ask.desired_salary > cap.saturating_mul(2)
        })
        .unwrap_or(false);

    let surplus = matches!(squad_status, PlayerSquadStatus::NotNeeded);

    // An affordable, concrete ask means the negotiation is structurally
    // alive: the player has named a number the club can pay. We refuse to
    // call this exhausted purely on rejection count — `should_improve_offer`
    // is the right next move. Only a *concurrent* hard blocker
    // (player demands departure / contract about to expire / open unrest)
    // can still escalate the situation past Severe.
    let affordable_actionable = ask_clearly_affordable && pending_ask.is_some();

    // Exhausted: clear ceiling has been hit. Match-with-cap (3 / yr) is
    // the canonical "the club has tried enough" signal — but only if the
    // negotiation has actually broken down. When the pending ask fits
    // wage headroom we stay at Severe and let the reactive renewal pass
    // converge on the ask instead.
    let count_alone_exhausts = rejections >= REJECTIONS_FOR_EXHAUSTED && !affordable_actionable;
    if count_alone_exhausts
        || (rejections >= REJECTIONS_FOR_SEVERE && critical_expiry && !affordable_actionable)
        || (recent && ask_clearly_unaffordable && is_unrest)
        || (surplus && recent && ask_clearly_unaffordable)
        || (rejections >= REJECTIONS_FOR_EXHAUSTED && is_unrest)
    {
        return StalemateLevel::Exhausted;
    }

    // Severe: more than one rejection, or one rejection with concrete
    // evidence the deal can't be done (expiry pressure, unaffordable ask,
    // unrest, or a wildly out-of-band demand).
    if rejections >= REJECTIONS_FOR_SEVERE
        || (recent && near_expiry)
        || (recent && ask_clearly_unaffordable)
        || (recent && is_unrest)
        || (recent && ask_unreasonable)
        || (surplus && recent)
    {
        return StalemateLevel::Severe;
    }

    StalemateLevel::Emerging
}

/// Translates a `RejectionReason` into a human-readable token for
/// telemetry, prompts, and decision_history. Lower-snake_case so the AI
/// can match on it like the existing reason keys.
pub fn rejection_reason_token(reason: RejectionReason) -> &'static str {
    match reason {
        RejectionReason::LowSalary => "low_salary",
        RejectionReason::ShortContract => "short_contract",
        RejectionReason::StatusBelowExpectation => "status_below_expectation",
        RejectionReason::NoReleaseClause => "no_release_clause",
        RejectionReason::NoSweetener => "no_sweetener",
        RejectionReason::AmbitionMismatch => "ambition_mismatch",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::PlayerSkills;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::club::player::mailbox::{PlayerContractAsk, RejectionReason};
    use crate::club::player::personality::PlayerDecisionHistory;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerClubContract, PlayerPositions, PlayerSquadStatus,
    };
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn make_history(items: &[(NaiveDate, &str)]) -> PlayerDecisionHistory {
        let mut h = PlayerDecisionHistory::new();
        for (date, decision) in items {
            h.add(
                *date,
                "movement".to_string(),
                decision.to_string(),
                "tester".to_string(),
            );
        }
        h
    }

    fn make_contract(
        salary: u32,
        squad_status: PlayerSquadStatus,
        expiration: NaiveDate,
    ) -> PlayerClubContract {
        let mut c = PlayerClubContract::new(salary, expiration);
        c.squad_status = squad_status;
        c
    }

    fn make_player_with(
        decisions: PlayerDecisionHistory,
        contract: Option<PlayerClubContract>,
        pending_ask: Option<PlayerContractAsk>,
    ) -> Player {
        let mut p = PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("Test".into(), "Player".into()))
            .birth_date(d(1995, 1, 1))
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions { positions: vec![] })
            .player_attributes(PlayerAttributes::default())
            .decision_history(decisions)
            .contract(contract)
            .build()
            .unwrap();
        p.pending_contract_ask = pending_ask;
        p
    }

    #[test]
    fn level_no_rejections_is_none() {
        let level = compute_level(
            0,
            None,
            Some(400),
            &None,
            None,
            false,
            &PlayerSquadStatus::FirstTeamRegular,
            10_000,
        );
        assert_eq!(level, StalemateLevel::None);
    }

    #[test]
    fn level_single_rejection_is_emerging() {
        let level = compute_level(
            1,
            Some(10),
            Some(400),
            &None,
            None,
            false,
            &PlayerSquadStatus::FirstTeamRegular,
            10_000,
        );
        assert_eq!(level, StalemateLevel::Emerging);
    }

    #[test]
    fn level_two_rejections_is_severe() {
        let level = compute_level(
            2,
            Some(10),
            Some(400),
            &None,
            None,
            false,
            &PlayerSquadStatus::MainBackupPlayer,
            10_000,
        );
        assert_eq!(level, StalemateLevel::Severe);
    }

    #[test]
    fn level_three_rejections_is_exhausted() {
        let level = compute_level(
            3,
            Some(10),
            Some(400),
            &None,
            None,
            false,
            &PlayerSquadStatus::KeyPlayer,
            10_000,
        );
        assert_eq!(level, StalemateLevel::Exhausted);
    }

    #[test]
    fn three_rejections_with_affordable_ask_stays_severe() {
        // Player has been rejected three times — but the latest pending
        // ask is one the club can pay. Negotiation is still alive; the
        // reactive renewal pass will take the converge-on-ask path.
        let ask = PlayerContractAsk {
            desired_salary: 60_000,
            desired_years: 3,
            recorded_on: d(2026, 4, 1),
            demanded_status: None,
            demanded_release_clause: None,
            demanded_signing_bonus: None,
            rejection_reason: Some(RejectionReason::LowSalary),
        };
        let level = compute_level(
            3,
            Some(10),
            Some(400),
            &Some(ask),
            Some(true),
            false,
            &PlayerSquadStatus::FirstTeamRegular,
            50_000,
        );
        assert_eq!(
            level,
            StalemateLevel::Severe,
            "affordable pending ask must not escalate to Exhausted on rejection count alone"
        );
    }

    #[test]
    fn three_rejections_with_unaffordable_ask_is_exhausted() {
        let ask = PlayerContractAsk {
            desired_salary: 500_000,
            desired_years: 4,
            recorded_on: d(2026, 4, 1),
            demanded_status: None,
            demanded_release_clause: None,
            demanded_signing_bonus: None,
            rejection_reason: Some(RejectionReason::LowSalary),
        };
        let level = compute_level(
            3,
            Some(10),
            Some(400),
            &Some(ask),
            Some(false),
            false,
            &PlayerSquadStatus::FirstTeamRegular,
            50_000,
        );
        assert_eq!(level, StalemateLevel::Exhausted);
    }

    #[test]
    fn three_rejections_affordable_but_unrest_is_exhausted() {
        // Even when the ask is affordable, an explicit REQ/UNH means the
        // player wants out — let the listing path proceed.
        let ask = PlayerContractAsk {
            desired_salary: 60_000,
            desired_years: 3,
            recorded_on: d(2026, 4, 1),
            demanded_status: None,
            demanded_release_clause: None,
            demanded_signing_bonus: None,
            rejection_reason: Some(RejectionReason::LowSalary),
        };
        let level = compute_level(
            3,
            Some(10),
            Some(400),
            &Some(ask),
            Some(true),
            true,
            &PlayerSquadStatus::FirstTeamRegular,
            50_000,
        );
        assert_eq!(level, StalemateLevel::Exhausted);
    }

    #[test]
    fn level_one_rejection_with_expiry_pressure_is_severe() {
        let level = compute_level(
            1,
            Some(20),
            Some(120),
            &None,
            None,
            false,
            &PlayerSquadStatus::MainBackupPlayer,
            10_000,
        );
        assert_eq!(level, StalemateLevel::Severe);
    }

    #[test]
    fn level_critical_expiry_with_two_rejections_is_exhausted() {
        let level = compute_level(
            2,
            Some(5),
            Some(45),
            &None,
            None,
            false,
            &PlayerSquadStatus::FirstTeamRegular,
            10_000,
        );
        assert_eq!(level, StalemateLevel::Exhausted);
    }

    #[test]
    fn surplus_recent_rejection_escalates_to_severe() {
        let level = compute_level(
            1,
            Some(15),
            Some(400),
            &None,
            None,
            false,
            &PlayerSquadStatus::NotNeeded,
            10_000,
        );
        assert_eq!(level, StalemateLevel::Severe);
    }

    #[test]
    fn permits_listing_keyplayer_needs_exhausted() {
        let mut s = ContractStalemate {
            offers_12m: 2,
            rejections_12m: 2,
            last_rejection_days_ago: Some(10),
            days_to_expiry: Some(400),
            squad_status: PlayerSquadStatus::KeyPlayer,
            has_market_interest: false,
            is_unrest: false,
            pending_ask: None,
            ask_affordable: None,
            level: StalemateLevel::Severe,
        };
        assert!(!s.permits_listing(), "Severe should not list a KeyPlayer");
        s.level = StalemateLevel::Exhausted;
        assert!(s.permits_listing());
    }

    #[test]
    fn permits_listing_backup_at_severe() {
        let s = ContractStalemate {
            offers_12m: 2,
            rejections_12m: 2,
            last_rejection_days_ago: Some(10),
            days_to_expiry: Some(400),
            squad_status: PlayerSquadStatus::MainBackupPlayer,
            has_market_interest: false,
            is_unrest: false,
            pending_ask: None,
            ask_affordable: None,
            level: StalemateLevel::Severe,
        };
        assert!(s.permits_listing());
    }

    #[test]
    fn permits_listing_notneeded_at_emerging() {
        let s = ContractStalemate {
            offers_12m: 1,
            rejections_12m: 1,
            last_rejection_days_ago: Some(60),
            days_to_expiry: Some(400),
            squad_status: PlayerSquadStatus::NotNeeded,
            has_market_interest: false,
            is_unrest: false,
            pending_ask: None,
            ask_affordable: None,
            level: StalemateLevel::Emerging,
        };
        assert!(s.permits_listing());
    }

    #[test]
    fn should_improve_offer_when_ask_affordable() {
        let s = ContractStalemate {
            offers_12m: 1,
            rejections_12m: 1,
            last_rejection_days_ago: Some(10),
            days_to_expiry: Some(400),
            squad_status: PlayerSquadStatus::FirstTeamRegular,
            has_market_interest: false,
            is_unrest: false,
            pending_ask: None,
            ask_affordable: Some(true),
            level: StalemateLevel::Emerging,
        };
        assert!(s.should_improve_offer());
    }

    #[test]
    fn should_not_improve_when_exhausted() {
        let s = ContractStalemate {
            offers_12m: 3,
            rejections_12m: 3,
            last_rejection_days_ago: Some(10),
            days_to_expiry: Some(60),
            squad_status: PlayerSquadStatus::MainBackupPlayer,
            has_market_interest: false,
            is_unrest: false,
            pending_ask: None,
            ask_affordable: Some(true),
            level: StalemateLevel::Exhausted,
        };
        assert!(!s.should_improve_offer());
    }

    // ─── Full-player integration cases ──────────────────────────────

    fn affordable() -> AffordabilityInput {
        AffordabilityInput {
            wage_budget_headroom: Some(1_000_000),
            current_salary: 50_000,
        }
    }

    fn tight() -> AffordabilityInput {
        AffordabilityInput {
            wage_budget_headroom: Some(0),
            current_salary: 50_000,
        }
    }

    #[test]
    fn first_rejection_for_useful_player_does_not_permit_listing() {
        let today = d(2026, 5, 1);
        let history = make_history(&[
            (d(2026, 4, 1), RENEWAL_OFFERED_LABEL),
            (d(2026, 4, 15), RENEWAL_REJECTED_LABEL),
        ]);
        let contract = make_contract(50_000, PlayerSquadStatus::FirstTeamRegular, d(2027, 7, 1));
        let player = make_player_with(history, Some(contract), None);
        let s = ContractStalemate::assess(&player, today, affordable());
        assert_eq!(s.level, StalemateLevel::Emerging);
        assert!(
            !s.permits_listing(),
            "FirstTeamRegular needs Exhausted, not Emerging"
        );
    }

    #[test]
    fn repeated_rejections_with_approaching_expiry_can_list_backup() {
        let today = d(2026, 5, 1);
        let history = make_history(&[
            (d(2026, 1, 10), RENEWAL_OFFERED_LABEL),
            (d(2026, 1, 25), RENEWAL_REJECTED_LABEL),
            (d(2026, 3, 1), RENEWAL_OFFERED_LABEL),
            (d(2026, 3, 18), RENEWAL_REJECTED_LABEL),
        ]);
        // Expires in ~150 days — within EXPIRY_PRESSURE_DAYS.
        let contract = make_contract(50_000, PlayerSquadStatus::MainBackupPlayer, d(2026, 9, 28));
        let player = make_player_with(history, Some(contract), None);
        let s = ContractStalemate::assess(&player, today, affordable());
        assert_eq!(s.level, StalemateLevel::Severe);
        assert!(s.permits_listing(), "Backup at Severe should be listable");
    }

    #[test]
    fn keyplayer_protected_at_severe() {
        let today = d(2026, 5, 1);
        let history = make_history(&[
            (d(2026, 1, 10), RENEWAL_OFFERED_LABEL),
            (d(2026, 1, 25), RENEWAL_REJECTED_LABEL),
            (d(2026, 3, 1), RENEWAL_OFFERED_LABEL),
            (d(2026, 3, 18), RENEWAL_REJECTED_LABEL),
        ]);
        let contract = make_contract(120_000, PlayerSquadStatus::KeyPlayer, d(2027, 7, 1));
        let player = make_player_with(history, Some(contract), None);
        let s = ContractStalemate::assess(&player, today, affordable());
        assert!(matches!(s.level, StalemateLevel::Severe));
        assert!(!s.permits_listing(), "KeyPlayer should not list at Severe");
    }

    #[test]
    fn keyplayer_listable_when_exhausted() {
        let today = d(2026, 5, 1);
        let history = make_history(&[
            (d(2026, 1, 10), RENEWAL_OFFERED_LABEL),
            (d(2026, 1, 25), RENEWAL_REJECTED_LABEL),
            (d(2026, 3, 1), RENEWAL_OFFERED_LABEL),
            (d(2026, 3, 18), RENEWAL_REJECTED_LABEL),
            (d(2026, 4, 1), RENEWAL_OFFERED_LABEL),
            (d(2026, 4, 25), RENEWAL_REJECTED_LABEL),
        ]);
        let contract = make_contract(120_000, PlayerSquadStatus::KeyPlayer, d(2027, 7, 1));
        let player = make_player_with(history, Some(contract), None);
        let s = ContractStalemate::assess(&player, today, affordable());
        assert_eq!(s.level, StalemateLevel::Exhausted);
        assert!(s.permits_listing());
    }

    #[test]
    fn notneeded_listable_after_first_failed_talk() {
        let today = d(2026, 5, 1);
        let history = make_history(&[
            (d(2026, 4, 1), RENEWAL_OFFERED_LABEL),
            (d(2026, 4, 15), RENEWAL_REJECTED_LABEL),
        ]);
        let contract = make_contract(50_000, PlayerSquadStatus::NotNeeded, d(2027, 7, 1));
        let player = make_player_with(history, Some(contract), None);
        let s = ContractStalemate::assess(&player, today, affordable());
        assert!(matches!(
            s.level,
            StalemateLevel::Severe | StalemateLevel::Emerging
        ));
        assert!(
            s.permits_listing(),
            "NotNeeded permits listing as soon as renewal has visibly failed"
        );
    }

    #[test]
    fn pure_expiry_with_no_rejection_history_is_not_listed() {
        let today = d(2026, 5, 1);
        let history = PlayerDecisionHistory::new();
        // Expiry in 90 days — well inside the old 180-day trigger.
        let contract = make_contract(50_000, PlayerSquadStatus::FirstTeamRegular, d(2026, 7, 30));
        let player = make_player_with(history, Some(contract), None);
        let s = ContractStalemate::assess(&player, today, affordable());
        assert_eq!(s.level, StalemateLevel::None);
        assert_eq!(s.rejections_12m, 0);
        assert!(
            !s.permits_listing(),
            "Bare expiry must NOT permit listing — only failed renewals do"
        );
    }

    #[test]
    fn affordable_pending_ask_triggers_improved_offer() {
        let today = d(2026, 5, 1);
        let history = make_history(&[
            (d(2026, 4, 1), RENEWAL_OFFERED_LABEL),
            (d(2026, 4, 15), RENEWAL_REJECTED_LABEL),
        ]);
        let contract = make_contract(50_000, PlayerSquadStatus::FirstTeamRegular, d(2027, 7, 1));
        let ask = PlayerContractAsk {
            desired_salary: 80_000,
            desired_years: 3,
            recorded_on: d(2026, 4, 15),
            demanded_status: None,
            demanded_release_clause: None,
            demanded_signing_bonus: None,
            rejection_reason: Some(RejectionReason::LowSalary),
        };
        let player = make_player_with(history, Some(contract), Some(ask));
        let s = ContractStalemate::assess(&player, today, affordable());
        assert_eq!(s.ask_affordable, Some(true));
        assert!(s.should_improve_offer());
        assert!(!s.permits_listing(), "Affordable ask must not list");
    }

    #[test]
    fn unaffordable_repeated_ask_can_list() {
        let today = d(2026, 5, 1);
        let history = make_history(&[
            (d(2026, 1, 10), RENEWAL_OFFERED_LABEL),
            (d(2026, 1, 25), RENEWAL_REJECTED_LABEL),
            (d(2026, 3, 1), RENEWAL_OFFERED_LABEL),
            (d(2026, 3, 18), RENEWAL_REJECTED_LABEL),
        ]);
        let contract = make_contract(50_000, PlayerSquadStatus::MainBackupPlayer, d(2027, 7, 1));
        let ask = PlayerContractAsk {
            desired_salary: 250_000,
            desired_years: 4,
            recorded_on: d(2026, 3, 18),
            demanded_status: None,
            demanded_release_clause: None,
            demanded_signing_bonus: None,
            rejection_reason: Some(RejectionReason::LowSalary),
        };
        let player = make_player_with(history, Some(contract), Some(ask));
        let s = ContractStalemate::assess(&player, today, tight());
        assert_eq!(s.ask_affordable, Some(false));
        assert!(matches!(
            s.level,
            StalemateLevel::Severe | StalemateLevel::Exhausted
        ));
        assert!(s.permits_listing());
        assert!(!s.should_improve_offer());
    }

    #[test]
    fn old_rejections_outside_window_are_dropped() {
        let today = d(2026, 6, 1);
        let history = make_history(&[
            (d(2024, 1, 1), RENEWAL_REJECTED_LABEL),
            (d(2025, 1, 1), RENEWAL_REJECTED_LABEL),
            (d(2026, 4, 1), RENEWAL_REJECTED_LABEL),
        ]);
        let cutoff = today - chrono::Duration::days(STALEMATE_WINDOW_DAYS);
        let count = history
            .items
            .iter()
            .filter(|d| d.date >= cutoff && d.decision == RENEWAL_REJECTED_LABEL)
            .count();
        assert_eq!(count, 1, "only the rejection inside the 365d window counts");
    }
}
