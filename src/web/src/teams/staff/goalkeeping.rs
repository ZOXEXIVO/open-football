//! The goalkeeping department, as the staff page shows it.
//!
//! The department's plan spans every squad the club owns, so it does not
//! belong on any one squad's page — but the first team's staff page is where
//! the man who wrote it is listed, and it is the page a user opens to ask
//! "who is actually running the keepers here". So the panel renders there:
//! the declared order, the succession clock, and what the specialist is
//! currently telling the manager.

use core::club::staff::goalkeeping::{KeeperAdvice, KeeperTier, KeeperUrgency};
use core::{SimulatorData, Team, TeamType};

/// One keeper in the room, ready to render.
pub struct KeeperRow {
    pub player_id: u32,
    pub name: String,
    pub squad: String,
    pub squad_slug: String,
    pub age: u8,
    /// i18n key for his declared standing.
    pub tier_key: &'static str,
    /// The keeper being groomed for the shirt.
    pub is_heir: bool,
    /// The keeper the specialist is currently asking the manager to play.
    pub is_nominated: bool,
    /// Senior competitive appearances this season.
    pub senior_apps: u16,
    /// Share of the season the plan intends for him, as a percentage.
    pub planned_share: u8,
}

/// One thing the specialist is telling the manager.
pub struct KeeperAdviceRow {
    pub advice_key: &'static str,
    pub player_id: Option<u32>,
    pub player_name: String,
    /// CSS modifier for how hard he is pushing.
    pub urgency_class: &'static str,
}

/// The whole panel, or `None` when the club has no department yet.
pub struct GoalkeepingDepartmentView {
    /// The specialist, when the club employs one. Absent means the manager
    /// is running the keeper room himself — worth saying out loud.
    pub coach_id: Option<u32>,
    pub coach_name: String,
    /// How far the manager acts on the department's word, as a percentage.
    pub authority: u8,
    pub succession_key: &'static str,
    pub rows: Vec<KeeperRow>,
    pub advice: Vec<KeeperAdviceRow>,
}

impl GoalkeepingDepartmentView {
    /// Build the panel for `team`. Only the first team carries it: the plan
    /// is the club's, and showing the same table on four squad pages would
    /// suggest four plans.
    pub fn build(data: &SimulatorData, team: &Team) -> Option<Self> {
        if team.team_type != TeamType::Main {
            return None;
        }
        let club = data.club(team.club_id)?;
        let today = data.date.date();
        let plan = club.keeper_plan()?;
        let room = club.keeper_room(today);

        let specialist = team.staffs.goalkeeper_coach();
        let coach_name = specialist
            .map(|s| s.full_name.to_string())
            .unwrap_or_else(|| team.staffs.head_coach_name());

        let nominated = plan.nominated(today);
        let heir = plan.heir();

        let mut rows: Vec<KeeperRow> = Vec::with_capacity(room.len());
        for keeper in room.iter() {
            let Some(tier) = plan.tier_of(keeper.player_id) else {
                continue;
            };
            let Some(squad) = club.teams.teams.iter().find(|t| t.id == keeper.team_id) else {
                continue;
            };
            let Some(player) = squad.players.iter().find(|p| p.id == keeper.player_id) else {
                continue;
            };
            rows.push(KeeperRow {
                player_id: keeper.player_id,
                name: Self::display_name(&player.full_name.last_name, &player.full_name.first_name),
                squad: squad.name.clone(),
                squad_slug: squad.slug.clone(),
                age: keeper.age,
                tier_key: tier.as_i18n_key(),
                is_heir: heir == Some(keeper.player_id),
                is_nominated: nominated == Some(keeper.player_id),
                senior_apps: keeper.senior_apps,
                planned_share: (tier.planned_share() * 100.0).round() as u8,
            });
        }
        // The declared order first, then everyone else as the room reads.
        rows.sort_by_key(|r| Self::tier_order(r.tier_key));

        let name_of = |player_id: u32| -> String {
            club.teams
                .teams
                .iter()
                .flat_map(|t| t.players.iter())
                .find(|p| p.id == player_id)
                .map(|p| Self::display_name(&p.full_name.last_name, &p.full_name.first_name))
                .unwrap_or_default()
        };

        let advice: Vec<KeeperAdviceRow> = plan
            .recommendations()
            .iter()
            // The cup designation and "leave him alone" are the department's
            // routine bookkeeping, not news. The panel shows what the
            // manager is actually being asked to decide.
            .filter(|r| {
                !matches!(
                    r.advice,
                    KeeperAdvice::HandHimTheCup | KeeperAdvice::KeepHimDeveloping
                )
            })
            .map(|r| KeeperAdviceRow {
                advice_key: r.advice.as_i18n_key(),
                player_id: r.player_id,
                player_name: r.player_id.map(name_of).unwrap_or_default(),
                urgency_class: Self::urgency_class(r.urgency),
            })
            .collect();

        if rows.is_empty() && advice.is_empty() {
            return None;
        }

        Some(GoalkeepingDepartmentView {
            coach_id: specialist.map(|s| s.id),
            coach_name,
            authority: (plan.authority() * 100.0).round().clamp(0.0, 100.0) as u8,
            succession_key: plan.succession().as_i18n_key(),
            rows,
            advice,
        })
    }

    fn display_name(last: &str, first: &str) -> String {
        if first.is_empty() {
            last.to_string()
        } else {
            format!("{last} {first}")
        }
    }

    fn tier_order(tier_key: &str) -> u8 {
        match tier_key {
            k if k == KeeperTier::NumberOne.as_i18n_key() => 0,
            k if k == KeeperTier::Deputy.as_i18n_key() => 1,
            k if k == KeeperTier::Third.as_i18n_key() => 2,
            k if k == KeeperTier::Pathway.as_i18n_key() => 3,
            k if k == KeeperTier::Academy.as_i18n_key() => 4,
            _ => 5,
        }
    }

    fn urgency_class(urgency: KeeperUrgency) -> &'static str {
        match urgency {
            KeeperUrgency::Urgent => "fm-badge-inj",
            KeeperUrgency::Pressing => "fm-badge-wnt",
            KeeperUrgency::Noted => "fm-badge-yth",
        }
    }
}

/// Convenience so the handler can stay a straight line.
pub struct GoalkeepingPanel;

impl GoalkeepingPanel {
    pub fn for_team(data: &SimulatorData, team: &Team) -> Option<GoalkeepingDepartmentView> {
        GoalkeepingDepartmentView::build(data, team)
    }
}
