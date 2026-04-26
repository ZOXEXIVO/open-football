use crate::club::relations::ChemistryContext;
use crate::club::team::behaviour::TeamBehaviour;
use crate::club::team::builder::TeamBuilder;
use crate::club::team::mentorship::process_mentorship;
use crate::context::GlobalContext;
use crate::shared::CurrencyValue;
use crate::utils::DateUtils;
use crate::club::team::reputation::{
    Achievement, CompetitionType, MatchOutcome, MatchResultInfo,
};
use crate::{
    HappinessEventType, MatchHistory, MatchTacticType, Player, PlayerCollection,
    PlayerFieldPositionGroup, PlayerSquadStatus, PlayerStatusType, StaffCollection, Tactics,
    TacticsSelector, TeamReputation, TeamResult, TeamTraining, TrainingSchedule, TransferItem,
    Transfers,
};
use chrono::NaiveDate;
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

    /// Appointed captain — wears the armband. Selected monthly by
    /// `assign_captaincy` based on leadership, loyalty, and tenure.
    /// Distinct from the emergent "influence leader" used elsewhere.
    pub captain_id: Option<u32>,
    /// Stand-in captain when the captain is unavailable (injured / benched).
    pub vice_captain_id: Option<u32>,
    /// Sticky flag flipped to true the first time `assign_captaincy`
    /// successfully picks a captain on this team. Used to suppress the
    /// false `CaptaincyAwarded` event that would otherwise fire on the
    /// game's very first monthly tick — that's just save-file setup, not
    /// a managerial decision the player made or experienced.
    pub captaincy_initialized: bool,
}

impl Team {
    pub fn builder() -> TeamBuilder {
        TeamBuilder::new()
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> TeamResult {
        // Recalculate squad statuses monthly (1st of month)
        if ctx.simulation.is_month_beginning() {
            self.update_squad_statuses(ctx.simulation.date.date());
            // Reappoint the captaincy at the same cadence — prevents the
            // armband drifting off a retiring veteran or onto a newcomer
            // who hasn't earned it yet.
            self.assign_captaincy(ctx.simulation.date.date());
        }

        // Weekly mentorship pass — pair senior players with juniors. Runs
        // before player development so any personality drift from mentoring
        // is already visible when weekly skill growth is computed.
        if ctx.simulation.is_week_beginning() {
            let week_date = ctx.simulation.date.date();
            let hoy_wwy = self.staffs.best_youth_development_wwy(10);
            let _pairings = process_mentorship(
                &mut self.players.players,
                week_date,
                hoy_wwy,
            );

            // Weekly social decay. Without this, every relationship and
            // rapport entry that ever fired stays at its peak forever —
            // squads wouldn't naturally drift toward neutral when contact
            // fades. Relations decay toward neutral if interaction was
            // light; rapport drifts to 0 after 21+ days of no training
            // contact. Runs before any new weekly relationship event so
            // today's events overwrite the decayed baseline.
            for player in self.players.players.iter_mut() {
                player.relations.process_weekly_update(week_date);
                player.rapport.decay(week_date);
            }

            // Squad-wide chemistry refresh. The per-relation update inside
            // each player's `process_weekly_update` recalculates a local
            // (per-player) view of chemistry but can't see captain,
            // leadership, turnover. We now feed those squad-level signals
            // back to every player so they all share a coherent chemistry
            // number — the one read by training, match rating, selection.
            let chem_ctx = build_chemistry_context(self, week_date);
            for player in self.players.players.iter_mut() {
                player
                    .relations
                    .recalculate_chemistry_with_context(&chem_ctx);
            }

            // Weekly physio preventive-rest pass. Elite sports-science staff
            // can predict which players are heading into the injury danger
            // zone (high jadedness + low condition) and flag them with Rst
            // so the selection logic leaves them out. Beyond FM: this is an
            // explicit, visible mechanism instead of an opaque "rest" hint.
            let best_sports_sci = self.staffs.best_sports_science();
            apply_preventive_rest(
                &mut self.players.players,
                best_sports_sci,
                week_date,
            );
        }

        // Pick (or keep) the team tactic before simulating players so the
        // player context carries the right formation for role-fit checks.
        if self.tactics.is_none() {
            self.tactics = Some(TacticsSelector::select(self, self.staffs.head_coach()));
        };

        let mut player_ctx = ctx.with_team_reputation(self.id, self.reputation.overall_score());
        if let (Some(team_ctx), Some(tac)) = (player_ctx.team.as_mut(), self.tactics.as_ref()) {
            team_ctx.formation = Some(*tac.positions());
        }

        let result = TeamResult::new(
            self.id,
            self.players.simulate(player_ctx.with_player(None)),
            self.staffs.simulate(ctx.with_staff(None)),
            self.behaviour
                .simulate(&mut self.players, &mut self.staffs, ctx.with_team(self.id)),
            TeamTraining::train(self, ctx.simulation.date, ctx.club_facilities_training()),
        );

        if self.training_schedule.is_default {
            //let coach = self.staffs.head_coach();
        }

        result
    }

    /// Assign squad status based on CA rank **within the player's own
    /// position group**. Ranking against the whole squad puts a backup
    /// goalkeeper at the bottom of a CA-sorted list dominated by
    /// outfield stars — you'd get `NotNeeded` for every 3rd/4th keeper
    /// at an elite club, and every downstream code path keyed on
    /// squad status would treat them as surplus.
    fn update_squad_statuses(&mut self, date: chrono::NaiveDate) {
        use std::collections::HashMap;

        let mut by_group: HashMap<PlayerFieldPositionGroup, Vec<u8>> = HashMap::new();
        for p in self.players.iter() {
            let g = p.position().position_group();
            by_group
                .entry(g)
                .or_default()
                .push(p.player_attributes.current_ability);
        }
        for cas in by_group.values_mut() {
            cas.sort_unstable_by(|a, b| b.cmp(a));
        }

        for player in self.players.iter_mut() {
            let group = player.position().position_group();
            let ca = player.player_attributes.current_ability;
            let age = DateUtils::age(player.birth_date, date);
            if let Some(ref mut contract) = player.contract {
                let group_cas = by_group
                    .get(&group)
                    .map(|v| v.as_slice())
                    .unwrap_or(&[]);
                contract.squad_status = PlayerSquadStatus::calculate(ca, age, group_cas);
            }
        }
    }

    pub fn players(&self) -> Vec<&Player> {
        self.players.players()
    }

    /// Rank squad by leadership × loyalty × tenure × reputation and pin
    /// the top scorer as captain, second as vice. Captaincy changes carry
    /// morale consequences: a stripped former captain takes a hit, a new
    /// appointee gets a lift.
    ///
    /// Three guards keep the events realistic:
    /// 1. The very first captain pick on a freshly-loaded team is silent
    ///    (`captaincy_initialized` flag) — it's save-file setup, not a
    ///    decision the player remembers.
    /// 2. A 120-day cooldown on each emit type prevents recalculation
    ///    oscillation from spamming armband-handover events.
    /// 3. If the previous captain has left the squad (transfer / loan
    ///    out / retirement), no `CaptaincyRemoved` is fired for them —
    ///    the move itself is what unsettled them, not "stripping the
    ///    armband" they no longer wear at this club.
    pub fn assign_captaincy(&mut self, date: chrono::NaiveDate) {
        use chrono::Datelike;

        let now_year = date.year();
        let mut ranked: Vec<(u32, f32)> = self
            .players
            .iter()
            .filter(|p| p.skills.mental.leadership >= 8.0)
            .filter_map(|p| {
                let Some(contract) = p.contract.as_ref() else { return None };
                let tenure_years = contract
                    .started
                    .map(|s| (now_year - s.year()).max(0) as f32)
                    .unwrap_or(0.0);
                let age = DateUtils::age(p.birth_date, date) as f32;
                // Age bell curve: peak captaincy fitness around 29-31.
                let age_factor = if age < 23.0 {
                    0.5
                } else if age >= 23.0 && age <= 34.0 {
                    1.0 + ((age - 28.0).abs() * -0.05).max(-0.25)
                } else {
                    0.7
                };
                let score = p.skills.mental.leadership * 1.5
                    + p.attributes.loyalty * 0.8
                    + p.attributes.professionalism * 0.4
                    + tenure_years.min(10.0) * 0.6
                    + p.player_attributes.current_reputation as f32 / 2500.0;
                Some((p.id, score * age_factor))
            })
            .collect();

        if ranked.is_empty() {
            self.captain_id = None;
            self.vice_captain_id = None;
            return;
        }

        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let new_captain = ranked.first().map(|(id, _)| *id);
        let new_vice = ranked.get(1).map(|(id, _)| *id);
        let was_initialized = self.captaincy_initialized;

        // Persistent state always updates regardless of whether we emit.
        // Captaincy events are about *narrative*; the underlying field is
        // the source of truth for the next match-day squad selection.
        if self.captain_id != new_captain && was_initialized {
            // Strip event for the outgoing captain — only if they're
            // still in the squad. A captain who left the club should not
            // get a `CaptaincyRemoved` event applied to their morale at
            // their next club; the transfer pipeline handles the move
            // itself, and pinning a "stripped of armband" event on a
            // departed player would be doubly wrong.
            if let Some(old_id) = self.captain_id {
                if let Some(p) = self.players.players.iter_mut().find(|p| p.id == old_id) {
                    let mag = Self::captaincy_removed_magnitude(p);
                    // 120-day cooldown to absorb monthly recalculation
                    // oscillation around an evenly-matched leadership
                    // group (the kind of churn we don't want narrated).
                    p.happiness.add_event_with_cooldown(
                        HappinessEventType::CaptaincyRemoved,
                        mag,
                        120,
                    );
                }
            }
            if let Some(new_id) = new_captain {
                if let Some(p) = self.players.players.iter_mut().find(|p| p.id == new_id) {
                    let mag = Self::captaincy_awarded_magnitude(p);
                    p.happiness.add_event_with_cooldown(
                        HappinessEventType::CaptaincyAwarded,
                        mag,
                        120,
                    );
                }
            }
        }

        self.captain_id = new_captain;
        self.vice_captain_id = new_vice;
        self.captaincy_initialized = true;
    }

    /// Magnitude for `CaptaincyAwarded`. Catalog default amplified by the
    /// player's leadership traits, loyalty, reputation, and tempered by
    /// age (a 19-year-old handed the armband feels it less viscerally
    /// than a 30-year-old club legend who's earned it). Returns a value
    /// near the catalog default (7.0) but in the band ~5..10.
    fn captaincy_awarded_magnitude(p: &Player) -> f32 {
        let cfg = crate::club::player::behaviour_config::HappinessConfig::default();
        let base = cfg.catalog.captaincy_awarded;
        // Leadership + loyalty drive how much a player wanted this.
        let leadership_lift =
            (p.skills.mental.leadership.clamp(0.0, 20.0) / 20.0) * 0.30;
        let loyalty_lift = (p.attributes.loyalty.clamp(0.0, 20.0) / 20.0) * 0.20;
        // Reputation amplifier — a star getting the armband at a marquee
        // club feels it carry more weight (pressure plus prestige).
        let rep_lift = (p.player_attributes.current_reputation as f32 / 10_000.0)
            .clamp(0.0, 1.0)
            * 0.20;
        let mul = (1.0 + leadership_lift + loyalty_lift + rep_lift).clamp(0.7, 1.6);
        base * mul
    }

    /// Magnitude for `CaptaincyRemoved`. Catalog default (-7.0)
    /// amplified by reputation and reactive personality (controversy /
    /// low temperament read this as a public humiliation), softened by
    /// professionalism (high-pro players keep it together).
    fn captaincy_removed_magnitude(p: &Player) -> f32 {
        let cfg = crate::club::player::behaviour_config::HappinessConfig::default();
        let base = cfg.catalog.captaincy_removed;
        let rep_amp =
            crate::club::player::events::scaling::reputation_amplifier(
                p.player_attributes.current_reputation,
            );
        let provoke_amp = crate::club::player::events::scaling::criticism_amplifier(
            p.attributes.controversy,
            p.attributes.temperament,
        );
        let prof_dampen =
            crate::club::player::events::scaling::criticism_dampener(p.attributes.professionalism);
        // base is negative; multiplying by these factors keeps the sign.
        base * rep_amp * provoke_amp * prof_dampen
    }

    pub fn add_player_to_transfer_list(&mut self, player_id: u32, value: CurrencyValue) {
        self.transfer_list.add(TransferItem {
            player_id,
            amount: value,
        })
    }

    /// Annual player wage bill at this team. Staff are billed separately by
    /// `Club::process_monthly_finances` via `StaffCollection::get_annual_salary`
    /// — including them here would double-count.
    ///
    /// Loan accounting:
    ///   - Loaned-IN players (contract_loan.is_some()): the borrower's
    ///     payroll line is the loan contract salary, not the parent
    ///     contract. The parent's residual share is accepted as zero on
    ///     the parent side — when a player is loaned out they leave the
    ///     parent's roster, so the parent's wage bill drops by their full
    ///     contract for the duration of the loan.
    ///   - Other players: parent contract salary as installed.
    pub fn get_annual_salary(&self) -> u32 {
        self.players
            .players
            .iter()
            .filter_map(|p| {
                if let Some(loan) = p.contract_loan.as_ref() {
                    Some(loan.salary)
                } else {
                    p.contract.as_ref().map(|c| c.salary)
                }
            })
            .sum()
    }

    pub fn tactics(&self) -> Cow<'_, Tactics> {
        if let Some(tactics) = &self.tactics {
            Cow::Borrowed(tactics)
        } else {
            Cow::Owned(Tactics::new(MatchTacticType::T442))
        }
    }

    /// React to a completed competitive match: feed the result into the
    /// reputation drift model. Caller supplies the opponent's overall
    /// reputation and the team's current league standing so we don't
    /// thread a Country reference in here.
    pub fn on_match_completed(
        &mut self,
        outcome: MatchOutcome,
        opponent_reputation: u16,
        competition: CompetitionType,
        league_position: u8,
        total_teams: u8,
        date: NaiveDate,
    ) {
        let info = MatchResultInfo {
            outcome,
            opponent_reputation,
            competition_type: competition,
        };
        self.reputation
            .process_weekly_update(&[info], league_position, total_teams, date);
    }

    /// Monthly decay pass — reputation softly drifts down without fresh
    /// achievements. Called on the 1st of each month.
    pub fn on_month_tick(&mut self) {
        self.reputation.apply_monthly_decay();
    }

    /// Record a season-end trophy/promotion/qualification event, feeding
    /// the reputation model so title wins stick to the club for years.
    pub fn on_season_trophy(&mut self, achievement: Achievement) {
        self.reputation.process_achievement(achievement);
    }
}

/// Build the squad-wide [`ChemistryContext`] consumed by every player's
/// chemistry recalculation this week. Centralised here so all players see
/// the same captain / leadership / turnover signals — otherwise per-player
/// chemistry numbers would drift apart and downstream consumers (training
/// chemistry multiplier, match-rating shift, selection cohesion) would
/// disagree on the dressing room mood.
fn build_chemistry_context(team: &Team, today: chrono::NaiveDate) -> ChemistryContext {
    use std::collections::HashMap;

    // Top-3 leadership scores. Raw 0..20 attribute (skills.mental.leadership).
    let mut leadership: Vec<f32> = team
        .players
        .players
        .iter()
        .map(|p| p.skills.mental.leadership.clamp(0.0, 20.0))
        .collect();
    leadership.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let top_leadership_scores = leadership.into_iter().take(3).collect();

    // Top-3 influence scores — sum of `relation.influence` references TO
    // each player from every other player. Captures dressing-room standing
    // distinct from raw leadership.
    let mut influence_totals: HashMap<u32, f32> = HashMap::new();
    for p in team.players.players.iter() {
        for (id, rel) in p.relations.player_relations_iter() {
            *influence_totals.entry(*id).or_insert(0.0) += rel.influence;
        }
    }
    let mut influences: Vec<f32> = influence_totals.into_values().collect();
    influences.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let top_influence_scores = influences.into_iter().take(3).collect();

    // Recent signings — anyone whose last_transfer_date is within 90 days.
    let cutoff = today - chrono::Duration::days(90);
    let recent_signings_90d = team
        .players
        .players
        .iter()
        .filter(|p| p.last_transfer_date.map(|d| d >= cutoff).unwrap_or(false))
        .count()
        .min(u8::MAX as usize) as u8;

    // Average inner-circle cohesion across the squad — a coarse signal of
    // how clique-y / cohesive the dressing room feels.
    let cohesion_avg: f32 = if team.players.players.is_empty() {
        0.0
    } else {
        team.players
            .players
            .iter()
            .map(|p| p.relations.inner_circle_cohesion())
            .sum::<f32>()
            / team.players.players.len() as f32
    };

    ChemistryContext {
        top_leadership_scores,
        top_influence_scores,
        captain_id: team.captain_id,
        vice_captain_id: team.vice_captain_id,
        recent_signings_90d,
        inner_circle_cohesion: cohesion_avg,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TeamType {
    Main = 0,
    B = 1,
    Reserve = 2,
    U18 = 3,
    U19 = 4,
    U20 = 5,
    U21 = 6,
    U23 = 7,
    /// Senior reserve squad that competes in a real lower division under the
    /// "{Club} 2" naming convention (e.g. "Ural 2", "Zenit 2"). Behaves like
    /// `B` in most respects (senior bracket, finance/transfer/staff handling)
    /// but renders as the suffix "2" so the team name reads naturally.
    Second = 8,
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
            TeamType::Second => "second_team",
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

    /// Senior squads that compete in a real league under their own brand.
    /// Used by the player-history pipeline (so a B/Second player's stats
    /// show their actual team and league) and by the web layer to decide
    /// when a team gets its own finances/transfers/etc. tabs.
    ///
    /// Reserve is intentionally excluded: it shares the Main team's brand
    /// and plays in a synthetic sub-league.
    pub fn is_own_team(&self) -> bool {
        matches!(self, TeamType::Main | TeamType::B | TeamType::Second)
    }

    /// Menu-row label for a team grouped under its parent club. Senior
    /// reserves (B, Second) carry their own canonical name like
    /// "Spartak Moscow 2" or "Ural B Team", so the row shows the team
    /// name as-is. Everything else (Main, Reserve, U18..U23) renders as
    /// "{Club}  |  {i18n type label}", e.g. "Spartak Moscow | First team".
    pub fn menu_label(&self, club_name: &str, team_name: &str, i18n_type_label: &str) -> String {
        match self {
            TeamType::B | TeamType::Second => team_name.to_string(),
            _ => format!("{}  |  {}", club_name, i18n_type_label),
        }
    }

    /// Sort priority for the parent club's left-menu listing. Lower comes
    /// first, so Main appears at the top, Second right after, then B,
    /// Reserve, and youth squads in descending age. Reputation tiebreaks
    /// between teams of the same type within a single club (rare).
    pub fn menu_order(&self) -> u8 {
        match self {
            TeamType::Main => 0,
            TeamType::Second => 1,
            TeamType::B => 2,
            TeamType::Reserve => 3,
            TeamType::U23 => 4,
            TeamType::U21 => 5,
            TeamType::U20 => 6,
            TeamType::U19 => 7,
            TeamType::U18 => 8,
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
            // Renders as " Team" so the runtime team-name formula
            // `format!("{} {}", t.name, team_type)` turns the satellite-curated
            // "Spartak Moscow 2" into "Spartak Moscow 2 Team" without
            // double-tagging the digit. The "2" lives in the data, the suffix
            // lives in the type.
            TeamType::Second => write!(f, "Team"),
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
            "Second" => Ok(TeamType::Second),
            _ => Err(format!("'{}' is not a valid value for WSType", s)),
        }
    }
}

#[cfg(test)]
mod payroll_tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerClubContract, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills,
    };
    use chrono::NaiveTime;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn make_player(id: u32, salary: u32) -> Player {
        let mut p = PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Test".into(), format!("Wager{}", id)))
            .birth_date(d(1995, 1, 1))
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 20,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap();
        p.contract = Some(PlayerClubContract::new(salary, d(2030, 6, 30)));
        p
    }

    fn build_team_with(players: Vec<Player>) -> Team {
        TeamBuilder::new()
            .id(1)
            .league_id(Some(1))
            .club_id(1)
            .name("Test FC".into())
            .slug("test-fc".into())
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(100, 100, 200))
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            ))
            .build()
            .unwrap()
    }

    #[test]
    fn get_annual_salary_does_not_include_staff() {
        // Two players, no staff: total = sum of player salaries.
        let team = build_team_with(vec![make_player(1, 100_000), make_player(2, 80_000)]);
        assert_eq!(team.get_annual_salary(), 180_000);
    }

    #[test]
    fn get_annual_salary_uses_loan_contract_for_loaned_in_player() {
        // Loaned-in player: borrower's payroll is the loan contract,
        // not the parent contract. Without this fix the borrower would
        // be billed the full parent salary every month.
        let mut p = make_player(7, 500_000);
        // Install a loan contract simulating the borrower side.
        let mut loan = PlayerClubContract::new_loan(120_000, d(2027, 6, 30), 1, 1, 2);
        loan.salary = 120_000;
        p.contract_loan = Some(loan);
        let team = build_team_with(vec![p]);
        // 120k loan, not 500k parent.
        assert_eq!(team.get_annual_salary(), 120_000);
    }

    #[test]
    fn wage_structure_uses_loan_salary_for_loanees_not_parent() {
        // Borrower has one permanent player (100k) and one loaned-in
        // player whose parent contract is 1M but loan contract is just
        // 100k. Top-earner must NOT be 1M — that would let the renewal
        // AI argue "we already pay 1M" against permanent squad members.
        use crate::club::team::squad::WageStructureSnapshot;

        let mut perm = make_player(1, 100_000);
        // Mark as KeyPlayer so it counts in the first_team bucket.
        if let Some(c) = perm.contract.as_mut() {
            c.squad_status = PlayerSquadStatus::KeyPlayer;
        }

        let mut loanee = make_player(2, 1_000_000);
        let mut loan = PlayerClubContract::new_loan(100_000, d(2027, 6, 30), 99, 1, 1);
        loan.salary = 100_000;
        loanee.contract_loan = Some(loan);
        if let Some(c) = loanee.contract.as_mut() {
            c.squad_status = PlayerSquadStatus::FirstTeamRegular;
        }

        let team = build_team_with(vec![perm, loanee]);
        let snap = WageStructureSnapshot::from_team(&team);
        // Top earner is 100k (permanent player), NOT the loanee's 1M
        // parent contract.
        assert_eq!(snap.top_earner, 100_000);
        // Wage bill is 100k + 100k (loan), not 100k + 1M.
        assert_eq!(snap.current_bill, 200_000);
    }
}

#[cfg(test)]
mod captaincy_tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerClubContract, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills,
    };
    use chrono::NaiveTime;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn build_leader(id: u32, leadership: f32, reputation: i16) -> Player {
        let mut skills = PlayerSkills::default();
        skills.mental.leadership = leadership;
        let mut attrs = PlayerAttributes::default();
        attrs.current_reputation = reputation;
        let mut contract = PlayerClubContract::new(20_000, d(2035, 6, 30));
        contract.started = Some(d(2020, 7, 1));
        let mut p = PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Test".into(), format!("Leader{}", id)))
            .birth_date(d(1996, 1, 1)) // age ~30 by 2026 — peak captaincy band
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 20,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        p.contract = Some(contract);
        p
    }

    fn build_team_with(players: Vec<Player>) -> Team {
        TeamBuilder::new()
            .id(1)
            .league_id(Some(1))
            .club_id(1)
            .name("Test FC".into())
            .slug("test-fc".into())
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(100, 100, 200))
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            ))
            .build()
            .unwrap()
    }

    fn captaincy_event_count(p: &Player, kind: &HappinessEventType) -> usize {
        p.happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == *kind)
            .count()
    }

    #[test]
    fn initial_captain_assignment_is_silent() {
        let p1 = build_leader(1, 18.0, 5_000);
        let p2 = build_leader(2, 14.0, 3_000);
        let mut team = build_team_with(vec![p1, p2]);

        assert!(!team.captaincy_initialized);
        team.assign_captaincy(d(2026, 7, 1));
        assert!(team.captaincy_initialized);
        // Captain set, but no narrative event fires on first run.
        assert!(team.captain_id.is_some());
        for player in team.players.players.iter() {
            assert_eq!(
                captaincy_event_count(player, &HappinessEventType::CaptaincyAwarded),
                0
            );
            assert_eq!(
                captaincy_event_count(player, &HappinessEventType::CaptaincyRemoved),
                0
            );
        }
    }

    #[test]
    fn replacing_existing_captain_emits_both_events() {
        let p1 = build_leader(1, 14.0, 3_000);
        let p2 = build_leader(2, 10.0, 2_000);
        let mut team = build_team_with(vec![p1, p2]);

        // Initial silent assignment.
        team.assign_captaincy(d(2026, 7, 1));
        let original_captain = team.captain_id.unwrap();

        // Bump the other player's leadership to overtake. Need to also
        // depress current captain's score so the rank flips deterministically.
        for p in team.players.players.iter_mut() {
            if p.id != original_captain {
                p.skills.mental.leadership = 20.0;
                p.player_attributes.current_reputation = 9_000;
            } else {
                p.skills.mental.leadership = 9.0;
            }
        }
        team.assign_captaincy(d(2026, 8, 1));

        let new_captain = team.captain_id.unwrap();
        assert_ne!(new_captain, original_captain);

        let outgoing = team
            .players
            .players
            .iter()
            .find(|p| p.id == original_captain)
            .unwrap();
        let incoming = team
            .players
            .players
            .iter()
            .find(|p| p.id == new_captain)
            .unwrap();
        assert_eq!(
            captaincy_event_count(outgoing, &HappinessEventType::CaptaincyRemoved),
            1
        );
        assert_eq!(
            captaincy_event_count(incoming, &HappinessEventType::CaptaincyAwarded),
            1
        );
    }

    #[test]
    fn departed_captain_does_not_get_removed_event() {
        let p1 = build_leader(1, 14.0, 3_000);
        let p2 = build_leader(2, 12.0, 2_500);
        let mut team = build_team_with(vec![p1, p2]);

        team.assign_captaincy(d(2026, 7, 1));
        let original_captain = team.captain_id.unwrap();

        // Captain "leaves" the squad — remove them from the player list
        // but leave `team.captain_id` stale, simulating the small window
        // between transfer execution and the next monthly recalc.
        team.players.players.retain(|p| p.id != original_captain);

        // Re-assign — the new appointment fires for the surviving leader,
        // but no `CaptaincyRemoved` event must be applied to the absent
        // player (they're not on this team to receive it anyway, and we
        // explicitly check the loop doesn't ghost-emit).
        team.assign_captaincy(d(2026, 8, 1));

        for player in team.players.players.iter() {
            // Survivor may receive `CaptaincyAwarded` if they're now the
            // pick, but they should never see `CaptaincyRemoved`.
            assert_eq!(
                captaincy_event_count(player, &HappinessEventType::CaptaincyRemoved),
                0
            );
        }
    }

    #[test]
    fn captaincy_cooldown_blocks_oscillation() {
        let p1 = build_leader(1, 14.0, 3_000);
        let p2 = build_leader(2, 10.0, 2_000);
        let mut team = build_team_with(vec![p1, p2]);

        team.assign_captaincy(d(2026, 7, 1)); // silent init
        let first_captain = team.captain_id.unwrap();

        // Flip leadership so other player wins.
        for p in team.players.players.iter_mut() {
            if p.id == first_captain {
                p.skills.mental.leadership = 9.0;
            } else {
                p.skills.mental.leadership = 20.0;
                p.player_attributes.current_reputation = 9_000;
            }
        }
        team.assign_captaincy(d(2026, 8, 1));

        // Flip back the very next month — within the 120d cooldown.
        for p in team.players.players.iter_mut() {
            if p.id == first_captain {
                p.skills.mental.leadership = 20.0;
                p.player_attributes.current_reputation = 9_000;
            } else {
                p.skills.mental.leadership = 9.0;
            }
        }
        team.assign_captaincy(d(2026, 9, 1));

        // Each player should have at most one `CaptaincyAwarded` event
        // — the cooldown absorbs the second handover.
        for player in team.players.players.iter() {
            let awarded =
                captaincy_event_count(player, &HappinessEventType::CaptaincyAwarded);
            assert!(awarded <= 1, "expected ≤1 award, got {} for player {}", awarded, player.id);
        }
    }

    #[test]
    fn high_reputation_removed_captain_takes_bigger_hit() {
        // Two parallel teams: one star captain (reputation 9000), one
        // anonymous captain (reputation 500). Both get displaced; star's
        // hit should be more negative due to reputation amplification.
        fn run(rep: i16) -> f32 {
            let mut p1 = build_leader(1, 14.0, rep);
            // Mild personality — keep reputation as the dominant axis.
            p1.attributes.controversy = 10.0;
            p1.attributes.temperament = 10.0;
            p1.attributes.professionalism = 10.0;
            let p2 = build_leader(2, 10.0, 1_000);
            let mut team = build_team_with(vec![p1, p2]);
            team.assign_captaincy(d(2026, 7, 1));
            let captain = team.captain_id.unwrap();
            // Flip ranks.
            for p in team.players.players.iter_mut() {
                if p.id == captain {
                    p.skills.mental.leadership = 9.0;
                } else {
                    p.skills.mental.leadership = 20.0;
                    p.player_attributes.current_reputation = 9_000;
                }
            }
            team.assign_captaincy(d(2026, 8, 1));
            team.players
                .players
                .iter()
                .find(|p| p.id == captain)
                .unwrap()
                .happiness
                .recent_events
                .iter()
                .find(|e| e.event_type == HappinessEventType::CaptaincyRemoved)
                .unwrap()
                .magnitude
        }
        let star = run(9_000);
        let anon = run(500);
        assert!(star < anon, "star {} should be more negative than anon {}", star, anon);
    }
}
