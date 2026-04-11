use crate::club::team::behaviour::TeamBehaviour;
use crate::club::team::builder::TeamBuilder;
use crate::club::team::mentorship::process_mentorship;
use crate::context::GlobalContext;
use crate::shared::CurrencyValue;
use crate::utils::DateUtils;
use crate::{
    MatchHistory, MatchTacticType, Player, PlayerCollection, PlayerSquadStatus, PlayerStatusType,
    StaffCollection, Tactics, TacticsSelector, TeamReputation, TeamResult, TeamTraining,
    TrainingSchedule, TransferItem, Transfers,
};
use chrono::Datelike;
use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct Team {
    pub id: u32,
    pub league_id: Option<u32>,
    pub club_id: u32,
    pub name: String,
    pub slug: String,

    pub team_type: TeamType,
    pub tactics: Option<Tactics>,

    pub players: PlayerCollection,
    pub staffs: StaffCollection,

    pub behaviour: TeamBehaviour,

    pub reputation: TeamReputation,
    pub training_schedule: TrainingSchedule,
    pub transfer_list: Transfers,
    pub match_history: MatchHistory,
}

impl Team {
    pub fn builder() -> TeamBuilder {
        TeamBuilder::new()
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> TeamResult {
        // Recalculate squad statuses monthly (1st of month)
        if ctx.simulation.date.day() == 1 {
            self.update_squad_statuses(ctx.simulation.date.date());
        }

        // Weekly mentorship pass — pair senior players with juniors. Runs
        // before player development so any personality drift from mentoring
        // is already visible when weekly skill growth is computed.
        if ctx.simulation.is_week_beginning() {
            let hoy_wwy = self.staffs.best_youth_development_wwy(10);
            let _pairings = process_mentorship(
                &mut self.players.players,
                ctx.simulation.date.date(),
                hoy_wwy,
            );

            // Weekly physio preventive-rest pass. Elite sports-science staff
            // can predict which players are heading into the injury danger
            // zone (high jadedness + low condition) and flag them with Rst
            // so the selection logic leaves them out. Beyond FM: this is an
            // explicit, visible mechanism instead of an opaque "rest" hint.
            let best_sports_sci = self.staffs.best_sports_science();
            apply_preventive_rest(
                &mut self.players.players,
                best_sports_sci,
                ctx.simulation.date.date(),
            );
        }

        // Pass team reputation to players via context
        let player_ctx = ctx.with_team_reputation(self.id, self.reputation.overall_score());
        let result = TeamResult::new(
            self.id,
            self.players.simulate(player_ctx.with_player(None)),
            self.staffs.simulate(ctx.with_staff(None)),
            self.behaviour
                .simulate(&mut self.players, &mut self.staffs, ctx.with_team(self.id)),
            TeamTraining::train(self, ctx.simulation.date, ctx.club_facilities_training()),
        );

        if self.tactics.is_none() {
            self.tactics = Some(TacticsSelector::select(self, self.staffs.head_coach()));
        };

        if self.training_schedule.is_default {
            //let coach = self.staffs.head_coach();
        }

        result
    }

    /// Assign squad status based on CA rank within the team.
    fn update_squad_statuses(&mut self, date: chrono::NaiveDate) {
        let team_cas = self.players.current_abilities_desc();

        for player in self.players.iter_mut() {
            if let Some(ref mut contract) = player.contract {
                let age = DateUtils::age(player.birth_date, date);
                contract.squad_status = PlayerSquadStatus::calculate(
                    player.player_attributes.current_ability,
                    age,
                    &team_cas,
                );
            }
        }
    }

    pub fn players(&self) -> Vec<&Player> {
        self.players.players()
    }

    pub fn add_player_to_transfer_list(&mut self, player_id: u32, value: CurrencyValue) {
        self.transfer_list.add(TransferItem {
            player_id,
            amount: value,
        })
    }

    pub fn get_annual_salary(&self) -> u32 {
        self.players
            .players
            .iter()
            .filter_map(|p| p.contract.as_ref())
            .map(|c| c.salary)
            .chain(
                self.staffs
                    .staffs
                    .iter()
                    .filter_map(|p| p.contract.as_ref())
                    .map(|c| c.salary),
            )
            .sum()
    }

    pub fn tactics(&self) -> Cow<'_, Tactics> {
        if let Some(tactics) = &self.tactics {
            Cow::Borrowed(tactics)
        } else {
            Cow::Owned(Tactics::new(MatchTacticType::T442))
        }
    }
}

/// Preventive-rest pass. An elite sports-science department flags players
/// whose fatigue/jadedness profile predicts an imminent injury and sets
/// the `Rst` status — a hint that the squad selector treats as "don't pick
/// this week unless emergency". A bare-bones medical team can't do this.
///
/// Thresholds are tuned so that with neutral (0.35-ish) sports science the
/// function flags no one, and with elite (0.85+) it flags the worst
/// offenders before they hit the danger zone.
fn apply_preventive_rest(
    players: &mut [Player],
    best_sports_sci: u8,
    date: chrono::NaiveDate,
) {
    if best_sports_sci < 12 {
        // Basic medical teams can't preempt — the manager finds out when
        // the player is actually injured.
        return;
    }

    // Scale: SS 12 → only the most extreme cases flagged; SS 20 → anyone
    // with moderately elevated load gets rested.
    let jaded_gate: i16 = match best_sports_sci {
        12..=13 => 8500,
        14..=15 => 7800,
        16..=17 => 7000,
        _ => 6200,
    };
    let condition_gate: u32 = match best_sports_sci {
        12..=13 => 55,
        14..=15 => 60,
        16..=17 => 65,
        _ => 70,
    };

    for player in players.iter_mut() {
        if player.player_attributes.is_injured {
            continue;
        }
        // Already resting? Renew the status so it sticks through the week.
        let statuses = player.statuses.get();
        let already_resting = statuses.contains(&PlayerStatusType::Rst);

        let needs_rest = player.player_attributes.jadedness >= jaded_gate
            || player.player_attributes.condition_percentage() < condition_gate;

        if needs_rest && !already_resting {
            player.statuses.add(date, PlayerStatusType::Rst);
        } else if !needs_rest && already_resting {
            // Player has recovered — clear the flag.
            player.statuses.remove(PlayerStatusType::Rst);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TeamType {
    Main = 0,
    B = 1,
    Reserve = 2,
    U18 = 3,
    U19 = 4,
    U20 = 5,
    U21 = 6,
    U23 = 7,
}

impl TeamType {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            TeamType::Main => "first_team",
            TeamType::B => "b_team",
            TeamType::Reserve => "reserve_team",
            TeamType::U18 => "under_18s",
            TeamType::U19 => "under_19s",
            TeamType::U20 => "under_20s",
            TeamType::U21 => "under_21s",
            TeamType::U23 => "under_23s",
        }
    }

    /// Maximum player age allowed on this team type (None = no limit)
    pub fn max_age(&self) -> Option<u8> {
        match self {
            TeamType::U18 => Some(18),
            TeamType::U19 => Some(19),
            _ => None,
        }
    }

    /// Youth team progression order: U18 → U19 → U20 → U21 → U23
    pub const YOUTH_PROGRESSION: &'static [TeamType] = &[
        TeamType::U18,
        TeamType::U19,
        TeamType::U20,
        TeamType::U21,
        TeamType::U23,
    ];
}

impl fmt::Display for TeamType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TeamType::Main => write!(f, "First team"),
            TeamType::B => write!(f, "B Team"),
            TeamType::Reserve => write!(f, "Reserve team"),
            TeamType::U18 => write!(f, "U18"),
            TeamType::U19 => write!(f, "U19"),
            TeamType::U20 => write!(f, "U20"),
            TeamType::U21 => write!(f, "U21"),
            TeamType::U23 => write!(f, "U23"),
        }
    }
}

impl FromStr for TeamType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Main" => Ok(TeamType::Main),
            "B" => Ok(TeamType::B),
            "Reserve" => Ok(TeamType::Reserve),
            "U18" => Ok(TeamType::U18),
            "U19" => Ok(TeamType::U19),
            "U20" => Ok(TeamType::U20),
            "U21" => Ok(TeamType::U21),
            "U23" => Ok(TeamType::U23),
            _ => Err(format!("'{}' is not a valid value for WSType", s)),
        }
    }
}
