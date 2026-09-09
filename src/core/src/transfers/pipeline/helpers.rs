use crate::club::team::squad::SquadEvidenceContext;
use crate::transfers::scouting::config::ScoutingConfig;
use chrono::{Datelike, NaiveDate};
use rustc_hash::FxHashMap;

use crate::club::player::events::transfer_social::TransferInterestSignal;
use crate::club::player::language::LanguageProfile;
use crate::club::player::mind::GoalKind;
use crate::club::player::statistics::StuckCareerScan;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::ScoutingRegion;
use crate::transfers::deal::reason::{ScoutVerdict, TransferReason};
use crate::transfers::loan::home::HomeLoanGates;
use crate::transfers::market::window::TransferCalendar;
use crate::transfers::pipeline::LoanDestinationPreference;
use crate::transfers::pipeline::processor::{
    PipelineProcessor, PlayerSummary, SellerPlausibilityContext,
};
use crate::transfers::pipeline::{DetailedScoutingReport, ReportRiskFlag, TransferRequest};
use crate::transfers::scouting::breakout::{BreakoutPerformanceSignal, LeaguePerformanceLookup};
use crate::transfers::squad::standing::CareerRecordSnapshot;
use crate::transfers::value::PlayerValuationCalculator;
use crate::utils::FormattingUtils;
use crate::{
    Club, Country, Person, Player, PlayerFieldPositionGroup, PlayerSquadStatus, PlayerStatusType,
    PositionCoverage, ReputationLevel, StaffPosition, Team, TeamType, TransferInterestSource,
    TransferInterestStage,
};
use chrono::Weekday;

impl PipelineProcessor {
    /// European-calendar fallback for the mid-season window bias. Kept
    /// only for the pure decision fns that carry no country context
    /// (`resolve_initial_approach`); every country-holding caller uses
    /// [`Self::is_mid_season_window_for`] instead.
    pub(in crate::transfers) fn is_january_window(date: NaiveDate) -> bool {
        date.month() == 1
    }

    /// The date falls inside the COUNTRY's shorter (mid-season
    /// reinforcement) window — January in Europe, July for MLS-style
    /// calendars, June for Latam. Drives the "prefer loans, quick
    /// fixes" mid-season bias, which the raw month==1 check applied to
    /// the wrong month everywhere outside Europe.
    pub(in crate::transfers) fn is_mid_season_window_for(
        country: &Country,
        date: NaiveDate,
    ) -> bool {
        let w = TransferCalendar::for_country(&country.code, date);
        let (s_start, s_end) = w.summer_window;
        let (w_start, w_end) = w.winter_window;
        let summer_len = (s_end - s_start).num_days();
        let winter_len = (w_end - w_start).num_days();
        let in_summer = date >= s_start && date <= s_end;
        let in_winter = date >= w_start && date <= w_end;
        if summer_len >= winter_len {
            in_winter
        } else {
            in_summer
        }
    }

    /// Full plan reset at the opening of each of the COUNTRY's transfer
    /// windows — the opening day and the day before it, mirroring the
    /// old May 31 + June 1 double for the European calendar. The
    /// hard-coded European triple reset Latam plans MID-window (their
    /// Dec–Jan window spans New Year, so Jan 1 wiped requests,
    /// shortlists, and the spent/reserved ledger halfway through their
    /// market) and never reset MLS-style calendars at their real
    /// openings at all.
    pub(in crate::transfers) fn is_window_start_for(country: &Country, date: NaiveDate) -> bool {
        let tomorrow = date
            .checked_add_signed(chrono::Duration::days(1))
            .unwrap_or(date);
        // Anchor the calendar on both dates: a window opening Jan 1
        // belongs to next year's anchor when today is Dec 31.
        let today_cal = TransferCalendar::for_country(&country.code, date);
        let tomorrow_cal = TransferCalendar::for_country(&country.code, tomorrow);
        [
            today_cal.summer_window.0,
            today_cal.winter_window.0,
            tomorrow_cal.summer_window.0,
            tomorrow_cal.winter_window.0,
        ]
        .into_iter()
        .any(|start| date == start || tomorrow == start)
    }

    /// Re-evaluate during the COUNTRY's transfer windows.
    /// Daily during the first week of each window for fast pipeline
    /// startup, then weekly (Monday) for the rest of the window. The
    /// old hard-coded June/January cadence starved every non-European
    /// calendar: an MLS-style Feb–Apr window got no evaluation ticks
    /// (and no staff recommendations) for its entire duration.
    pub(in crate::transfers) fn should_evaluate_for(country: &Country, date: NaiveDate) -> bool {
        let w = TransferCalendar::for_country(&country.code, date);
        for (start, end) in [w.summer_window, w.winter_window] {
            if date >= start && date <= end {
                let days_in = (date - start).num_days();
                return days_in < 7 || date.weekday() == Weekday::Mon;
            }
        }
        false
    }

    /// Build the localisable transfer reason from the transfer request
    /// and optional scout report. Both halves travel as data — the motive
    /// as an i18n key, the verdict as the numbers the scout filed — so the
    /// history row can be phrased in whatever language the reader picked.
    pub(in crate::transfers) fn build_transfer_reason(
        request: Option<&TransferRequest>,
        report: Option<&DetailedScoutingReport>,
    ) -> TransferReason {
        let key = request
            .map(|r| r.reason.as_signing_reason_key())
            .unwrap_or_default();

        let scout = report.map(|r| ScoutVerdict {
            recommendation: r.recommendation.clone(),
            assessed_ability: r.assessed_ability,
            assessed_potential: r.assessed_potential,
            confidence: r.confidence,
        });

        TransferReason::key(key).with_scout(scout)
    }

    pub(in crate::transfers) fn find_player_in_country<'a>(
        country: &'a Country,
        player_id: u32,
    ) -> Option<&'a Player> {
        for club in &country.clubs {
            for team in &club.teams.teams {
                if let Some(player) = team.players.find(player_id) {
                    return Some(player);
                }
            }
        }
        None
    }

    /// Build the structured interest signal for a DOMESTIC rumour-tier
    /// beat: `buyer_club_id` (this country) is sniffing around a player
    /// at another club in the same country. Pre-bid stages only — the
    /// negotiation resolvers own the concrete stages with their richer
    /// `NegotiationData` context. Returns `None` when either club can't
    /// be resolved or the "target" already plays for the buyer.
    pub(crate) fn local_interest_signal(
        country: &Country,
        buyer_club_id: u32,
        player_id: u32,
        stage: TransferInterestStage,
        source: TransferInterestSource,
    ) -> Option<TransferInterestSignal> {
        let (seller_club, player) = country.clubs.iter().find_map(|c| {
            c.teams
                .teams
                .iter()
                .find_map(|t| t.players.find(player_id))
                .map(|p| (c, p))
        })?;
        if seller_club.id == buyer_club_id {
            return None;
        }
        let buyer_club = country.clubs.iter().find(|c| c.id == buyer_club_id)?;

        let rep01 = |club: &Club| -> f32 {
            club.teams.main().map(|t| t.reputation.world).unwrap_or(0) as f32 / 10_000.0
        };
        let league_rep = |club: &Club| -> u16 {
            club.teams
                .teams
                .first()
                .and_then(|t| t.league_id)
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| l.reputation)
                .unwrap_or(0)
        };

        Some(TransferInterestSignal {
            interested_club_id: buyer_club_id,
            interested_league_id: buyer_club.teams.teams.first().and_then(|t| t.league_id),
            buyer_rep: rep01(buyer_club),
            seller_rep: rep01(seller_club),
            buyer_league_rep: league_rep(buyer_club),
            seller_league_rep: league_rep(seller_club),
            stage,
            source,
            repeated_attention: false,
            is_rival: seller_club.is_rival(buyer_club_id),
            is_home_country: player.country_id == country.id,
            is_seller_in_home_country: player.country_id == country.id,
            is_former_club: player
                .sold_from
                .as_ref()
                .map(|(cid, _)| *cid == buyer_club_id)
                .unwrap_or(false),
            buyer_country_id: country.id,
            buyer_continent_id: country.continent_id,
            // Rumour-tier beats stay narratively modest — the continental
            // opportunity framing belongs to concrete approaches.
            buyer_has_continental_path: false,
            buyer_competition_path: None,
        })
    }

    /// Resolve player full name and selling club name from the country data.
    pub(in crate::transfers) fn resolve_player_and_club_name(
        country: &Country,
        player_id: u32,
        club_id: u32,
    ) -> (String, String) {
        let player_name = country
            .clubs
            .iter()
            .flat_map(|c| c.teams.iter())
            .find_map(|t| t.players.find(player_id))
            .map(|p| p.full_name.to_string())
            .unwrap_or_default();

        let club_name = country
            .clubs
            .iter()
            .find(|c| c.id == club_id)
            .map(|c| c.name.clone())
            .unwrap_or_default();

        (player_name, club_name)
    }

    pub(in crate::transfers) fn find_player_in_club<'a>(
        club: &'a Club,
        player_id: u32,
    ) -> Option<&'a Player> {
        for team in &club.teams.teams {
            if let Some(player) = team.players.find(player_id) {
                return Some(player);
            }
        }
        None
    }

    pub(in crate::transfers) fn find_player_summary_in_country(
        country: &Country,
        player_id: u32,
        date: NaiveDate,
    ) -> Option<PlayerSummary> {
        for club in &country.clubs {
            for team in &club.teams.teams {
                if let Some(player) = team.players.find(player_id) {
                    return Some(Self::build_player_summary(country, club, player, date));
                }
            }
        }
        None
    }

    /// Build the market summary for a player already resolved to his club.
    /// Shared by the country-wide scan above, the per-pass
    /// [`CountryPlayerLookup`] fast path, and callers that walk rosters
    /// directly (breakout watch) and therefore never need the scan.
    pub(in crate::transfers) fn build_player_summary(
        country: &Country,
        club: &Club,
        player: &Player,
        date: NaiveDate,
    ) -> PlayerSummary {
        Self::build_player_summary_ranked(country, club, player, date, None)
    }

    /// [`Self::build_player_summary`] with an optional precomputed
    /// [`ClubGroupRanks`] for the club. The per-call fallback re-sorts
    /// the main team's position group TWICE per summary (rank + best);
    /// passes that resolve many candidates per club (shortlists,
    /// recommendation re-checks) hand the batched snapshot in instead —
    /// same values by construction (see `ClubGroupRanks`).
    pub(in crate::transfers) fn build_player_summary_ranked(
        country: &Country,
        club: &Club,
        player: &Player,
        date: NaiveDate,
        ranks: Option<&ClubGroupRanks>,
    ) -> PlayerSummary {
        let skill_ability = Self::position_evaluation_ability(player);
        // Blended reputation, not just `world` — keeps domestic
        // strength visible in valuation for clubs whose home
        // standing exceeds their international footprint.
        let (league_reputation, club_reputation) =
            PlayerValuationCalculator::seller_context(country, club);
        let estimated_value = PlayerValuationCalculator::calculate_value_with_price_level(
            player,
            date,
            country.settings.pricing.price_level,
            league_reputation,
            club_reputation,
        )
        .amount;
        let (contract_months_remaining, salary) = player
            .contract
            .as_ref()
            .map(|c| {
                let days = (c.expiration - date).num_days().max(0);
                ((days / 30).min(i16::MAX as i64) as i16, c.salary)
            })
            .unwrap_or((0, 0));
        let pos_group = player.position().position_group();
        let main_team = club.teams.main();
        let seller_ctx = SellerPlausibilityContext {
            club_reputation_score: main_team
                .map(|t| t.reputation.overall_score())
                .unwrap_or(0.3),
            league_reputation,
            league_id: main_team.and_then(|t| t.league_id),
            // Not in the first team's depth chart at all ⇒ ranked BEHIND
            // it, never folded into it at rank 1. See the matching note in
            // `collect_player_pool`.
            position_group_rank: match ranks
                .map(|r| r.rank(player.id))
                .unwrap_or_else(|| Self::position_group_rank(club, player.id, pos_group))
            {
                u8::MAX => main_team
                    .map(|t| {
                        t.players
                            .players
                            .iter()
                            .filter(|p| p.position().position_group() == pos_group)
                            .count()
                            .min(u8::MAX as usize) as u8
                    })
                    .unwrap_or(0)
                    .saturating_add(1),
                r => r,
            },
            squad_status: player
                .contract
                .as_ref()
                .map(|c| {
                    c.squad_status
                        .as_first_team_designation(club.teams.squad_tier_of(player.id))
                })
                .unwrap_or(PlayerSquadStatus::NotYetSet),
            is_transfer_requested: player.statuses.has(PlayerStatusType::Req),
            is_unhappy: player.statuses.has(PlayerStatusType::Unh),
            in_debt: club.finance.balance.balance < 0,
            days_on_market: player.days_available(date).min(i16::MAX as i64) as i16,
            market_resignation: player.market_resignation(date),
            club_matches_played: SquadEvidenceContext::current_season_sample(date, club)
                .club_matches_proxy(),
            big_stage_inclination: player.big_stage_inclination,
            is_marketed: club.transfer_plan.is_marketed(player.id),
        };
        PlayerSummary {
            player_id: player.id,
            club_id: club.id,
            country_id: country.id,
            continent_id: country.continent_id,
            region: ScoutingRegion::from_country(country.continent_id, &country.code),
            country_code: country.code.clone(),
            // Where he is FROM — see the twin block in
            // `collect_player_pool`. Both builders must agree, or a
            // candidate row and a pool row would describe two players.
            nationality_country_id: player.country_id,
            nationality_continent_id: player.nationality_continent_id,
            nationality_region: player.home_region(),
            starter_share: player.happiness.starter_ratio,
            tenure_days: StuckCareerScan::club_tenure_days(player, date)
                .unwrap_or(i64::from(u16::MAX))
                .clamp(0, i64::from(u16::MAX)) as u16,
            // The weekly cache, exactly as `collect_player_pool` reads it
            // — the two builders describe the same man or they describe
            // two different ones.
            return_home_desire: player.home_pull.desire,
            home_return_wanted: HomeLoanGates::is_posted(
                player.home_pull.wanted,
                club.transfer_plan.loan_out_candidates.iter().any(|c| {
                    c.player_id == player.id
                        && c.preferred_destination != LoanDestinationPreference::Any
                }),
            ),
            ambition: player.attributes.ambition as u8,
            loyalty: player.attributes.loyalty as u8,
            adaptability: player.attributes.adaptability as u8,
            leave_pressure: player
                .mind
                .pressure_of(GoalKind::GoOutOnLoan)
                .max(player.mind.pressure_of(GoalKind::LeaveThisClub))
                .max(player.mind.pressure_of(GoalKind::PlayFirstTeamFootball))
                .clamp(0.0, 1.0),
            stay_pressure: player
                .mind
                .pressure_of(GoalKind::StayAtThisClub)
                .max(player.mind.pressure_of(GoalKind::BecomeAClubLegend))
                .clamp(0.0, 1.0),
            player_name: player.full_name.to_string(),
            club_name: club.name.clone(),
            position: player.position(),
            position_group: player.position().position_group(),
            coverage: PositionCoverage::of(&player.positions),
            age: player.age(date),
            estimated_value,
            is_listed: player.statuses.has(PlayerStatusType::Lst),
            is_loan_listed: player.statuses.has(PlayerStatusType::Loa),
            skill_ability,
            // Sample-size-regressed: this candidate row
            // feeds the same scouting recommendation tier
            // logic as the scouting pipeline above.
            average_rating: player
                .statistics
                .average_rating_realistic(player.position().position_group()),
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
            club_world_reputation: Self::club_world_reputation(club),
            club_best_in_group: ranks
                .map(|r| r.best(player.position().position_group()))
                .unwrap_or_else(|| {
                    Self::best_ca_in_group(club, player.position().position_group())
                }),
            is_injured: player.player_attributes.is_injured,
            contract_months_remaining,
            salary,
            seller_ctx,
            language_profile: LanguageProfile::from_languages(&player.languages),
            international_apps: player.player_attributes.international_apps,
            career_record: CareerRecordSnapshot::read(player, pos_group),
        }
    }

    /// Derive risk flags for a scouted player from their observable signals.
    /// Buyer rep is passed in so we can flag wage demands that blow the budget.
    /// Thresholds (determination floor, age cutoff, contract-month window,
    /// rep gap) live in `ScoutingConfig::risk_flags`.
    pub(in crate::transfers) fn evaluate_risk_flags(
        is_injured: bool,
        determination: f32,
        age: u8,
        contract_months_remaining: i16,
        player_world_rep: i16,
        buyer_world_rep: i16,
    ) -> Vec<ReportRiskFlag> {
        ScoutingConfig::default().risk_flags_for(
            is_injured,
            determination,
            age,
            contract_months_remaining,
            player_world_rep,
            buyer_world_rep,
        )
    }

    pub(in crate::transfers) fn club_world_reputation(club: &Club) -> i16 {
        club.teams
            .iter()
            .find(|t| matches!(t.team_type, TeamType::Main))
            .map(|t| t.reputation.world as i16)
            .unwrap_or(0)
    }

    /// Squads whose players are part of the FIRST TEAM's own depth chart:
    /// the main squad plus the club's age-restricted development sides.
    ///
    /// Senior reserves (B / Second / Reserve) are deliberately excluded —
    /// they are branded senior sides that own their own hierarchy, and
    /// folding their regulars into the first team's ranking would tell the
    /// market a B-team striker is competing for the first team's shirt.
    /// The youth squads are the opposite case: a boy registered there is
    /// registered there BY the first team, and where his ability puts him
    /// in the club's depth chart is exactly the fact every rank-driven
    /// reading needs. Reading the main roster alone is what let a
    /// first-team-calibre teenager score `seller_position_rank = unknown`
    /// (importance 0.62 at best), which is the number the loan gates then
    /// judged him unimportant on.
    pub(in crate::transfers) fn ranks_with_first_team(team_type: TeamType) -> bool {
        matches!(team_type, TeamType::Main) || team_type.is_youth()
    }

    /// 0-indexed position-group rank of a player within their club's
    /// first-team depth chart (main squad plus development squads),
    /// ordered by current ability (descending). Used by the
    /// plausibility layer — first-choice GKs are protected by rank in
    /// a way average + status alone don't capture.
    /// Returns `u8::MAX` when the player is not on any of those squads —
    /// callers should treat that as "unknown" and avoid using the
    /// rank-driven importance bump.
    pub(crate) fn position_group_rank(
        club: &Club,
        player_id: u32,
        group: PlayerFieldPositionGroup,
    ) -> u8 {
        let mut peers: Vec<(u32, u8)> = club
            .teams
            .iter()
            .filter(|t| Self::ranks_with_first_team(t.team_type))
            .flat_map(|t| t.players.players.iter())
            .filter(|p| p.position().position_group() == group)
            .map(|p| (p.id, p.player_attributes.current_ability))
            .collect();
        if peers.is_empty() {
            return u8::MAX;
        }
        peers.sort_by(|a, b| b.1.cmp(&a.1));
        peers
            .iter()
            .position(|(pid, _)| *pid == player_id)
            .map(|idx| idx.min(u8::MAX as usize - 1) as u8)
            .unwrap_or(u8::MAX)
    }

    /// Best CA at the given position group across the club's first-team
    /// depth chart — see [`Self::ranks_with_first_team`].
    pub(crate) fn best_ca_in_group(club: &Club, group: PlayerFieldPositionGroup) -> u8 {
        club.teams
            .iter()
            .filter(|t| Self::ranks_with_first_team(t.team_type))
            .flat_map(|t| t.players.players.iter())
            .filter(|p| p.position().position_group() == group)
            .map(|p| p.player_attributes.current_ability)
            .max()
            .unwrap_or(0)
    }

    /// Best `judging_player_data` across the club's scouting staff.
    /// Drives how aggressively the data department narrows the scout pool.
    /// Defaults from `ScoutingConfig::data_prefilter::default_data_skill`
    /// when the club has no scouts at all.
    pub(in crate::transfers) fn club_data_analysis_skill(club: &Club) -> u8 {
        let default_skill = ScoutingConfig::default().data_prefilter.default_data_skill;
        club.teams
            .iter()
            .flat_map(|t| t.staffs.iter())
            .filter(|s| {
                s.contract
                    .as_ref()
                    .map(
                        |c| matches!(c.position, StaffPosition::Scout | StaffPosition::ChiefScout,),
                    )
                    .unwrap_or(false)
            })
            .map(|s| s.staff_attributes.data_analysis.judging_player_data)
            .max()
            .unwrap_or(default_skill)
    }

    /// Performance-adjusted data score used as a pre-scouting filter.
    /// Weights ability, form (rating × appearances), raw output (G+A), and
    /// the performance-breakout signal — so a high-output player whose
    /// *results* outrun his level rises up the data department's shortlist
    /// and actually gets watched, instead of being buried behind
    /// higher-ability names. The breakout term is league-reputation
    /// discounted inside the signal, so a flat-track scorer in a weak
    /// division doesn't leapfrog proven quality.
    pub(in crate::transfers) fn player_data_score(
        p: &PlayerSummary,
        perf: &LeaguePerformanceLookup,
    ) -> f32 {
        let ability = p.skill_ability as f32 * 0.4;
        let form = p.average_rating * (p.appearances.min(40) as f32 / 4.0);
        let output = ((p.goals + p.assists).min(30)) as f32 * 0.3;
        let breakout = BreakoutPerformanceSignal::compute(&perf.breakout_inputs(
            p.player_id,
            p.position_group,
            p.goals,
            p.assists,
            p.appearances,
            p.average_rating,
            p.age,
            p.seller_ctx.league_reputation,
        ));
        ability + form + output + breakout.score * 0.2
    }

    pub(in crate::transfers) fn get_scout_skills(club: &Club, scout_id: u32) -> (u8, u8) {
        for team in &club.teams.teams {
            if let Some(staff) = team.staffs.find(scout_id) {
                return (
                    staff.staff_attributes.knowledge.judging_player_ability,
                    staff.staff_attributes.knowledge.judging_player_potential,
                );
            }
        }
        // Pointer is stale (staff was removed mid-tick or assignment was
        // never tied to a real scout). Use the configured "missing staff"
        // defaults rather than panic — quality silently downgrades.
        let cfg = ScoutingConfig::default();
        (
            cfg.observation.default_judging_when_staff_missing,
            cfg.observation.default_judging_when_staff_missing,
        )
    }

    /// Estimate a player's growth potential from observable attributes.
    /// Scouts can't see PA — they judge ceiling from age, character, and current skill level.
    /// Young players with strong determination, work rate, composure show higher ceiling.
    pub(in crate::transfers) fn estimate_growth_potential(
        age: u8,
        determination: f32,
        work_rate: f32,
        composure: f32,
        anticipation: f32,
        current_skill_ability: u8,
    ) -> u8 {
        // Mental quality score: how much this player's character suggests growth (0.0-1.0)
        let mental_quality =
            ((determination + work_rate + composure + anticipation) / 4.0 - 1.0) / 19.0;
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

    pub(in crate::transfers) fn calculate_asking_price(
        player: &Player,
        country: &Country,
        club: &Club,
        date: NaiveDate,
        price_level: f32,
    ) -> CurrencyValue {
        // The club's own ledger price, when its monthly pass has reached
        // him. That number knows three things this function cannot: what he
        // is to THIS side (a core player costs twice what his market value
        // says, because the club does not want to sell him), how long the
        // club still controls him, and where he sits on his own career arc.
        // See [`AssetLedger::asking_for`]. `SellerFeeFloor` is still the
        // absolute floor underneath whatever comes out.
        if let Some(asking) = club.transfer_plan.asking_for(player.id) {
            if asking > 0.0 {
                return CurrencyValue {
                    amount: FormattingUtils::round_fee(asking),
                    currency: Currency::Usd,
                };
            }
        }
        // Selling clubs anchor on their own market context — a Serie A
        // club asking the same fee as a Maltese side for an identical
        // player is an obvious flatness bug. Pull the seller's blended
        // league + club reputation so the base value reflects who is
        // actually selling.
        let (league_rep, club_rep) = PlayerValuationCalculator::seller_context(country, club);
        let base_value = PlayerValuationCalculator::calculate_value_with_price_level(
            player,
            date,
            price_level,
            league_rep,
            club_rep,
        );

        let multiplier =
            PlayerValuationCalculator::seller_distress_multiplier(club.finance.balance.balance);

        CurrencyValue {
            amount: FormattingUtils::round_fee(base_value.amount * multiplier),
            currency: base_value.currency,
        }
    }

    pub(in crate::transfers) fn get_club_reputation(country: &Country, club_id: u32) -> f32 {
        country
            .clubs
            .iter()
            .find(|c| c.id == club_id)
            .and_then(|c| c.teams.teams.first())
            .map(|t| t.reputation.attractiveness_factor())
            .unwrap_or(0.3)
    }

    pub(in crate::transfers) fn get_club_reputation_level(
        country: &Country,
        club_id: u32,
    ) -> ReputationLevel {
        country
            .clubs
            .iter()
            .find(|c| c.id == club_id)
            .and_then(|c| c.teams.teams.first())
            .map(|t| t.reputation.level())
            .unwrap_or(ReputationLevel::Amateur)
    }

    pub(in crate::transfers) fn get_player_negotiation_data(
        country: &Country,
        player_id: u32,
        date: NaiveDate,
    ) -> (u8, f32) {
        Self::find_player_in_country(country, player_id)
            .map(|p| (p.age(date), p.attributes.ambition))
            .unwrap_or((25, 0.5))
    }

    pub(in crate::transfers) fn rep_level_value(level: &ReputationLevel) -> u8 {
        match level {
            ReputationLevel::Elite => 5,
            ReputationLevel::Continental => 4,
            ReputationLevel::National => 3,
            ReputationLevel::Regional => 2,
            ReputationLevel::Local => 1,
            ReputationLevel::Amateur => 0,
        }
    }

    /// Canonical "evaluate this player against a tier baseline" ability.
    /// Returns the position-weighted ability (1..200 scale) used
    /// throughout the transfer pipeline — squad evaluation, scout
    /// reach, listed-star sweeps. Both `current_ability` and this
    /// helper share the 1..200 scale and are kept in sync by the
    /// training and development paths
    /// (`development/tick.rs:190`, `training/result.rs:88`), which
    /// always recompute CA from `calculate_ability_for_position`.
    /// Naming the helper so call-sites can't accidentally mix it up
    /// with raw skill averages prevents the CA-vs-skill drift the
    /// audit flagged.
    pub(crate) fn position_evaluation_ability(player: &Player) -> u8 {
        player
            .skills
            .calculate_ability_for_position(player.position())
    }

    /// Linear-interpolated lookup of base baseline CA from a continuous
    /// reputation score. Anchors are calibrated so that the midpoint of
    /// each enum tier reproduces the bucketed baseline the rest of the
    /// pipeline expects. Score is `Reputation::overall_score()` (0..1).
    fn baseline_anchor_curve(score: f32) -> f32 {
        const ANCHORS: [(f32, f32); 7] = [
            (0.000, 50.0),
            (0.075, 55.0),
            (0.225, 70.0),
            (0.400, 88.0),
            (0.575, 110.0),
            (0.725, 130.0),
            (0.900, 145.0),
        ];
        let s = score.clamp(0.0, 1.0);
        // Above the top anchor we keep climbing — top-of-Elite (e.g. a
        // generational Real Madrid side) demands more than mid-Elite.
        if s >= ANCHORS[ANCHORS.len() - 1].0 {
            let (s_top, b_top) = ANCHORS[ANCHORS.len() - 1];
            let extrapolation = (s - s_top) * (162.0 - b_top) / (1.0 - s_top).max(1e-6);
            return b_top + extrapolation;
        }
        for window in ANCHORS.windows(2) {
            let (s0, b0) = window[0];
            let (s1, b1) = window[1];
            if s >= s0 && s <= s1 {
                let t = (s - s0) / (s1 - s0).max(1e-6);
                return b0 + (b1 - b0) * t;
            }
        }
        ANCHORS[0].1
    }

    /// Linear-interpolated headroom (max CA above baseline a club can
    /// realistically pursue). Anchored at the same enum-tier midpoints
    /// as [`baseline_anchor_curve`].
    fn headroom_anchor_curve(score: f32) -> f32 {
        const ANCHORS: [(f32, f32); 7] = [
            (0.000, 6.0),
            (0.075, 8.0),
            (0.225, 10.0),
            (0.400, 14.0),
            (0.575, 22.0),
            (0.725, 35.0),
            (0.900, 55.0),
        ];
        let s = score.clamp(0.0, 1.0);
        if s >= ANCHORS[ANCHORS.len() - 1].0 {
            let (s_top, h_top) = ANCHORS[ANCHORS.len() - 1];
            let extrapolation = (s - s_top) * (65.0 - h_top) / (1.0 - s_top).max(1e-6);
            return h_top + extrapolation;
        }
        for window in ANCHORS.windows(2) {
            let (s0, h0) = window[0];
            let (s1, h1) = window[1];
            if s >= s0 && s <= s1 {
                let t = (s - s0) / (s1 - s0).max(1e-6);
                return h0 + (h1 - h0) * t;
            }
        }
        ANCHORS[0].1
    }

    /// Per-group offset applied on top of the base baseline. Goalkeepers
    /// naturally score lower on the unified CA scale (fewer outfield-
    /// style attributes feed the rating); forwards a touch higher. Kept
    /// here as the single source of truth — neither evaluation nor
    /// recommendations carries its own per-group adjustment.
    fn group_baseline_offset(group: PlayerFieldPositionGroup) -> i16 {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => -8,
            PlayerFieldPositionGroup::Defender => -3,
            PlayerFieldPositionGroup::Midfielder => 0,
            PlayerFieldPositionGroup::Forward => 2,
        }
    }

    /// Expected current-ability of an at-tier starter for a club whose
    /// reputation `overall_score` is `score` (0..1). The continuous
    /// version of [`tier_starter_ca`] — a club mid-Continental gets a
    /// different baseline from a top-of-Continental club, instead of
    /// snapping to the same enum bucket. Position offsets are applied
    /// uniformly via [`group_baseline_offset`].
    pub(crate) fn tier_starter_ca_score(score: f32, group: PlayerFieldPositionGroup) -> u8 {
        let base = Self::baseline_anchor_curve(score);
        let offset = Self::group_baseline_offset(group);
        (base.round() as i16 + offset).clamp(20, 200) as u8
    }

    /// Continuous-score counterpart of [`tier_target_ceiling`].
    pub(crate) fn tier_target_ceiling_score(score: f32, group: PlayerFieldPositionGroup) -> u8 {
        let baseline = Self::tier_starter_ca_score(score, group);
        let headroom = Self::headroom_anchor_curve(score).round() as i16;
        (baseline as i16 + headroom).clamp(20, 200) as u8
    }

    /// Continuous-score counterpart of [`tier_quality_tolerance`]. Top
    /// clubs upgrade aggressively (small tolerance); small clubs
    /// patient. Linear in 1 - score so that going up the reputation
    /// ladder reduces tolerance smoothly, no enum cliff.
    pub(crate) fn tier_quality_tolerance_score(score: f32) -> i16 {
        let s = score.clamp(0.0, 1.0);
        let raw = 4.0 + (1.0 - s) * 11.0; // 4 at top, 15 at the bottom
        raw.round() as i16
    }
}

/// Per-pass `player_id → club` index for
/// [`PipelineProcessor::find_player_summary_in_country`]-style lookups.
///
/// The scan walks every club/team in the country PER CANDIDATE, which made
/// it the single hottest leaf of the transfer pipeline (shortlist builds,
/// recommendation plausibility re-checks). Passes that resolve many
/// candidates build this once, then each lookup is a hash probe + a
/// verified fetch. The hit is verified against the club's roster (mirrors
/// `resolve_foreign_player_club`'s stale-index guard) and any miss falls
/// back to the authoritative scan, so results are identical to the scan —
/// the index is built at pass start and rosters don't move mid-pass, the
/// fallback is a pure safety net.
pub(in crate::transfers) struct CountryPlayerLookup {
    club_idx_by_player: FxHashMap<u32, u32>,
    /// Per-club [`ClubGroupRanks`], indexed like `Country::clubs`.
    /// Pre-built for every club (one group sort per club — trivial next
    /// to the scans this index serves) so `find_summary` stays `&self`
    /// and the lookup can be shared across parallel per-club scans.
    /// Rosters and CA don't move inside a pass — same freshness
    /// contract as `club_idx_by_player`.
    ranks_by_club: Vec<ClubGroupRanks>,
}

impl CountryPlayerLookup {
    pub(in crate::transfers) fn build(country: &Country) -> Self {
        let mut club_idx_by_player = FxHashMap::default();
        let mut ranks_by_club = Vec::with_capacity(country.clubs.len());
        for (club_idx, club) in country.clubs.iter().enumerate() {
            for team in &club.teams.teams {
                for player in &team.players.players {
                    club_idx_by_player.insert(player.id, club_idx as u32);
                }
            }
            ranks_by_club.push(ClubGroupRanks::build(club));
        }
        CountryPlayerLookup {
            club_idx_by_player,
            ranks_by_club,
        }
    }

    pub(in crate::transfers) fn find_summary(
        &self,
        country: &Country,
        player_id: u32,
        date: NaiveDate,
    ) -> Option<PlayerSummary> {
        if let Some(&club_idx) = self.club_idx_by_player.get(&player_id) {
            if let Some(club) = country.clubs.get(club_idx as usize) {
                if let Some(player) = PipelineProcessor::find_player_in_club(club, player_id) {
                    return Some(PipelineProcessor::build_player_summary_ranked(
                        country,
                        club,
                        player,
                        date,
                        self.ranks_by_club.get(club_idx as usize),
                    ));
                }
            }
        }
        PipelineProcessor::find_player_summary_in_country(country, player_id, date)
    }

    /// Indexed counterpart of [`PipelineProcessor::find_player_in_country`],
    /// with the same verified-hit / scan-fallback contract as
    /// [`Self::find_summary`].
    pub(in crate::transfers) fn find_player<'a>(
        &self,
        country: &'a Country,
        player_id: u32,
    ) -> Option<&'a Player> {
        if let Some(&club_idx) = self.club_idx_by_player.get(&player_id) {
            if let Some(club) = country.clubs.get(club_idx as usize) {
                if let Some(player) = PipelineProcessor::find_player_in_club(club, player_id) {
                    return Some(player);
                }
            }
        }
        PipelineProcessor::find_player_in_country(country, player_id)
    }
}

/// Per-club snapshot of the main team's position-group CA order —
/// the batched counterpart of [`PipelineProcessor::position_group_rank`] and
/// [`PipelineProcessor::best_ca_in_group`]. The pool build used to call
/// both PER PLAYER, and each call re-collected and re-sorted the whole
/// group (`O(squad²·log)` per club per tick). One build serves every player
/// of the club. Same values by construction: identical comparator over the
/// identical roster sequence (stable sort keeps roster order on CA ties),
/// `u8::MAX` for anyone not on the main team, `0` best for a missing group
/// or missing main team.
pub(in crate::transfers) struct ClubGroupRanks {
    rank_by_player: FxHashMap<u32, u8>,
    best_by_group: [u8; PlayerFieldPositionGroup::COUNT],
    size_by_group: [u8; PlayerFieldPositionGroup::COUNT],
}

impl ClubGroupRanks {
    pub(in crate::transfers) fn build(club: &Club) -> Self {
        let mut rank_by_player = FxHashMap::default();
        let mut best_by_group = [0u8; PlayerFieldPositionGroup::COUNT];
        let mut size_by_group = [0u8; PlayerFieldPositionGroup::COUNT];
        // The FIRST TEAM's own depth chart: the main squad plus the club's
        // age-restricted development sides, which the first team registers
        // its own youngsters in. Senior reserves stay out — see
        // [`PipelineProcessor::ranks_with_first_team`], whose contract this
        // mirrors so the pool builder and the live lookup can never
        // disagree about a player's rank.
        let squads: Vec<&Team> = club
            .teams
            .teams
            .iter()
            .filter(|t| PipelineProcessor::ranks_with_first_team(t.team_type))
            .collect();
        if !squads.is_empty() {
            let mut peers_by_group: [Vec<(u32, u8)>; PlayerFieldPositionGroup::COUNT] =
                Default::default();
            for p in squads.iter().flat_map(|t| t.players.players.iter()) {
                let group = p.position().position_group();
                peers_by_group[group.index()].push((p.id, p.player_attributes.current_ability));
            }
            for (group_idx, peers) in peers_by_group.iter_mut().enumerate() {
                peers.sort_by(|a, b| b.1.cmp(&a.1));
                best_by_group[group_idx] = peers.first().map(|(_, ca)| *ca).unwrap_or(0);
                size_by_group[group_idx] = peers.len().min(u8::MAX as usize) as u8;
                for (rank, (pid, _)) in peers.iter().enumerate() {
                    rank_by_player.insert(*pid, rank.min(u8::MAX as usize - 1) as u8);
                }
            }
        }
        ClubGroupRanks {
            rank_by_player,
            best_by_group,
            size_by_group,
        }
    }

    /// Rank of the player inside his main-team position group (0 = best),
    /// `u8::MAX` when he isn't on the main team — same contract as
    /// [`PipelineProcessor::position_group_rank`].
    pub(in crate::transfers) fn rank(&self, player_id: u32) -> u8 {
        self.rank_by_player
            .get(&player_id)
            .copied()
            .unwrap_or(u8::MAX)
    }

    /// Best main-team CA in the group — same contract as
    /// [`PipelineProcessor::best_ca_in_group`].
    pub(in crate::transfers) fn best(&self, group: PlayerFieldPositionGroup) -> u8 {
        self.best_by_group[group.index()]
    }

    /// How many first-team players occupy this position group. Lets a
    /// caller place someone who isn't in the main-team depth chart at all
    /// BEHIND it, rather than folding him into it at an invented rank.
    pub(in crate::transfers) fn group_size(&self, group: PlayerFieldPositionGroup) -> u8 {
        self.size_by_group[group.index()]
    }
}

#[cfg(test)]
mod breakout;
#[cfg(test)]
mod group;
#[cfg(test)]
mod role;
#[cfg(test)]
mod slot;
#[cfg(test)]
mod tier;
