use chrono::{Datelike, NaiveDate};

use crate::shared::CurrencyValue;
use crate::transfers::pipeline::processor::{PipelineProcessor, PlayerSummary};
use crate::transfers::pipeline::{
    DetailedScoutingReport, ScoutingRecommendation, TransferNeedReason, TransferRequest,
};
use crate::transfers::window::PlayerValuationCalculator;
use crate::utils::FormattingUtils;
use crate::{Club, Country, Person, Player, PlayerStatusType, ReputationLevel};

impl PipelineProcessor {
    pub(super) fn is_january_window(date: NaiveDate) -> bool {
        date.month() == 1
    }

    /// Full reset only at transfer window opening dates
    pub(super) fn is_window_start(date: NaiveDate) -> bool {
        let month = date.month();
        let day = date.day();
        (month == 5 && day == 31) || (month == 6 && day == 1) || (month == 1 && day == 1)
    }

    /// Re-evaluate during transfer windows.
    /// Daily during the first week of each window for fast pipeline startup,
    /// then weekly (Monday) for the rest of the window.
    pub(super) fn should_evaluate(date: NaiveDate) -> bool {
        let month = date.month();
        let day = date.day();

        // First week of summer window (June 1-7) or winter window (Jan 1-7): daily
        if (month == 6 && day <= 7) || (month == 1 && day <= 7) {
            return true;
        }

        // Rest of window: weekly on Monday
        ((month >= 6 && month <= 8) || month == 1)
            && date.weekday() == chrono::Weekday::Mon
    }

    pub fn transfer_need_reason_text(reason: &TransferNeedReason) -> &'static str {
        match reason {
            TransferNeedReason::FormationGap => "Formation gap — no player for required position",
            TransferNeedReason::QualityUpgrade => "Quality upgrade — current player below squad level",
            TransferNeedReason::DepthCover => "Squad depth — need backup for position group",
            TransferNeedReason::SuccessionPlanning => "Succession planning — aging key player needs replacement",
            TransferNeedReason::DevelopmentSigning => "Development signing — young prospect with high potential",
            TransferNeedReason::StaffRecommendation => "Staff recommendation",
            TransferNeedReason::LoanToFillSquad => "Loan to fill squad — cannot afford to buy",
            TransferNeedReason::ExperiencedHead => "Experienced head — need senior player for leadership",
            TransferNeedReason::SquadPadding => "Squad padding — too few players to compete",
            TransferNeedReason::CheapReinforcement => "Cheap reinforcement — affordable quality improvement",
            TransferNeedReason::InjuryCoverLoan => "Injury cover — loan to replace injured player",
            TransferNeedReason::OpportunisticLoanUpgrade => "Opportunistic loan — player better than current options",
        }
    }

    /// Build a transfer reason string from the transfer request and optional scout report.
    pub(super) fn build_transfer_reason(
        request: Option<&TransferRequest>,
        report: Option<&DetailedScoutingReport>,
    ) -> String {
        let need_reason = request.map(|r| Self::transfer_need_reason_text(&r.reason));

        let scout_reason = report.map(|r| {
            let rec = match r.recommendation {
                ScoutingRecommendation::StrongBuy => "Strong buy",
                ScoutingRecommendation::Buy => "Buy",
                ScoutingRecommendation::Consider => "Consider",
                ScoutingRecommendation::Pass => "Pass",
            };
            let ability_label = Self::ability_label(r.assessed_ability);
            let potential_label = Self::ability_label(r.assessed_potential);
            format!("Scout: {} (ability: {}, potential: {}, confidence: {:.0}%)",
                rec, ability_label, potential_label, r.confidence * 100.0)
        });

        match (need_reason, scout_reason) {
            (Some(need), Some(scout)) => format!("{} — {}", need, scout),
            (Some(need), None) => need.to_string(),
            (None, Some(scout)) => scout,
            (None, None) => String::new(),
        }
    }

    /// Convert a raw assessed ability value to a qualitative label.
    fn ability_label(value: u8) -> &'static str {
        match value {
            0..=30 => "Very poor",
            31..=60 => "Poor",
            61..=80 => "Below average",
            81..=100 => "Average",
            101..=120 => "Decent",
            121..=140 => "Good",
            141..=160 => "Very good",
            161..=180 => "Excellent",
            181..=200 => "World class",
            _ => "Unknown",
        }
    }

    pub(super) fn find_player_in_country<'a>(country: &'a Country, player_id: u32) -> Option<&'a Player> {
        for club in &country.clubs {
            for team in &club.teams.teams {
                if let Some(player) = team.players.find(player_id) {
                    return Some(player);
                }
            }
        }
        None
    }

    /// Resolve player full name and selling club name from the country data.
    pub(super) fn resolve_player_and_club_name(country: &Country, player_id: u32, club_id: u32) -> (String, String) {
        let player_name = country.clubs.iter()
            .flat_map(|c| c.teams.iter())
            .find_map(|t| t.players.find(player_id))
            .map(|p| p.full_name.to_string())
            .unwrap_or_default();

        let club_name = country.clubs.iter()
            .find(|c| c.id == club_id)
            .map(|c| c.name.clone())
            .unwrap_or_default();

        (player_name, club_name)
    }

    pub(super) fn find_player_in_club<'a>(club: &'a Club, player_id: u32) -> Option<&'a Player> {
        for team in &club.teams.teams {
            if let Some(player) = team.players.find(player_id) {
                return Some(player);
            }
        }
        None
    }

    pub(super) fn find_player_summary_in_country(
        country: &Country,
        player_id: u32,
        date: NaiveDate,
    ) -> Option<PlayerSummary> {
        for club in &country.clubs {
            for team in &club.teams.teams {
                if let Some(player) = team.players.find(player_id) {
                    let skill_ability = player.skills.calculate_ability_for_position(player.position());
                    return Some(PlayerSummary {
                        player_id: player.id,
                        club_id: club.id,
                        country_id: country.id,
                        continent_id: country.continent_id,
                        country_code: country.code.clone(),
                        player_name: player.full_name.to_string(),
                        club_name: club.name.clone(),
                        position: player.position(),
                        position_group: player.position().position_group(),
                        age: player.age(date),
                        estimated_value: skill_ability as f64 * 10000.0,
                        is_listed: player.statuses.get().contains(&PlayerStatusType::Lst),
                        is_loan_listed: player.statuses.get().contains(&PlayerStatusType::Loa),
                        skill_ability,
                        average_rating: player.statistics.average_rating,
                        goals: player.statistics.goals,
                        assists: player.statistics.assists,
                        appearances: player.statistics.total_games(),
                        determination: player.skills.mental.determination,
                        work_rate: player.skills.mental.work_rate,
                        composure: player.skills.mental.composure,
                        anticipation: player.skills.mental.anticipation,
                        technical_avg: player.skills.technical.average(),
                        mental_avg: player.skills.mental.average(),
                        physical_avg: player.skills.physical.average(),
                        current_reputation: player.player_attributes.current_reputation,
                        home_reputation: player.player_attributes.home_reputation,
                        world_reputation: player.player_attributes.world_reputation,
                        country_reputation: country.reputation,
                    });
                }
            }
        }
        None
    }

    pub(super) fn get_scout_skills(club: &Club, scout_id: u32) -> (u8, u8) {
        for team in &club.teams.teams {
            if let Some(staff) = team.staffs.find(scout_id) {
                return (
                    staff.staff_attributes.knowledge.judging_player_ability,
                    staff.staff_attributes.knowledge.judging_player_potential,
                );
            }
        }
        (10, 10)
    }

    /// Estimate a player's growth potential from observable attributes.
    /// Scouts can't see PA — they judge ceiling from age, character, and current skill level.
    /// Young players with strong determination, work rate, composure show higher ceiling.
    pub(super) fn estimate_growth_potential(
        age: u8,
        determination: f32,
        work_rate: f32,
        composure: f32,
        anticipation: f32,
        current_skill_ability: u8,
    ) -> u8 {
        // Mental quality score: how much this player's character suggests growth (0.0-1.0)
        let mental_quality = ((determination + work_rate + composure + anticipation) / 4.0 - 1.0) / 19.0;
        let mental_factor = mental_quality.clamp(0.0, 1.0);

        // Age-based growth window: younger = more room to grow
        let base_growth = match age {
            0..=17 => 35.0,
            18 => 30.0,
            19 => 25.0,
            20 => 20.0,
            21 => 15.0,
            22 => 12.0,
            23 => 8.0,
            24 => 5.0,
            25 => 3.0,
            26..=27 => 1.0,
            _ => 0.0,
        };

        // Players already at high skill level have less room to grow
        let ceiling_factor = if current_skill_ability > 160 {
            0.3
        } else if current_skill_ability > 120 {
            0.6
        } else {
            1.0
        };

        (base_growth * mental_factor * ceiling_factor) as u8
    }

    pub(super) fn calculate_asking_price(
        player: &Player,
        club: &Club,
        date: NaiveDate,
        price_level: f32,
    ) -> CurrencyValue {
        let base_value =
            PlayerValuationCalculator::calculate_value_with_price_level(player, date, price_level, 0, 0);

        let multiplier = if club.finance.balance.balance < 0 {
            0.9
        } else {
            1.1
        };

        CurrencyValue {
            amount: FormattingUtils::round_fee(base_value.amount * multiplier),
            currency: base_value.currency,
        }
    }

    pub(super) fn get_club_reputation(country: &Country, club_id: u32) -> f32 {
        country
            .clubs
            .iter()
            .find(|c| c.id == club_id)
            .and_then(|c| c.teams.teams.first())
            .map(|t| t.reputation.attractiveness_factor())
            .unwrap_or(0.3)
    }

    pub(super) fn get_club_reputation_level(country: &Country, club_id: u32) -> ReputationLevel {
        country
            .clubs
            .iter()
            .find(|c| c.id == club_id)
            .and_then(|c| c.teams.teams.first())
            .map(|t| t.reputation.level())
            .unwrap_or(ReputationLevel::Amateur)
    }

    pub(super) fn get_player_negotiation_data(
        country: &Country,
        player_id: u32,
        date: NaiveDate,
    ) -> (u8, f32) {
        Self::find_player_in_country(country, player_id)
            .map(|p| (p.age(date), p.attributes.ambition))
            .unwrap_or((25, 0.5))
    }

    pub(super) fn rep_level_value(level: &ReputationLevel) -> u8 {
        match level {
            ReputationLevel::Elite => 5,
            ReputationLevel::Continental => 4,
            ReputationLevel::National => 3,
            ReputationLevel::Regional => 2,
            ReputationLevel::Local => 1,
            ReputationLevel::Amateur => 0,
        }
    }
}
