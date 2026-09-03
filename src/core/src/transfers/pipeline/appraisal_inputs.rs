//! Where the appraisal's two inputs come from.
//!
//! [`super::appraisal`] is deliberately world-free — it reasons over a
//! flat [`OfferView`] and a flat [`PlayerStance`] so every row of the
//! calibration tables is a unit test against a plain struct. This module
//! is the other half: the two builders that walk the simulator once and
//! fill those structs in.
//!
//! The split matters most across a border. A domestic negotiation can
//! rebuild the stance live at every round; a cross-border one cannot —
//! the player, his club, his depth chart and his mind all live in another
//! country's borrow by the time personal terms resolve. So the stance is
//! **staged at negotiation creation**, when the seller's country is still
//! in scope, and rides on the negotiation exactly as
//! `foreign_seller_importance` and `foreign_seller_finances` already do.
//! One builder, two call sites, no second model.

use super::appraisal::{OfferKind, OfferView, PlayerStance};
use crate::club::player::calculators::{ContractValuation, ValuationContext};
use crate::club::player::contract::agent::PlayerAgent;
use crate::club::player::happiness::processing::PlayingTimeFrustrationConfig;
use crate::club::player::language::Language;
use crate::club::player::mind::MindSituation;
use crate::club::player::statistics::StuckCareerScan;
use crate::transfers::ScoutingRegion;
use crate::transfers::market::TransferListingOrigin;
use crate::transfers::offer::PromisedSquadStatus;
use crate::transfers::pipeline::PlayerSummary;
use crate::{Club, Country, HappinessEventType, Player, PlayerSquadStatus, PlayerStatusType};
use chrono::NaiveDate;

/// Everything the stance builder needs that it cannot read off the player.
///
/// Assembled by the caller that already holds the two clubs — the
/// domestic resolver, or the cross-border creation pass.
pub struct StanceInputs<'a> {
    pub player: &'a Player,
    /// The country the player currently plays in.
    pub seller_country: &'a Country,
    pub seller_club: &'a Club,
    /// Who is asking. Used for his memory of them and for the boyhood-club
    /// read; the rest of the buyer's side is on the [`OfferView`].
    pub buyer_club_id: u32,
    /// Buyer club reputation minus seller club reputation, 0..1 scale —
    /// the only thing the agent's lean reads.
    pub rep_diff: f32,
    /// What he is to his current club, 0..1. The ONE importance formula
    /// (`TransferPlausibilityEvaluator::player_importance`), passed in so
    /// the domestic and foreign paths cannot drift apart on it.
    pub importance: f32,
    /// His club has genuinely advertised him — a seller-listed row, not a
    /// synthetic stub backing somebody's cold call.
    pub listed_by_club: bool,
    /// Any real availability signal at all (`Lst`/`Loa`/`Req`/`Unh`,
    /// `NotNeeded`, a seller listing).
    pub available: bool,
    /// Months until his nation's next tournament, `u8::MAX` for none.
    pub months_to_tournament: u8,
    pub date: NaiveDate,
}

/// What the market can honestly see about a player being gettable, TYPE-
/// MATCHED to the deal on the table.
///
/// There were three definitions of "listed". The domestic resolver read
/// the listing's origin and matched permanent-vs-loan; the foreign
/// permanent path read `Lst || Loa` for the advert and
/// `Lst || Loa || Req || Unh || NotNeeded` for the signal. So a
/// LOAN-listed player collected the "his club is advertising him" push
/// toward a PERMANENT move abroad and not toward the same move at home.
///
/// One reading, one place. A permanent advert supports a permanent
/// approach and a loan advert supports a loan; everything else — a
/// request, unhappiness, a squad status of `NotNeeded` — is a signal
/// about the man rather than about the deal, and counts either way.
#[derive(Debug, Clone, Copy, Default)]
pub struct AvailabilityView {
    /// Any real signal at all: an advert for THIS kind of deal, a formal
    /// request, unhappiness, or a club that has written him off.
    pub available_soft: bool,
    /// His club has genuinely advertised him for this kind of deal — not
    /// a synthetic listing created to back somebody's cold call.
    pub listed_by_club: bool,
    pub requested: bool,
    pub unhappy: bool,
}

impl AvailabilityView {
    /// Read a player. `listing_origin` is the origin of the listing the
    /// negotiation is bound to, when there is one; `None` on a path that
    /// has no listing in scope.
    pub fn read(
        player: &Player,
        is_loan: bool,
        listing_origin: Option<TransferListingOrigin>,
    ) -> Self {
        let advertised_permanent = player.statuses.has(PlayerStatusType::Lst)
            || matches!(
                listing_origin,
                Some(TransferListingOrigin::SellerListed)
                    | Some(TransferListingOrigin::EndOfContract)
            );
        let advertised_loan = player.statuses.has(PlayerStatusType::Loa)
            || matches!(listing_origin, Some(TransferListingOrigin::LoanOutListed));
        let listed_by_club = if is_loan {
            advertised_loan
        } else {
            advertised_permanent
        };
        let requested = player.statuses.has(PlayerStatusType::Req);
        let unhappy = player.statuses.has(PlayerStatusType::Unh);
        let not_needed = player
            .contract
            .as_ref()
            .is_some_and(|c| matches!(c.squad_status, PlayerSquadStatus::NotNeeded));
        AvailabilityView {
            available_soft: listed_by_club || requested || unhappy || not_needed,
            listed_by_club,
            requested,
            unhappy,
        }
    }
}

/// Builds the player's side of the appraisal.
pub struct PlayerStanceBuilder;

impl PlayerStanceBuilder {
    /// Days of a `WantsReturnHome` mood that still count as current. The
    /// mood fires on a 60-day cooldown, so three windows is a man who has
    /// been saying it all season rather than one bad month.
    const HOME_MOOD_WINDOW: u16 = 180;

    pub fn build(inputs: &StanceInputs<'_>) -> PlayerStance {
        let player = inputs.player;
        let date = inputs.date;
        let country = inputs.seller_country;

        // The mind's own weekly picture, rebuilt from the same builder the
        // faculties read — never a second copy that can drift. Cheap: it is
        // fields, not scans.
        let mut situation = player.mind_situation(date, country.id, &country.code);
        situation.months_to_tournament = inputs.months_to_tournament;

        let seller_rep_score = inputs
            .seller_club
            .teams
            .main()
            .map(|t| t.reputation.overall_score())
            .unwrap_or(0.3);
        let seller_league_rep = inputs
            .seller_club
            .teams
            .main()
            .or_else(|| inputs.seller_club.teams.teams.first())
            .and_then(|t| t.league_id)
            .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.reputation)
            .unwrap_or(0);

        let current_wage = player.contract.as_ref().map(|c| c.salary).unwrap_or(0);
        // What a man of his standing is worth at the level he is playing
        // at now. Paired with what he is actually paid, this is what turns
        // "a 50 % cut" into "back to fair value" for the legacy earner and
        // into "a raise" for the underpaid starter.
        let fair_wage_at_current = ContractValuation::evaluate(
            player,
            &ValuationContext {
                age: situation.age,
                club_reputation_score: seller_rep_score,
                league_reputation: seller_league_rep,
                squad_status: player
                    .contract
                    .as_ref()
                    .map(|c| c.squad_status.clone())
                    .unwrap_or(PlayerSquadStatus::FirstTeamRegular),
                current_salary: current_wage,
                months_remaining: player
                    .contract
                    .as_ref()
                    .map(|c| ((c.expiration - date).num_days() / 30) as i32)
                    .unwrap_or(0),
                has_market_interest: inputs.available,
            },
        )
        .expected_wage;

        let tenure_days = StuckCareerScan::club_tenure_days(player, date)
            .unwrap_or(i64::from(u16::MAX))
            .clamp(0, i64::from(u16::MAX)) as u16;

        let mind_ctx = player.mind_context(date, Some(inputs.seller_club.id));

        let stance = PlayerStance {
            current_wage: current_wage as f64,
            fair_wage_at_current: fair_wage_at_current as f64,
            market_resignation: player.market_resignation(date),
            importance: inputs.importance,
            big_stage_inclination: player.big_stage_inclination,
            nationality_country_id: player.country_id,
            nationality_region: player.home_region(),
            available_soft: inputs.available,
            unhappy: player.statuses.has(PlayerStatusType::Unh),
            requested: player.statuses.has(PlayerStatusType::Req),
            listed_by_club: inputs.listed_by_club,
            at_favourite_club: player.favorite_clubs.contains(&inputs.seller_club.id),
            buyer_sentiment: player
                .mind
                .club_sentiment(inputs.buyer_club_id, &mind_ctx)
                .clamp(-1.0, 1.0),
            buyer_is_favourite: player.favorite_clubs.contains(&inputs.buyer_club_id),
            language_profile: player.language_profile(),
            seller_continent_id: country.continent_id,
            seller_country_id: country.id,
            seller_region: ScoutingRegion::from_country(country.continent_id, &country.code),
            ..PlayerStance::neutral()
        };

        stance
            .with_situation(&situation)
            .with_mind(&player.mind, Self::home_mood_desire(player, &situation))
            .with_agent(&PlayerAgent::for_player(player), inputs.rep_diff)
            // `club_tenure_days` is the honest anchor for a stay — a loan
            // return re-stamps `last_transfer_date`, which would otherwise
            // read a five-year servant as a brand-new signing twice a year
            // (memory `loan_pipeline`).
            .with_tenure(tenure_days)
    }

    /// The player's side, from a market summary alone.
    ///
    /// The cross-border LOAN market never holds the player — it works off
    /// the world pool the way a scouting network does — so this is the
    /// stance a borrowing club can honestly build about a man it has only
    /// read about. Everything it cannot know stays at its no-view default,
    /// and on a loan that costs almost nothing: money and sport are halved
    /// anyway, and what actually decides the answer is the shirt, the
    /// push, and whether he is being asked to go home.
    ///
    /// `importance` is the ONE importance formula, passed in from the
    /// staged plausibility read the loan scan already runs.
    ///
    /// # What still differs from the live builder
    ///
    /// Everything a market summary cannot carry stays at its no-view
    /// default, and the residual list is short and deliberate:
    ///
    /// | axis | live | from a summary |
    /// |---|---|---|
    /// | `nt_stake` | tournament clock × standing | 0 — a borrower abroad does not know his federation's calendar |
    /// | `agent_bias` | [`PlayerAgent`] on this move | 0 — the agent is not on the pool |
    /// | `buyer_sentiment` | his memory of the buying club | 0 |
    /// | `at_favourite_club` / `buyer_is_favourite` | his own list | false |
    /// | `secure_future_pressure` | the mind's `SecureMyFuture` | 0 |
    /// | `playing_time_gap` | `starter − standing_expectation` (wage-aware) | `starter − expected_start_share` (role only) |
    ///
    /// Everything else — money, personality, the shirt, the push, the
    /// stay, where he is from — now travels, so a loyalty-18 boyhood-club
    /// Brazilian reads the same abroad as at home.
    pub fn from_summary(
        summary: &PlayerSummary,
        importance: f32,
        seller_region: ScoutingRegion,
    ) -> PlayerStance {
        let runway = ((34.0 - summary.age as f32) / 12.0).clamp(0.0, 1.0);
        let wage = (summary.salary as f64).max(1.0);
        PlayerStance {
            current_wage: wage,
            // No valuation is knowable from a summary — the borrower has
            // read about him, not appraised him — so his own wage is the
            // honest no-view anchor.
            //
            // This is NOT what silences the money axis on a loan. That is
            // done where it belongs, on the offer: a loan pays the deal he
            // already has, so `offered_wage == anchor` and `M ≡ 0` on the
            // domestic and the foreign path alike (Part VIII, "the loan
            // that pays"). Reading the same value here was an accident
            // that happened to agree.
            fair_wage_at_current: wage,
            age: summary.age,
            career_runway: runway,
            career_spent: 1.0 - runway,
            contract_pressure: (1.0_f32
                - (summary.contract_months_remaining.max(0) as f32 * 30.0) / 730.0)
                .clamp(0.0, 1.0),
            // His own three drives, not a stand-in. `determination` was
            // standing in for ambition and loyalty/adaptability sat at the
            // neutral 0.5, so the same man read differently across a
            // border.
            ambition_drive: (summary.ambition as f32 / 20.0).clamp(0.0, 1.0),
            loyalty_drive: (summary.loyalty as f32 / 20.0).clamp(0.0, 1.0),
            adaptability_drive: (summary.adaptability as f32 / 20.0).clamp(0.0, 1.0),
            big_stage_inclination: summary.seller_ctx.big_stage_inclination,
            importance,
            starter_ratio: summary.starter_share,
            // Against what his ROLE implies, the way the live builder's
            // `MindSituation::playing_time_gap` does — not against a flat
            // half, which read every rotation player as a man being
            // frozen out.
            playing_time_gap: (summary.starter_share
                - PlayingTimeFrustrationConfig::expected_start_share(Some(
                    &summary.seller_ctx.squad_status,
                )))
            .clamp(-1.0, 1.0),
            market_resignation: summary.seller_ctx.market_resignation,
            nationality_country_id: summary.nationality_country_id,
            nationality_region: summary.nationality_region,
            days_at_club: summary.tenure_days,
            return_home_desire: summary.return_home_desire,
            available_soft: summary.is_listed || summary.is_loan_listed,
            unhappy: summary.seller_ctx.is_unhappy,
            requested: summary.seller_ctx.is_transfer_requested,
            listed_by_club: summary.is_loan_listed,
            // The mind's own restlessness, carried on the summary — the
            // posting is the club's decision and is already read through
            // `home_return_wanted` on the home axis, so it is not a second
            // channel into the push. A posted man usually has the want as
            // well; taking the MAX keeps the two from counting twice.
            leave_pressure: summary
                .leave_pressure
                .max(if summary.home_return_wanted { 0.6 } else { 0.0 })
                .clamp(0.0, 1.0),
            stay_pressure: summary.stay_pressure.clamp(0.0, 1.0),
            language_profile: summary.language_profile,
            buyer_is_favourite: false,
            buyer_sentiment: 0.0,
            seller_continent_id: summary.continent_id,
            seller_country_id: summary.country_id,
            seller_region,
            ..PlayerStance::neutral()
        }
    }

    /// How badly he wants to go home, from the channels the mind does NOT
    /// own: the legacy `WantsReturnHome` mood and raw cultural isolation.
    ///
    /// Taken as the floor under the mind's own `GoHome` pressure rather
    /// than added to it, so a player who is homesick in both models is not
    /// counted twice (Part VIII, "double-counting push").
    pub fn home_mood_desire(player: &Player, situation: &MindSituation) -> f32 {
        let recent = player
            .happiness
            .recent_events
            .iter()
            .filter(|e| {
                e.event_type == HappinessEventType::WantsReturnHome
                    && e.days_ago <= Self::HOME_MOOD_WINDOW
            })
            .map(|e| {
                // Fades over the window rather than switching off at its
                // edge — a mood from last week is louder than one from
                // five months ago.
                1.0 - (e.days_ago as f32 / Self::HOME_MOOD_WINDOW as f32).clamp(0.0, 1.0)
            })
            .fold(0.0_f32, f32::max);
        let isolation = if situation.is_culturally_isolated() {
            0.45
        } else if situation.is_abroad && !situation.speaks_local_language {
            0.25
        } else {
            0.0
        };
        recent.max(isolation).clamp(0.0, 1.0)
    }
}

/// Builds the buyer's side of the appraisal.
pub struct OfferViewBuilder;

impl OfferViewBuilder {
    /// The move as the buying club is putting it.
    ///
    /// Everything about the player comes off the STANCE, never off a
    /// `&Player` — the buyer's country cannot reach him on a cross-border
    /// deal, and a builder that needed him would have had to be written
    /// twice. `seller_region` / `seller_continent_id` come off the stance
    /// too, so the place term is priced against where he actually lives
    /// rather than where the negotiation happens to be resolved.
    pub fn build(
        kind: OfferKind,
        buyer_country: &Country,
        buyer_club_id: u32,
        stance: &PlayerStance,
        offered_wage: f64,
        promised_status: Option<PromisedSquadStatus>,
        sporting_drop: f32,
    ) -> OfferView {
        let buyer_region =
            ScoutingRegion::from_country(buyer_country.continent_id, &buyer_country.code);
        OfferView {
            kind,
            buyer_club_id,
            buyer_country_id: buyer_country.id,
            buyer_continent_id: buyer_country.continent_id,
            buyer_region,
            offered_wage,
            promised_status,
            sporting_drop,
            prestige_drop: stance.seller_region.league_prestige() - buyer_region.league_prestige(),
            crosses_continent: stance.seller_continent_id != buyer_country.continent_id,
            language_affinity: stance
                .language_profile
                .affinity_for(Language::country_language_mask(&buyer_country.code)),
            is_favourite_club: stance.buyer_is_favourite,
            deadline_urgency: 0.0,
            release_clause_triggered: false,
            returning_to_seller: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::appraisal::{AppraisalConfig, PlayerOfferAppraisal};
    use super::*;
    use crate::club::player::ability::position::PositionCoverage;
    use crate::club::player::language::LanguageProfile;
    use crate::transfers::pipeline::SellerPlausibilityContext;
    use crate::transfers::pipeline::standing::CareerRecordSnapshot;
    use crate::{PlayerFieldPositionGroup, PlayerPositionType};

    /// A market summary of a homesick loyal Brazilian at an English club:
    /// three starts in a season, a formed want, and the personality the
    /// live builder would read straight off the player.
    fn brazilian_at_arsenal() -> PlayerSummary {
        PlayerSummary {
            player_id: 1,
            club_id: 100,
            country_id: 1,
            continent_id: 1,
            region: ScoutingRegion::WesternEurope,
            country_code: "GB".to_string(),
            nationality_country_id: 55,
            nationality_continent_id: Some(7),
            nationality_region: Some(ScoutingRegion::SouthAmerica),
            starter_share: 0.12,
            tenure_days: 400,
            return_home_desire: 0.7,
            home_return_wanted: true,
            ambition: 14,
            loyalty: 18,
            adaptability: 6,
            leave_pressure: 0.55,
            stay_pressure: 0.1,
            player_name: "Test".to_string(),
            club_name: "Test Club".to_string(),
            position: PlayerPositionType::Striker,
            position_group: PlayerFieldPositionGroup::Forward,
            coverage: PositionCoverage::single(PlayerPositionType::Striker),
            age: 21,
            estimated_value: 8_000_000.0,
            is_listed: false,
            is_loan_listed: true,
            skill_ability: 120,
            average_rating: 6.6,
            goals: 1,
            assists: 0,
            appearances: 8,
            determination: 12.0,
            work_rate: 12.0,
            composure: 12.0,
            anticipation: 12.0,
            technical_avg: 12.0,
            mental_avg: 12.0,
            physical_avg: 12.0,
            current_reputation: 3000,
            home_reputation: 3000,
            world_reputation: 3000,
            country_reputation: 8000,
            club_world_reputation: 8000,
            club_best_in_group: 160,
            is_injured: false,
            contract_months_remaining: 36,
            salary: 900_000,
            language_profile: LanguageProfile::default(),
            international_apps: 0,
            career_record: CareerRecordSnapshot::default(),
            seller_ctx: SellerPlausibilityContext {
                club_reputation_score: 0.8,
                league_reputation: 8000,
                league_id: None,
                position_group_rank: 3,
                squad_status: PlayerSquadStatus::HotProspectForTheFuture,
                is_transfer_requested: false,
                is_unhappy: false,
                in_debt: false,
                days_on_market: 0,
                market_resignation: 0.0,
                club_matches_played: 30,
                big_stage_inclination: 0.3,
                is_marketed: false,
            },
        }
    }

    /// B5 — the same man, read live and read off a summary, has to agree.
    ///
    /// The eight axes the cross-border stance used to guess (ambition off
    /// `determination`, loyalty and adaptability at the neutral 0.5, the
    /// playing-time gap against a flat half, the push from the posting
    /// flag alone) made a loyalty-18 boyhood-club Brazilian read as a
    /// 0.125 attachment abroad against 0.35+ at home.
    #[test]
    fn a_stance_from_a_summary_agrees_with_the_live_one_on_a_loan() {
        let summary = brazilian_at_arsenal();
        let staged =
            PlayerStanceBuilder::from_summary(&summary, 0.35, ScoutingRegion::WesternEurope);

        // What the live builder would produce for the same player: the
        // drives off his attributes, the wants off his mind, the gap
        // against what his role implies.
        let live = PlayerStance {
            ambition_drive: 14.0 / 20.0,
            loyalty_drive: 18.0 / 20.0,
            adaptability_drive: 6.0 / 20.0,
            leave_pressure: 0.55,
            stay_pressure: 0.1,
            playing_time_gap: 0.12 - 0.10,
            ..staged
        };

        assert!((staged.loyalty_drive - live.loyalty_drive).abs() < 1e-6);
        assert!((staged.ambition_drive - live.ambition_drive).abs() < 1e-6);
        assert!((staged.adaptability_drive - live.adaptability_drive).abs() < 1e-6);
        assert!((staged.playing_time_gap - live.playing_time_gap).abs() < 0.02);

        let loan_home = OfferView {
            kind: OfferKind::Loan,
            offered_wage: PlayerOfferAppraisal::anchor(&staged),
            buyer_country_id: 55,
            buyer_region: ScoutingRegion::SouthAmerica,
            promised_status: Some(PromisedSquadStatus::FirstTeamRegular),
            crosses_continent: true,
            ..OfferView::neutral()
        };
        let cfg = AppraisalConfig::default();
        let a = PlayerOfferAppraisal::appraise(&staged, &loan_home, 0.0, &cfg);
        let b = PlayerOfferAppraisal::appraise(&live, &loan_home, 0.0, &cfg);
        assert!(
            (a.utility - b.utility).abs() < 0.05,
            "staged {:.3} vs live {:.3}",
            a.utility,
            b.utility
        );
        assert!(a.accepts(), "he goes home: {}", a.explain());
    }

    /// B14 — one reading of "listed", type-matched to the deal. A
    /// loan-listed man is advertised for a LOAN, not for a sale.
    #[test]
    fn a_listing_supports_the_kind_of_deal_it_advertises() {
        let cfg = AppraisalConfig::default();
        let _ = cfg;
        let summary = brazilian_at_arsenal();
        // The summary path reads the same rule.
        let stance =
            PlayerStanceBuilder::from_summary(&summary, 0.35, ScoutingRegion::WesternEurope);
        assert!(
            stance.listed_by_club,
            "he is loan-listed and this is a loan"
        );
    }
}
