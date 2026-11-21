//! A player has left the club: the one fact every part of it reacts to.

use chrono::{Duration, NaiveDate};

use crate::club::person::Person;
use crate::club::player::squad::PathwayStage;
use crate::club::staff::coach::{PlannedRole, PlayerPlanEntry};
use crate::club::staff::perception::{AbilityEstimator, PotentialEstimator};
use crate::r#match::squad::selection::PitchLine;
use crate::transfers::pipeline::ShirtHolder;
use crate::{
    Club, ClubDirectionContext, ClubDirectionEvidence, ClubDirectionKind, ClubFinances, Player,
    PlayerPositionType, PlayerPreferredFoot, PlayerSquadStatus,
};

/// How he left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepartureKind {
    /// Sold to another club.
    Sold,
    /// Bought by the club he was on loan at.
    BoughtOut,
    /// Let go for nothing, or his contract ran out.
    Released,
    /// A borrowed player went back to his own club.
    LoanEnded,
}

impl DepartureKind {
    /// The club chose to take money for him.
    pub fn is_sale(self) -> bool {
        matches!(self, DepartureKind::Sold | DepartureKind::BoughtOut)
    }
}

/// What the club knew of a player on the day he left it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepartedPlayer {
    pub player_id: u32,
    /// The shirt he was planned for, or his own position when the first
    /// team was not picking him.
    pub shirt: PlayerPositionType,
    /// Level and ceiling as the staff perceived them.
    pub level: u8,
    pub ceiling: u8,
    pub age: u8,
    pub foot: PlayerPreferredFoot,
    pub role: Option<PlannedRole>,
    /// The first team's first choice for his shirt.
    pub first_choice: bool,
    /// The share of the first team's football the plan gave him.
    pub share: f32,
    pub fee: f64,
    pub date: NaiveDate,
    pub kind: DepartureKind,
    /// The day the club's transfer window shuts, when one is open.
    pub window_closes: Option<NaiveDate>,
}

impl DepartedPlayer {
    /// What being the first choice for a shirt is worth on its own.
    const FIRST_CHOICE: f32 = 0.6;

    /// Read while he is still in the club's squads: the plan of the man
    /// picking him has not let go of him yet.
    pub fn read(club: &Club, player_id: u32, kind: DepartureKind, date: NaiveDate) -> Option<Self> {
        let (team, player) = club
            .teams
            .teams
            .iter()
            .find_map(|t| t.players.find(player_id).map(|p| (t, p)))?;
        let first_team = club.teams.main().is_some_and(|main| main.id == team.id);
        let entry = first_team
            .then(|| team.staffs.head_coach().squad_plan.entry(player_id))
            .flatten();
        Some(Self::of(player, entry, kind, date))
    }

    /// A man the club owned and was not picking: out on loan when he went.
    pub fn away(player: &Player, kind: DepartureKind, date: NaiveDate) -> Self {
        Self::of(player, None, kind, date)
    }

    fn of(
        player: &Player,
        entry: Option<&PlayerPlanEntry>,
        kind: DepartureKind,
        date: NaiveDate,
    ) -> Self {
        DepartedPlayer {
            player_id: player.id,
            shirt: entry
                .and_then(|e| e.shirt)
                .unwrap_or_else(|| player.position()),
            level: AbilityEstimator::observable_level(player),
            ceiling: PotentialEstimator::observable_ceiling(player, date),
            age: player.age(date),
            foot: player.preferred_foot,
            role: entry.map(|e| e.role),
            first_choice: entry.is_some_and(|e| e.shirt.is_some() && e.place == 0),
            share: entry.map_or(0.0, |e| e.share),
            fee: 0.0,
            date,
            kind,
            window_closes: None,
        }
    }

    pub fn for_fee(mut self, fee: f64) -> Self {
        self.fee = fee.max(0.0);
        self
    }

    pub fn with_window(mut self, closes: Option<NaiveDate>) -> Self {
        self.window_closes = closes.filter(|day| *day >= self.date);
        self
    }

    /// How much he mattered to this club, 0..1: the football he was
    /// taking and the role he was planned for. Never his fame.
    pub fn importance(&self) -> f32 {
        let role = match self.role {
            Some(PlannedRole::Cornerstone) => 0.9,
            Some(PlannedRole::Starter) => 0.75,
            _ => 0.0,
        };
        let first = if self.first_choice {
            Self::FIRST_CHOICE
        } else {
            0.0
        };
        self.share.max(role).max(first).clamp(0.0, 1.0)
    }
}

/// A sale the dressing room has not judged yet. The club is given until
/// `due` to answer it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyDeparture {
    pub departed: DepartedPlayer,
    pub due: NaiveDate,
}

impl KeyDeparture {
    /// Importance at which a sale is one the dressing room watches.
    const MATTERS: f32 = 0.5;
    /// The longest the club is given to answer.
    const GRACE_DAYS: i64 = 21;
    /// Perceived levels within which a shirt is held "at about his level".
    const ABOUT_HIS_LEVEL: u8 = 8;
    /// Perceived levels by which a signing is clearly better than he was.
    const CLEARLY_BETTER: u8 = 8;

    fn of(departed: &DepartedPlayer) -> Option<Self> {
        (departed.kind.is_sale() && departed.importance() >= Self::MATTERS).then(|| {
            let grace = departed.date + Duration::days(Self::GRACE_DAYS);
            KeyDeparture {
                departed: *departed,
                due: departed.window_closes.map_or(grace, |closes| closes.min(grace)),
            }
        })
    }
}

/// What the dressing room makes of how the club answered a sale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DepartureVerdict {
    Concern,
    Settled,
    Encouraged { signing: u32 },
}

impl Club {
    /// A player has left the club. Each part of it reacts in its own way:
    /// the coach's plan moves the man behind him up, the money is booked,
    /// and the dressing room is given a date to judge what the club did.
    pub fn on_player_departed(&mut self, departed: &DepartedPlayer) {
        self.move_up_behind(departed);
        let mut recycled = 0.0;
        if departed.kind.is_sale() && departed.fee > 0.0 {
            let share = ClubFinances::reinvest_share_for(&self.philosophy);
            self.finance.add_transfer_income_at(departed.fee, share);
            self.transfer_plan.fees_received_this_window += departed.fee;
            recycled = departed.fee * share;
        }
        self.transfer_plan.stage_vacancy(departed, recycled);
        if let Some(staged) = KeyDeparture::of(departed) {
            self.key_departures.push(staged);
        }
    }

    /// The plan is drawn again without him, and the man who now holds a
    /// first choice's shirt moves up the pathway with it.
    fn move_up_behind(&mut self, departed: &DepartedPlayer) {
        let lean = self.philosophy.development_lean();
        let Some(main) = self.teams.main_mut() else {
            return;
        };
        main.staffs.squad_planner_mut().squad_plan.mark_stale();
        main.revise_coach_squad_plan(departed.date, Some(lean));
        if !departed.first_choice {
            return;
        }
        if let Some(heir) = self.heir_to(departed) {
            self.promote_into_the_shirt(heir, departed.date);
        }
    }

    /// The man the departure brings through: a first choice on the line
    /// the departed player played in whom the pathway still reads as
    /// below a starter. The holder of his own shirt before a mirror of it,
    /// the better man before the lesser.
    fn heir_to(&self, departed: &DepartedPlayer) -> Option<u32> {
        let main = self.teams.main()?;
        let plan = &main.staffs.head_coach().squad_plan;
        let line = PitchLine::of(departed.shirt);
        main.players
            .iter()
            .filter(|p| !p.pathway_stage().is_terminal())
            .filter(|p| {
                !matches!(
                    p.pathway_stage(),
                    PathwayStage::Starter | PathwayStage::Core | PathwayStage::LoanOut
                )
            })
            .filter_map(|p| {
                let entry = plan.entry(p.id).filter(|e| e.place == 0)?;
                let shirt = entry.shirt?;
                (PitchLine::of(shirt) == line).then_some((
                    shirt == departed.shirt,
                    AbilityEstimator::observable_level(p),
                    p.id,
                ))
            })
            .max()
            .map(|(_, _, id)| id)
    }

    /// Who holds a shirt a man left on `since`: the first choice's level
    /// as the staff perceive it, and whether anybody planned for the shirt
    /// has joined the club after him.
    pub fn shirt_holder(&self, shirt: PlayerPositionType, since: NaiveDate) -> Option<ShirtHolder> {
        let holder = self.holder_of(shirt)?;
        let main = self.teams.main()?;
        let plan = &main.staffs.head_coach().squad_plan;
        let signed_since = main.players.iter().any(|p| {
            p.last_transfer_date.is_some_and(|joined| joined >= since)
                && plan.entry(p.id).is_some_and(|e| e.shirt == Some(shirt))
        });
        Some(ShirtHolder {
            level: AbilityEstimator::observable_level(holder),
            signed_since,
        })
    }

    /// The first team's first choice for a shirt, as the plan stands.
    fn holder_of(&self, shirt: PlayerPositionType) -> Option<&Player> {
        let main = self.teams.main()?;
        let plan = &main.staffs.head_coach().squad_plan;
        main.players.iter().find(|p| {
            plan.entry(p.id)
                .is_some_and(|e| e.shirt == Some(shirt) && e.place == 0)
        })
    }

    /// The sales whose grace has run out are judged, and the players who
    /// care are told what the club did about each.
    pub(in crate::club::core) fn judge_key_departures(&mut self, today: NaiveDate) {
        if !self.key_departures.iter().any(|d| d.due <= today) {
            return;
        }
        let (due, waiting): (Vec<KeyDeparture>, Vec<KeyDeparture>) = self
            .key_departures
            .drain(..)
            .partition(|d| d.due <= today);
        self.key_departures = waiting;
        for staged in due {
            let verdict = self.departure_verdict(&staged.departed);
            self.tell_the_dressing_room(&staged.departed, verdict);
        }
    }

    /// The shirt's holder answers for the shirt; a man signed since for
    /// anywhere on his line answers too, because the plan may have moved
    /// the men round to fit him in, and the dressing room sees the club
    /// went and got somebody.
    fn departure_verdict(&self, departed: &DepartedPlayer) -> DepartureVerdict {
        let Some(main) = self.teams.main() else {
            return DepartureVerdict::Concern;
        };
        let plan = &main.staffs.head_coach().squad_plan;
        let line = PitchLine::of(departed.shirt);
        let holder = self.holder_of(departed.shirt);
        let signing = main
            .players
            .iter()
            .filter(|p| p.last_transfer_date.is_some_and(|joined| joined >= departed.date))
            .filter(|p| {
                plan.entry(p.id)
                    .is_some_and(|e| e.shirt.is_some_and(|shirt| PitchLine::of(shirt) == line))
            })
            .max_by_key(|p| AbilityEstimator::observable_level(p));
        let level_of = |p: &Player| AbilityEstimator::observable_level(p);
        if let Some(signing) = signing
            && level_of(signing) >= departed.level.saturating_add(KeyDeparture::CLEARLY_BETTER)
        {
            return DepartureVerdict::Encouraged { signing: signing.id };
        }
        let answered = holder
            .into_iter()
            .chain(signing)
            .any(|p| level_of(p).saturating_add(KeyDeparture::ABOUT_HIS_LEVEL) >= departed.level);
        if answered {
            DepartureVerdict::Settled
        } else {
            DepartureVerdict::Concern
        }
    }

    fn tell_the_dressing_room(&mut self, departed: &DepartedPlayer, verdict: DepartureVerdict) {
        if verdict == DepartureVerdict::Settled {
            return;
        }
        let weight = departed.importance();
        for team in &mut self.teams.teams {
            for player in team.players.iter_mut() {
                let status = player
                    .contract
                    .as_ref()
                    .map(|c| c.squad_status.clone())
                    .unwrap_or(PlayerSquadStatus::FirstTeamRegular);
                let cares = player.attributes.ambition >= 14.0
                    || matches!(
                        status,
                        PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular
                    );
                if !cares {
                    continue;
                }
                let mut ctx = match verdict {
                    DepartureVerdict::Concern => {
                        ClubDirectionContext::new(ClubDirectionKind::Concern)
                            .with_focal_player(departed.player_id)
                            .with_evidence(ClubDirectionEvidence::KeyPlayerSoldUnreplaced)
                            .with_evidence(ClubDirectionEvidence::SquadQualityWeakened)
                    }
                    DepartureVerdict::Encouraged { signing } => {
                        if player.id == signing {
                            continue;
                        }
                        ClubDirectionContext::new(ClubDirectionKind::Encouragement)
                            .with_focal_player(signing)
                            .with_evidence(ClubDirectionEvidence::MeaningfulSigningArrived)
                    }
                    DepartureVerdict::Settled => continue,
                }
                .with_weight(weight);
                if player.attributes.ambition >= 15.0 {
                    ctx = ctx.with_evidence(ClubDirectionEvidence::HighAmbition);
                }
                if matches!(status, PlayerSquadStatus::KeyPlayer) {
                    ctx = ctx.with_evidence(ClubDirectionEvidence::HighInfluence);
                }
                match verdict {
                    DepartureVerdict::Concern => player.on_club_direction_concern(ctx),
                    _ => player.on_club_direction_encouragement(ctx),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::transfers::pipeline::{WatchAvailability, WatchlistEntry};
    use crate::transfers::squad::plan::BriefTier;
    use crate::{
        ClubColors, ClubFacilities, ClubStatus, HappinessEventType, MatchTacticType,
        PersonAttributes, PlayerAttributes, PlayerClubContract, PlayerCollection, PlayerPosition,
        PlayerPositions, PlayerSkills, StaffCollection, Tactics, TeamBuilder, TeamCollection,
        TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::NaiveTime;
    use PlayerPositionType::*;

    /// A club with a 4-4-2 eleven (ids 1..=11 in shirt order, level 150)
    /// and whoever the test puts behind it.
    struct Sale;

    impl Sale {
        const LEFT_BACK: u32 = 2;
        const STRIKER: u32 = 10;

        fn day() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 8, 4).unwrap()
        }

        fn player(id: u32, position: PlayerPositionType, level: u8, age: i32) -> Player {
            let mut contract =
                PlayerClubContract::new(50_000, NaiveDate::from_ymd_opt(2030, 6, 30).unwrap());
            contract.squad_status = PlayerSquadStatus::FirstTeamRegular;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("P".to_string(), format!("L{id}")))
                .birth_date(NaiveDate::from_ymd_opt(2026 - age, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::flat_for_ability(level))
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position,
                        level: 20,
                    }],
                })
                .player_attributes(PlayerAttributes {
                    current_ability: level,
                    potential_ability: level,
                    condition: 9500,
                    ..Default::default()
                })
                .contract(Some(contract))
                .build()
                .unwrap()
        }

        fn club(extra: Vec<Player>) -> Club {
            let mut players: Vec<Player> = Tactics::new(MatchTacticType::T442)
                .positions()
                .iter()
                .enumerate()
                .map(|(i, &slot)| Self::player(i as u32 + 1, slot, 150, 27))
                .collect();
            assert_eq!(players[Self::LEFT_BACK as usize - 1].position(), DefenderLeft);
            assert!(players[Self::STRIKER as usize - 1].position().is_forward());
            players.extend(extra);
            let team = TeamBuilder::new()
                .id(1)
                .league_id(Some(1))
                .club_id(1)
                .name("team".to_string())
                .slug("team".to_string())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(100, 100, 100))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
                ))
                .build()
                .unwrap();
            let mut club = Club::new(
                1,
                "Club".to_string(),
                Location::new(1),
                ClubFinances::new(10_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![team]),
                ClubFacilities::default(),
            );
            club.teams.teams[0].revise_coach_squad_plan(Self::day(), None);
            club
        }

        /// The player goes for `fee`, with the window shutting on `closes`.
        fn sell(club: &mut Club, player_id: u32, closes: Option<NaiveDate>) -> DepartedPlayer {
            let departed = DepartedPlayer::read(club, player_id, DepartureKind::Sold, Self::day())
                .unwrap()
                .for_fee(20_000_000.0)
                .with_window(closes);
            club.teams.teams[0].players.take_player(&player_id);
            club.on_player_departed(&departed);
            departed
        }

        fn entry(club: &Club, player_id: u32) -> PlayerPlanEntry {
            *club.teams.teams[0]
                .staffs
                .head_coach()
                .squad_plan
                .entry(player_id)
                .unwrap()
        }

        fn sign(club: &mut Club, mut player: Player, on: NaiveDate) {
            player.last_transfer_date = Some(on);
            club.teams.teams[0].players.add(player);
            club.teams.teams[0].revise_coach_squad_plan(on, None);
        }

        fn concerned(club: &Club) -> usize {
            Self::recorded(club, HappinessEventType::ConcernedByClubDirection)
        }

        fn recorded(club: &Club, event: HappinessEventType) -> usize {
            club.teams.teams[0]
                .players
                .iter()
                .filter(|p| p.happiness.has_recent_event(&event, 60))
                .count()
        }
    }

    fn filed(position: PlayerPositionType, believed: u8, value: f64) -> WatchlistEntry {
        WatchlistEntry {
            player_id: 900 + u32::from(believed),
            club_id: 77,
            country_id: 1,
            position,
            position_group: position.position_group(),
            age: 25,
            believed_ability: believed,
            estimated_value: value,
            score: 10.0,
            availability: WatchAvailability::None,
            slot_position: position,
            slot_tier: BriefTier::B,
            last_refreshed: Sale::day(),
            is_loan: false,
        }
    }

    fn id_at(club: &Club, shirt: PlayerPositionType) -> u32 {
        club.teams.teams[0]
            .players
            .iter()
            .find(|p| p.position() == shirt)
            .unwrap()
            .id
    }

    #[test]
    fn an_heir_at_his_level_makes_a_willing_seller() {
        let club = Sale::club(vec![Sale::player(30, DefenderLeft, 148, 21)]);
        let cover = club.seller_cover(Sale::LEFT_BACK, 20_000_000.0, Some(3));
        assert!(cover.first_choice);
        assert!(cover.readiness > 0.95, "{}", cover.readiness);
        assert!(!cover.declines(Some(0), 1.0, false));
    }

    #[test]
    fn with_nobody_behind_him_the_club_holds_its_first_choice_on_deadline_day() {
        let club = Sale::club(Vec::new());
        let cover = club.seller_cover(Sale::STRIKER, 20_000_000.0, Some(0));
        assert_eq!(cover.readiness, 0.0);
        assert!(cover.declines(Some(0), 1.0, false), "an offer at his value");
        assert!(!cover.declines(Some(21), 1.0, false), "three weeks left: he is priced, not held");
        assert!(!cover.declines(Some(0), 1.6, false), "an offer it cannot refuse");
        assert!(!cover.declines(Some(0), 1.0, true), "a player forcing the move");
    }

    #[test]
    fn a_cheap_full_back_on_file_is_no_successor_to_a_centre_back() {
        let mut club = Sale::club(Vec::new());
        let centre_back = id_at(&club, DefenderCenterLeft);
        club.transfer_plan
            .watchlist
            .push(filed(DefenderLeft, 110, 1_000_000.0));
        let before = club.replacement_readiness(centre_back, 20_000_000.0, Some(30));
        assert_eq!(before.names, 0.0);

        club.transfer_plan
            .watchlist
            .push(filed(DefenderCenterLeft, 146, 15_000_000.0));
        let after = club.replacement_readiness(centre_back, 20_000_000.0, Some(30));
        assert!(after.names > 0.0, "a centre-back near his level that the fee would buy");
        let dear = club.replacement_readiness(centre_back, 10_000_000.0, Some(30));
        assert_eq!(dear.names, 0.0, "one the fee would not buy is no help");
    }

    #[test]
    fn the_heir_is_the_first_choice_at_left_back_on_the_day_of_the_sale() {
        let mut club = Sale::club(vec![Sale::player(30, DefenderLeft, 120, 20)]);
        assert_eq!(Sale::entry(&club, 30).place, 1, "planned behind him");

        Sale::sell(&mut club, Sale::LEFT_BACK, None);
        let heir = Sale::entry(&club, 30);
        assert_eq!((heir.shirt, heir.place), (Some(DefenderLeft), 0));
    }

    #[test]
    fn a_centre_backs_role_does_not_change_because_a_left_back_was_sold() {
        let mut club = Sale::club(vec![
            Sale::player(30, DefenderLeft, 120, 20),
            Sale::player(31, DefenderCenterLeft, 140, 20),
        ]);
        let before = Sale::entry(&club, 31);
        let stage = club.teams.teams[0].players.find(31).unwrap().pathway_stage();

        Sale::sell(&mut club, Sale::LEFT_BACK, None);
        let after = Sale::entry(&club, 31);
        assert_eq!((after.role, after.shirt, after.place), (before.role, before.shirt, before.place));
        assert_eq!(club.teams.teams[0].players.find(31).unwrap().pathway_stage(), stage);
    }

    #[test]
    fn a_sale_books_the_fee_and_the_share_the_club_recycles() {
        let mut club = Sale::club(Vec::new());
        Sale::sell(&mut club, Sale::STRIKER, None);
        assert_eq!(club.finance.season_fees.received, 20_000_000.0);
        assert_eq!(
            club.finance.season_fees.recycled,
            20_000_000.0 * ClubFinances::reinvest_share_for(&club.philosophy)
        );
    }

    #[test]
    fn nobody_reacts_on_the_day_and_the_club_is_given_until_the_window_shuts() {
        let mut club = Sale::club(vec![Sale::player(40, ForwardLeft, 100, 24)]);
        let closes = Sale::day() + Duration::days(10);
        Sale::sell(&mut club, Sale::STRIKER, Some(closes));

        assert_eq!(Sale::concerned(&club), 0, "no reaction on the day");
        assert_eq!(club.key_departures.len(), 1);
        assert_eq!(club.key_departures[0].due, closes, "the window shuts before three weeks are up");

        club.judge_key_departures(closes - Duration::days(1));
        assert_eq!(Sale::concerned(&club), 0, "nor before the grace ends");
    }

    #[test]
    fn a_sale_with_no_window_open_is_judged_after_three_weeks() {
        let mut club = Sale::club(Vec::new());
        Sale::sell(&mut club, Sale::STRIKER, None);
        assert_eq!(club.key_departures[0].due, Sale::day() + Duration::days(21));
    }

    #[test]
    fn a_striker_sold_and_replaced_at_his_level_inside_the_window_concerns_nobody() {
        let mut club = Sale::club(Vec::new());
        let closes = Sale::day() + Duration::days(20);
        let departed = Sale::sell(&mut club, Sale::STRIKER, Some(closes));
        Sale::sign(
            &mut club,
            Sale::player(50, departed.shirt, 147, 26),
            Sale::day() + Duration::days(14),
        );

        club.judge_key_departures(closes);
        assert!(club.key_departures.is_empty());
        assert_eq!(Sale::concerned(&club), 0);
        assert_eq!(Sale::recorded(&club, HappinessEventType::EncouragedBySquadInvestment), 0);
    }

    #[test]
    fn a_striker_sold_and_not_replaced_concerns_the_men_who_care_and_names_the_sale() {
        let mut club = Sale::club(vec![Sale::player(40, ForwardLeft, 100, 24)]);
        let closes = Sale::day() + Duration::days(20);
        let departed = Sale::sell(&mut club, Sale::STRIKER, Some(closes));

        club.judge_key_departures(closes);
        assert!(Sale::concerned(&club) >= 10, "{} of the squad", Sale::concerned(&club));
        let named = club.teams.teams[0].players.iter().any(|p| {
            p.happiness.recent_events.iter().any(|event| {
                event.context.as_ref().is_some_and(|ctx| {
                    ctx.club_direction_context
                        .as_ref()
                        .is_some_and(|d| d.focal_player_id == Some(departed.player_id))
                })
            })
        });
        assert!(named, "the concern names the man who was sold");
    }

    #[test]
    fn a_signing_at_his_level_on_his_line_settles_it_whichever_shirt_he_is_planned_for() {
        let mut club = Sale::club(Vec::new());
        let closes = Sale::day() + Duration::days(20);
        let departed = Sale::sell(&mut club, Sale::STRIKER, Some(closes));
        // A striker for the other shirt of the pair, who pushes the man
        // there across: the plan decides where each one plays.
        let other = if departed.shirt == ForwardLeft { ForwardRight } else { ForwardLeft };
        Sale::sign(
            &mut club,
            Sale::player(50, other, 152, 26),
            Sale::day() + Duration::days(9),
        );
        club.judge_key_departures(closes);
        assert_eq!(Sale::concerned(&club), 0);
    }

    #[test]
    fn an_heir_the_staff_see_at_his_level_concerns_nobody() {
        let mut club = Sale::club(vec![Sale::player(40, ForwardLeft, 146, 20)]);
        let closes = Sale::day() + Duration::days(20);
        Sale::sell(&mut club, Sale::STRIKER, Some(closes));
        club.judge_key_departures(closes);
        assert_eq!(Sale::concerned(&club), 0);
    }

    #[test]
    fn a_signing_clearly_better_than_the_man_sold_encourages_the_squad() {
        let mut club = Sale::club(Vec::new());
        let closes = Sale::day() + Duration::days(20);
        let departed = Sale::sell(&mut club, Sale::STRIKER, Some(closes));
        Sale::sign(
            &mut club,
            Sale::player(50, departed.shirt, 170, 25),
            Sale::day() + Duration::days(5),
        );
        club.judge_key_departures(closes);
        assert_eq!(Sale::concerned(&club), 0);
        assert!(Sale::recorded(&club, HappinessEventType::EncouragedBySquadInvestment) >= 10);
    }

    #[test]
    fn importance_is_read_from_his_place_at_this_club_and_never_from_his_fame() {
        // The same first-choice striker, unknown outside his country at
        // one club and famous at the other.
        let weigh = |fame: i16| {
            let mut club = Sale::club(Vec::new());
            let striker = club.teams.teams[0]
                .players
                .players
                .iter_mut()
                .find(|p| p.id == Sale::STRIKER)
                .unwrap();
            striker.player_attributes.world_reputation = fame;
            striker.player_attributes.current_reputation = fame;
            Sale::sell(&mut club, Sale::STRIKER, None).importance()
        };
        assert_eq!(weigh(300), weigh(9_000));
        assert!(weigh(300) >= 0.6);
    }

    #[test]
    fn a_famous_man_planned_as_cover_leaves_and_nobody_reacts() {
        let mut famous = Sale::player(60, MidfielderCenterLeft, 118, 31);
        famous.player_attributes.world_reputation = 9_000;
        famous.player_attributes.current_reputation = 9_000;
        let mut club = Sale::club(vec![Sale::player(61, MidfielderCenterLeft, 140, 26), famous]);
        let departed = Sale::sell(&mut club, 60, None);

        assert!(departed.importance() < 0.5, "{}", departed.importance());
        assert!(club.key_departures.is_empty(), "there is nothing to judge");
        club.judge_key_departures(Sale::day() + Duration::days(60));
        assert_eq!(Sale::concerned(&club), 0);
    }
}
