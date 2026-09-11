//! Earning a professional contract, and the evidence that brings the
//! first-team promotion bar down.

use chrono::{Datelike, NaiveDate};
use log::debug;

use crate::club::ClubPhilosophy;
use crate::club::staff::perception::{CoachProfile, DevelopmentFormEvidence};
use crate::{Club, ContractType, Person, Player, PlayerClubContract, PlayerStatusType, TeamType};

use super::super::academy::GraduationTerms;

impl Club {
    /// Weekly: award a first professional contract to youth-team players
    /// whose form has earned it, without waiting for a main-team
    /// promotion.
    ///
    /// Previously a youth contract only became a full one when the player
    /// was good enough to be pulled into the senior squad (see
    /// [`ProfessionalContractPromotion::upgrade`] at the promotion sites).
    /// That meant a standout 17/18-year-old stayed on youth terms — and,
    /// because the
    /// utilization audit skips youth contracts, stayed invisible to the
    /// loan market — until he could already displace a senior. Real clubs
    /// hand a promising academy player pro terms on the back of good
    /// results and *then* send him out on loan to develop. This pass
    /// models that: a youth player with a real run of strong games is
    /// upgraded in place, in his current youth/reserve squad.
    pub(in crate::club::core) fn review_youth_contracts(&mut self, date: NaiveDate) {
        let main_idx = match self.teams.main_index() {
            Some(idx) => idx,
            None => return,
        };
        let club_rep = self.teams.teams[main_idx].reputation.world;

        // Collect first, mutate second: the scan borrows the team
        // collection immutably, so we can't upgrade in the same loop.
        let mut earned: Vec<(usize, u32)> = Vec::new();
        for (ti, team) in self.teams.iter().enumerate() {
            if ti == main_idx {
                continue;
            }
            for player in team.players.iter() {
                if ProfessionalContractPromotion::is_earned(player, date) {
                    earned.push((ti, player.id));
                }
            }
        }

        for (ti, player_id) in earned {
            let team_name = self.teams.teams[ti].name.clone();
            if let Some(player) = self.teams.teams[ti].players.find_mut(player_id) {
                ProfessionalContractPromotion::upgrade(player, date, club_rep);
                // First pro deal — a genuine career milestone for the
                // player, distinct from the senior-debut breakthrough.
                player.on_professional_contract_awarded();
                debug!(
                    "youth → pro contract on merit: {} (CA={}, age={}) at {}",
                    player.full_name,
                    player.player_attributes.current_ability,
                    player.age(date),
                    team_name,
                );
            }
        }
    }
}

/// Evidence-based adjustments to the first-team promotion bar in
/// [`Club::rebalance_squads`]. Real clubs don't wait for a prospect to
/// out-train the worst senior: senior cameos already played, a coach who
/// reads potential well, and a development-first club identity all bring
/// the decision forward. Every input is observable — appearance counters
/// and staff profile, never hidden attributes.
pub(in crate::club::core) struct PromotionEvidence;

impl PromotionEvidence {
    /// Bar discount per senior appearance already made (observable-level
    /// points on the 1..200 scale).
    const PER_SENIOR_APP: u16 = 3;
    /// Cap on the cameo discount — four-plus senior games are a made
    /// case; more cameos shouldn't erode the bar indefinitely.
    const CAMEO_DISCOUNT_CAP: u16 = 12;
    /// Reach of the head coach's potential judgement on the bar.
    const JUDGEMENT_SPAN: f32 = 4.0;
    /// Extra aggressiveness for a develop-and-sell club.
    const DEVELOP_AND_SELL_DISCOUNT: u8 = 4;

    /// Senior appearances a youth-rostered player has already made.
    /// Youth-league fixtures are friendly-flagged and book into the
    /// friendly bucket, so for an age-restricted squad the official
    /// league + cup counters only move when the player turns out for a
    /// senior side — they ARE his senior-cameo record. Senior reserve
    /// squads (B/Second/Reserve) play official football of their own, so
    /// no such read exists for them.
    pub(in crate::club::core) fn cameo_discount(player: &Player, team_type: TeamType) -> u8 {
        if !team_type.is_youth() {
            return 0;
        }
        let senior_apps = player.statistics.played
            + player.statistics.played_subs
            + player.cup_statistics.played
            + player.cup_statistics.played_subs;
        (senior_apps * Self::PER_SENIOR_APP).min(Self::CAMEO_DISCOUNT_CAP) as u8
    }

    /// Club-level bar discount from the head coach's potential judgement
    /// and the club philosophy.
    pub(in crate::club::core) fn club_discount(
        profile: &CoachProfile,
        philosophy: &ClubPhilosophy,
    ) -> u8 {
        let judgement =
            (profile.potential_accuracy.clamp(0.0, 1.0) * Self::JUDGEMENT_SPAN).round() as u8;
        let identity = match philosophy {
            ClubPhilosophy::DevelopAndSell => Self::DEVELOP_AND_SELL_DISCOUNT,
            _ => 0,
        };
        judgement + identity
    }
}

/// Youth → professional contract promotion on merit: deciding whether a
/// youth-contract player has earned pro terms, and performing the upgrade.
/// Both the promotion sites in [`Club::rebalance_squads`] and the weekly
/// [`Club::review_youth_contracts`] pass route through here.
pub(in crate::club::core) struct ProfessionalContractPromotion;

impl ProfessionalContractPromotion {
    /// Meaningful sample — mirrors the "established regular" bar used by
    /// the stalled-prospect pathway.
    const MIN_GAMES: u16 = 8;
    /// Clearly above the ~6.6 positional neutral: a standout youth
    /// season, not merely "featured".
    const MIN_RATING: f32 = 7.0;
    /// Real-world floor for signing professional terms.
    const MIN_AGE: u8 = 16;

    /// Has this youth-contract player earned a first professional
    /// contract? A pure *results* signal, never raw ability or potential:
    /// a real sample of matches at a reliability-adjusted rating clearly
    /// above the positional neutral (~6.6). The realistic average already
    /// regresses small samples toward neutral, so a hot three-game streak
    /// can't trigger it — the player must sustain the form.
    ///
    /// Reads [`DevelopmentFormEvidence`] — the season bucket that actually
    /// carries the player's football. Youth-league fixtures book into the
    /// friendly bucket, so judging only the official counters (as this
    /// gate originally did) made academy-league form invisible and the
    /// merit upgrade effectively fired only off borrowed senior minutes.
    fn is_earned(player: &Player, date: NaiveDate) -> bool {
        let is_youth = player
            .contract
            .as_ref()
            .map(|c| c.contract_type == ContractType::Youth)
            .unwrap_or(false);
        if !is_youth {
            return false;
        }

        // Don't upgrade a player already earmarked to leave.
        if player.statuses.has(PlayerStatusType::Lst) || player.statuses.has(PlayerStatusType::Loa)
        {
            return false;
        }

        if player.age(date) < Self::MIN_AGE {
            return false;
        }
        if DevelopmentFormEvidence::games(player) < Self::MIN_GAMES {
            return false;
        }

        DevelopmentFormEvidence::regressed_rating(player) >= Self::MIN_RATING
    }

    /// Upgrade a youth contract to a full professional contract. Used both
    /// when a player is promoted to the main team and when a youth-team
    /// player earns pro terms on form. `club_rep` is the main team's world
    /// reputation — it scales the graduation salary so the same ability
    /// earns far more at a big club.
    pub(in crate::club::core) fn upgrade(player: &mut Player, date: NaiveDate, club_rep: u16) {
        let is_youth = player
            .contract
            .as_ref()
            .map(|c| c.contract_type == ContractType::Youth)
            .unwrap_or(false);
        if !is_youth {
            return;
        }

        let expiration = NaiveDate::from_ymd_opt(date.year() + 3, date.month(), date.day().min(28))
            .unwrap_or(date);
        let salary = GraduationTerms::salary(player.player_attributes.current_ability, club_rep);
        let mut upgraded = PlayerClubContract::new(salary, expiration);
        // Anchor the new senior contract to today so the wage-envy
        // grace window correctly treats a freshly graduated youngster
        // as "just signed" — otherwise the monthly audit fires
        // SalaryGapNoticed on a player who only just earned a senior
        // wage in the first place.
        upgraded.started = Some(date);
        player.contract = Some(upgraded);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, HappinessEventType, PersonAttributes,
        PlayerAttributes, PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills, StaffCollection, TeamBuilder, TeamCollection, TeamReputation,
        TrainingSchedule,
    };
    use chrono::{Datelike, NaiveDate, NaiveTime};

    struct Fx;

    impl Fx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()
        }

        fn schedule() -> TrainingSchedule {
            TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            )
        }

        /// A youth-contract central midfielder with `games` starts each
        /// rated `rating`, so `average_rating_realistic` is deterministic.
        /// `record_match_rating` builds the minutes-weighted ledger;
        /// `played` is set separately because the ledger doesn't count
        /// appearances.
        fn youth(id: u32, age: u8, games: u16, rating: f32) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = 90;
            attrs.potential_ability = 140;
            attrs.condition = 10_000;
            let contract = PlayerClubContract::new_youth(
                10_000,
                NaiveDate::from_ymd_opt(2029, 6, 30).unwrap(),
            );
            let mut p = PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Y".into(), format!("P{id}")))
                .birth_date(
                    NaiveDate::from_ymd_opt(Self::date().year() - age as i32, 1, 1).unwrap(),
                )
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 18,
                    }],
                })
                .player_attributes(attrs)
                .contract(Some(contract))
                .build()
                .unwrap();
            p.statistics.played = games;
            for _ in 0..games {
                p.statistics.record_match_rating(rating, 90, true);
            }
            p
        }

        /// Club with an (empty) Main team — only needed so `main_index`
        /// resolves and the salary scale has a reputation — plus a U19
        /// squad holding the youth players under test.
        fn club(youth: Vec<Player>) -> Club {
            let main = TeamBuilder::new()
                .id(10)
                .league_id(Some(1))
                .club_id(100)
                .name("Main".into())
                .slug("main".into())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(Vec::new()))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(2_000, 2_000, 2_000))
                .training_schedule(Self::schedule())
                .build()
                .unwrap();
            let u19 = TeamBuilder::new()
                .id(19)
                .league_id(None)
                .club_id(100)
                .name("U19".into())
                .slug("u19".into())
                .team_type(TeamType::U19)
                .players(PlayerCollection::new(youth))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(800, 800, 800))
                .training_schedule(Self::schedule())
                .build()
                .unwrap();
            Club::new(
                100,
                "Club".into(),
                Location::new(1),
                ClubFinances::new(10_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![main, u19]),
                ClubFacilities::default(),
            )
        }

        fn contract_type(club: &Club, id: u32) -> ContractType {
            club.teams
                .teams
                .iter()
                .flat_map(|t| t.players.players.iter())
                .find(|p| p.id == id)
                .and_then(|p| p.contract.as_ref())
                .map(|c| c.contract_type.clone())
                .expect("player must exist with a contract")
        }
    }

    /// A youth player with a real run of strong games earns a full
    /// professional contract — in place, still on the U19 roster.
    #[test]
    fn good_form_youth_earns_pro_contract() {
        let mut club = Fx::club(vec![Fx::youth(1, 17, 12, 7.8)]);
        club.review_youth_contracts(Fx::date());

        assert_eq!(
            Fx::contract_type(&club, 1),
            ContractType::FullTime,
            "a youth player on strong form must be upgraded to a full contract"
        );
        // Upgraded in place — not moved to the main team.
        assert!(
            club.teams.teams[1]
                .players
                .players
                .iter()
                .any(|p| p.id == 1),
            "the upgraded player stays in his youth squad"
        );
        // The milestone fires a positive contract event.
        let fired = club.teams.teams[1]
            .players
            .players
            .iter()
            .find(|p| p.id == 1)
            .unwrap()
            .happiness
            .recent_events
            .iter()
            .any(|e| e.event_type == HappinessEventType::ContractRenewal);
        assert!(fired, "earning pro terms must register a contract event");
    }

    /// Strong ratings but too small a sample: no upgrade yet.
    #[test]
    fn insufficient_games_stays_youth() {
        let mut club = Fx::club(vec![Fx::youth(2, 17, 4, 8.5)]);
        club.review_youth_contracts(Fx::date());
        assert_eq!(
            Fx::contract_type(&club, 2),
            ContractType::Youth,
            "a tiny sample must not trigger a pro contract, however high the rating"
        );
    }

    /// A full season of merely average ratings doesn't earn pro terms.
    #[test]
    fn mediocre_form_stays_youth() {
        let mut club = Fx::club(vec![Fx::youth(3, 18, 15, 6.7)]);
        club.review_youth_contracts(Fx::date());
        assert_eq!(
            Fx::contract_type(&club, 3),
            ContractType::Youth,
            "around-average form must not earn a professional contract"
        );
    }

    /// Youth-league fixtures are friendly-flagged and book into the
    /// friendly bucket — a standout academy-league season must count as
    /// merit evidence even with zero official (senior) minutes.
    #[test]
    fn youth_league_form_earns_pro_contract() {
        let mut prospect = Fx::youth(9, 17, 0, 0.0);
        prospect.friendly_statistics.played = 12;
        for _ in 0..12 {
            prospect
                .friendly_statistics
                .record_match_rating(7.8, 90, true);
        }
        let mut club = Fx::club(vec![prospect]);
        club.review_youth_contracts(Fx::date());
        assert_eq!(
            Fx::contract_type(&club, 9),
            ContractType::FullTime,
            "academy-league form must earn a professional contract"
        );
    }

    /// Even outstanding form is gated by the real-world minimum age for
    /// signing professional terms.
    #[test]
    fn below_min_age_stays_youth() {
        let mut club = Fx::club(vec![Fx::youth(4, 15, 12, 7.8)]);
        club.review_youth_contracts(Fx::date());
        assert_eq!(
            Fx::contract_type(&club, 4),
            ContractType::Youth,
            "a player below the professional-contract age floor must stay on youth terms"
        );
    }
}
