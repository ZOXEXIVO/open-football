//! Free types shared between the various Player::on_* event handlers.
//!
//! Constructed by callers (league/match-result pipeline, transfer
//! pipeline) and handed to the player one outcome at a time.

use chrono::NaiveDate;

use crate::TeamInfo;
use crate::club::PlayerClubContract;
use crate::r#match::PlayerMatchEndStats;
use crate::transfers::offer::PersonalTermsOffer;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MatchParticipation {
    Starter,
    Substitute,
}

/// Identity of the team a player was fielded for in a match. Usually the
/// player's own (rostered) team, but for a borrowed appearance across two
/// teams of the same club (a reserve/Second player pulled up to the main
/// XI, or vice versa) it differs from the player's active history spell.
/// Carried on [`MatchOutcome`] so league stat bookkeeping can attribute
/// the appearance to the correct team — the home team's games stay on
/// `Player::statistics`, everything else lands in the per-team secondary
/// bucket and surfaces as its own career-history row.
#[derive(Debug, Clone, Copy)]
pub struct MatchTeamRef<'a> {
    pub slug: &'a str,
    pub name: &'a str,
    pub reputation: u16,
    pub league_slug: &'a str,
    pub league_name: &'a str,
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
    /// Slug of the competition this match belongs to (the match's
    /// `league_slug`). For cup matches it keys the per-competition cup
    /// stats bucket so each cup keeps its own line; ignored for league
    /// and friendly matches.
    pub competition_slug: &'a str,
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
    /// True when this is a continental competition match (UCL, UEL,
    /// Conference, Libertadores). Lets the events feed promote a
    /// regular cup tie into a "knockout" headline only when continental.
    pub is_continental: bool,
    /// Opponent team id, when known. Used by the relationship-arc
    /// emit path to name the rival in the headline / detail row.
    pub opponent_team_id: Option<u32>,
    /// The team this player was actually fielded for. `None` when the
    /// caller doesn't resolve it; only the league-result pipeline
    /// populates it. Drives per-team league attribution so a borrowed
    /// appearance for another of the club's teams gets its own history
    /// row instead of folding under the player's active-spell team.
    pub played_for: Option<MatchTeamRef<'a>>,
    /// Season start-year this match belongs to (`Season::from_date`).
    /// Used only when booking a borrowed (secondary-team) league
    /// appearance so it freezes under the right season; ignored for the
    /// home bucket. 0 when unresolved.
    pub match_season_year: u16,
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

    /// Classify this fixture as one of the dedicated "big match" kinds,
    /// or `None` for a routine league game. The render-side
    /// `BigMatchKind` enum is the closed catalog of headline-worthy
    /// fixtures; emit sites use this to decide whether to fire
    /// `TrustedInBigMatch` / `BenchedForBigMatch`. We deliberately don't
    /// promote a plain domestic cup round-1 tie to "big" — only the
    /// late-stage / final / continental cases qualify.
    pub fn big_match_kind(&self) -> Option<crate::BigMatchKind> {
        use crate::BigMatchKind;
        // Friendlies never qualify as "big" — pre-season minutes don't
        // carry the same psychological weight.
        if self.is_friendly {
            return None;
        }
        if self.is_continental && self.is_cup {
            // Continental cups in this sim are always knockout-flavoured
            // once we reach the dedicated competition IDs (group stage
            // ties already feature on the badge so trust still applies).
            return Some(BigMatchKind::ContinentalKnockout);
        }
        if self.is_derby {
            return Some(BigMatchKind::Derby);
        }
        // Domestic cup ties below the continental tier — we only promote
        // them to "big" once the slug carries the conventional
        // late-round marker. Most generators name the bracket cup
        // straightforwardly enough that the slug check covers the
        // realistic case without dragging the full bracket through.
        if self.is_cup {
            let slug = self.competition_slug.to_ascii_lowercase();
            if slug.contains("final") || slug.contains("semi") || slug.contains("quarter") {
                return Some(BigMatchKind::NationalCupSemiOrLater);
            }
        }
        None
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
    /// Selling club's league reputation (0–10000). Captured at this point
    /// so the transfer-environment profile in `process_transfer_shock`
    /// can compare league_rep_gap without re-walking the world. 0 when
    /// unknown.
    pub selling_league_reputation: u16,
    /// True when the selling club sits in the buying club's rivals list.
    /// Captured at staging time (the executor has both club objects; the
    /// player never does) — drives the cold-shoulder reception at the
    /// new club.
    pub source_is_rival: bool,
    /// Sell-on percentage pledged by the buyer to the current seller. Added
    /// to `player.sell_on_obligations` so the next sale pays the seller out.
    pub record_sell_on: Option<f32>,
    /// Structured personal-terms package agreed during the AI flow —
    /// signing bonus, agent fee, release clause, contract years,
    /// promised role. When `Some`, execution installs the exact deal
    /// that was negotiated. When `None`, falls back to compute-from-
    /// context defaults for legacy callers (manual UI moves, free-
    /// agent in-country signings, tests). All fields inside are
    /// individually `Option` so a partial package is honoured field-
    /// by-field.
    pub personal_terms: Option<PersonalTermsOffer>,
}

pub struct LoanCompletion<'a> {
    pub from: &'a TeamInfo,
    /// History identity of the squad the player physically occupies
    /// (own slug for Main/B/Second, Main alias for Reserve/youth). The
    /// loan departs THIS spell. Distinct from `from`, which stays the
    /// parent Main team so the transfer-shock prestige anchor remains the
    /// player's club rather than the lower-rep reserve squad he was parked
    /// on. Equal to `from` for a player loaned straight off the Main team.
    pub history_source: &'a TeamInfo,
    pub to: &'a TeamInfo,
    pub loan_fee: f64,
    pub date: NaiveDate,
    pub loan_contract: PlayerClubContract,
    pub borrowing_club_id: u32,
    /// Parent (selling) club's league reputation (0–10000), captured so
    /// the transfer-environment profile can compare a loan's source tier.
    /// 0 when unknown.
    pub parent_league_reputation: u16,
}
