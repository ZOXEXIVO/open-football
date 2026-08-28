//! The coach's standing intention for each player in his squad.
//!
//! Everything the coach layer did before this was a *reaction*: a
//! per-fixture strategy, a per-match memory update, a signed nudge folded
//! into one selection. Nothing anywhere recorded what the coach actually
//! meant to do with a man over a season — who he was building around, who
//! he was grooming, whose competition the domestic cup was, and who he had
//! simply stopped counting on. So the squad had no shape and no direction:
//! each week's team was re-derived from ability and form, and a player's
//! standing never changed except as an accident of those two.
//!
//! [`CoachSquadPlan`] is that missing intention, held on the coach (not
//! the player) because it is his opinion, revised monthly and on his
//! arrival — a new manager really does tear up his predecessor's plan.
//!
//! It is deliberately a PLAN, not a mechanism: nothing here moves a
//! player. Consumers read it and decide — selection gives the cup keeper
//! his competition, the renewal desk declines to re-sign a man who is not
//! in the plans, the listing desk knows who is in the shop window. That
//! keeps the plan honest and removable: with an empty plan every consumer
//! falls back to exactly the behaviour it had before.

use std::collections::HashMap;

use chrono::NaiveDate;

use crate::club::person::Person;
use crate::club::player::statistics::StuckCareerScan;
use crate::club::staff::goalkeeping::{KeeperRoomPlan, KeeperTier};
use crate::club::staff::perception::{AbilityEstimator, PotentialEstimator};
use crate::{Player, PlayerCollection, PlayerFieldPositionGroup};

/// What the coach intends to do with a player this season.
///
/// Ordered loosely from most to least central so a consumer can compare
/// standing without a lookup table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlannedRole {
    /// The team is built around him.
    Cornerstone,
    /// In the strongest available XI.
    Starter,
    /// Genuinely rotated — a real share of competitive minutes.
    Rotation,
    /// The deputy whose competition is the domestic cup. A real role with
    /// real matches, and the honest answer for a second-choice keeper
    /// (where only one man can play the league).
    CupKeeper,
    /// Being groomed to take a specific incumbent's place.
    SuccessionHeir,
    /// A prospect on a development path — minutes where the stakes allow.
    DevelopmentPathway,
    /// Squad depth: he plays when others cannot.
    Cover,
    /// The club would do business at the right price.
    ShopWindow,
    /// No future here. The honest end of the ladder, and the one the
    /// player is entitled to be told about.
    NotInPlans,
}

impl PlannedRole {
    /// Roles that commit the coach to giving the player real football.
    /// A plan that promises minutes is a promise the verifier can check.
    pub fn promises_minutes(self) -> bool {
        matches!(
            self,
            Self::Cornerstone
                | Self::Starter
                | Self::Rotation
                | Self::CupKeeper
                | Self::SuccessionHeir
                | Self::DevelopmentPathway
        )
    }

    /// Roles under which the club has no reason to offer fresh terms.
    pub fn is_exit_path(self) -> bool {
        matches!(self, Self::ShopWindow | Self::NotInPlans)
    }

    /// Stable key for the events feed / UI.
    pub fn as_i18n_key(self) -> &'static str {
        match self {
            Self::Cornerstone => "planned_role_cornerstone",
            Self::Starter => "planned_role_starter",
            Self::Rotation => "planned_role_rotation",
            Self::CupKeeper => "planned_role_cup_keeper",
            Self::SuccessionHeir => "planned_role_succession_heir",
            Self::DevelopmentPathway => "planned_role_development_pathway",
            Self::Cover => "planned_role_cover",
            Self::ShopWindow => "planned_role_shop_window",
            Self::NotInPlans => "planned_role_not_in_plans",
        }
    }
}

/// One player's place in the plan.
#[derive(Debug, Clone, Copy)]
pub struct PlayerPlanEntry {
    pub role: PlannedRole,
    /// When the coach last committed to this role — so a consumer can
    /// tell a fresh decision from a standing one, and so a change of
    /// role is visible as an event rather than a silent re-derivation.
    pub set_on: NaiveDate,
    /// For [`PlannedRole::SuccessionHeir`], the incumbent he is being
    /// groomed to replace.
    pub succeeds: Option<u32>,
}

/// Every player the coach currently holds an opinion about.
#[derive(Debug, Clone, Default)]
pub struct CoachSquadPlan {
    entries: HashMap<u32, PlayerPlanEntry>,
    last_revised: Option<NaiveDate>,
}

impl CoachSquadPlan {
    /// How long a plan stands before the coach revisits it.
    const REVISION_DAYS: i64 = 30;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn role_of(&self, player_id: u32) -> Option<PlannedRole> {
        self.entries.get(&player_id).map(|e| e.role)
    }

    pub fn entry(&self, player_id: u32) -> Option<&PlayerPlanEntry> {
        self.entries.get(&player_id)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// True when the standing plan is old enough to be worth revisiting.
    pub fn due_for_revision(&self, today: NaiveDate) -> bool {
        match self.last_revised {
            None => true,
            Some(last) => (today - last).num_days() >= Self::REVISION_DAYS,
        }
    }

    /// A new manager inherits a squad, not a plan.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.last_revised = None;
    }

    /// Re-derive the plan across a squad, returning the players whose role
    /// CHANGED so the caller can tell them.
    ///
    /// Roles are assigned from what the coach can observe — assessed level
    /// and ceiling, age, depth rank, contract, and whether the player's
    /// career here has stalled. Never from hidden ability.
    pub fn revise(
        &mut self,
        players: &PlayerCollection,
        today: NaiveDate,
    ) -> Vec<(u32, PlannedRole)> {
        self.revise_with_keepers(players, None, today)
    }

    /// As [`Self::revise`], but with the goalkeeping department's standing
    /// plan in hand.
    ///
    /// The coach does not rank his keepers himself. There is one shirt, the
    /// queue for it runs across every squad the club owns, and the man who
    /// watches all of them has already written the order down — so where a
    /// keeper room plan exists the coach reads it rather than re-deriving a
    /// pecking order from this squad's depth chart, which cannot see the
    /// under-eighteens and cannot tell a third keeper from a stalled career.
    pub fn revise_with_keepers(
        &mut self,
        players: &PlayerCollection,
        keepers: Option<&KeeperRoomPlan>,
        today: NaiveDate,
    ) -> Vec<(u32, PlannedRole)> {
        let ranks = SquadDepthRanks::build(players, today);
        let mut changes = Vec::new();

        for player in players.iter() {
            // Loanees are their parent club's to plan for.
            if player.is_on_loan() {
                continue;
            }
            let role = Self::keeper_role(player, keepers)
                .unwrap_or_else(|| Self::derive_role(player, &ranks, today));
            let previous = self.entries.get(&player.id).map(|e| e.role);
            if previous != Some(role) {
                changes.push((player.id, role));
            }
            self.entries.insert(
                player.id,
                PlayerPlanEntry {
                    role,
                    set_on: today,
                    succeeds: if role == PlannedRole::SuccessionHeir {
                        ranks.incumbent_of(player)
                    } else {
                        None
                    },
                },
            );
        }

        // Players who have left the squad are no longer the coach's
        // problem — drop them so the plan cannot grow without bound.
        let present: std::collections::HashSet<u32> = players.iter().map(|p| p.id).collect();
        self.entries.retain(|id, _| present.contains(id));

        self.last_revised = Some(today);
        changes
    }

    /// A keeper's role, read off the goalkeeping department's plan.
    ///
    /// `None` for outfielders, for keepers the department has no view on,
    /// and whenever no department exists — the caller then falls back to
    /// the ordinary depth-chart read, exactly as before.
    ///
    /// The mapping is the point. A squad depth chart can only say "third
    /// best keeper here", which for a settled thirty-five-year-old third
    /// choice reads as a stalled career and used to put him in the shop
    /// window. The department says what he actually is: the cover, and the
    /// senior voice in the goalkeeping group.
    fn keeper_role(player: &Player, keepers: Option<&KeeperRoomPlan>) -> Option<PlannedRole> {
        let plan = keepers?;
        if player.position().position_group() != PlayerFieldPositionGroup::Goalkeeper {
            return None;
        }
        // A manager-pinned keeper is in the plans by definition, and the pin
        // outranks the specialist's opinion.
        if player.is_force_match_selection {
            return Some(PlannedRole::Starter);
        }
        if plan.heir() == Some(player.id) {
            return Some(PlannedRole::SuccessionHeir);
        }
        match plan.tier_of(player.id)? {
            KeeperTier::NumberOne => Some(PlannedRole::Starter),
            KeeperTier::Deputy => Some(PlannedRole::CupKeeper),
            KeeperTier::Third => Some(PlannedRole::Cover),
            KeeperTier::Pathway => Some(PlannedRole::DevelopmentPathway),
            // The department is watching him but has not put him on the
            // pathway, and it has no opinion about a keeper it wants moved
            // on. Both fall back to the ordinary read.
            KeeperTier::Academy | KeeperTier::Surplus => None,
        }
    }

    /// The coach's read of one player.
    fn derive_role(player: &Player, ranks: &SquadDepthRanks, today: NaiveDate) -> PlannedRole {
        let group = player.position().position_group();
        let is_goalkeeper = group == PlayerFieldPositionGroup::Goalkeeper;
        let age = player.age(today);
        let level = AbilityEstimator::observable_level(player);
        let ceiling = PotentialEstimator::observable_ceiling(player, today);
        let rank = ranks.rank_of(player, level);

        // A manager-pinned player is in the plans by definition.
        if player.is_force_match_selection {
            return PlannedRole::Starter;
        }

        // ── The heir ──
        // A young player the staff believe will reach the level of an
        // ageing man ahead of him is being groomed, not warehoused.
        if ranks.is_succession_heir(player, age, ceiling) {
            return PlannedRole::SuccessionHeir;
        }

        // ── Youth ──
        if age <= 21 && ceiling > level.saturating_add(8) {
            return PlannedRole::DevelopmentPathway;
        }

        // ── The stalled ──
        // Seasons here without first-team football, past the age where
        // that is a stage rather than a verdict. The honest answers are
        // the shop window or, for a player with nothing left to offer,
        // no future at all — and the point of saying so is that the
        // player finds out.
        let stalled = age > 23
            && StuckCareerScan::of(player, today).is_some_and(|scan| scan.stuck_years >= 2);
        if stalled {
            // A keeper is a special case: only one man can play, so a
            // settled senior deputy has a real role rather than a
            // stalled career — the cup is his.
            if is_goalkeeper && rank == 1 && age >= 24 {
                return PlannedRole::CupKeeper;
            }
            if level.saturating_add(20) < ranks.squad_level() {
                return PlannedRole::NotInPlans;
            }
            return PlannedRole::ShopWindow;
        }

        // ── The picture at his position ──
        match rank {
            0 if ranks.is_squad_leader(level) => PlannedRole::Cornerstone,
            0 => PlannedRole::Starter,
            // Only one keeper plays, so the second-choice keeper is the
            // cup keeper rather than a rotation option.
            1 if is_goalkeeper => PlannedRole::CupKeeper,
            1 | 2 => PlannedRole::Rotation,
            _ => PlannedRole::Cover,
        }
    }
}

/// The depth picture the coach reasons from — who is ahead of whom at
/// each position, and how the squad's levels are spread. Built once per
/// revision from OBSERVABLE levels only.
pub struct SquadDepthRanks {
    /// Descending observable levels per position group.
    group_levels: HashMap<PlayerFieldPositionGroup, Vec<u8>>,
    /// Oldest first-choice per group, with his level — the man an heir
    /// would be groomed to replace.
    incumbents: HashMap<PlayerFieldPositionGroup, (u32, u8, u8)>,
    squad_level: u8,
    top_level: u8,
}

impl SquadDepthRanks {
    /// Age from which an incumbent is old enough to be worth succeeding.
    const INCUMBENT_AGEING_FROM: u8 = 31;
    /// Oldest a groomed heir can be.
    const HEIR_MAX_AGE: u8 = 24;

    pub fn build(players: &PlayerCollection, today: NaiveDate) -> Self {
        let mut group_levels: HashMap<PlayerFieldPositionGroup, Vec<u8>> = HashMap::new();
        let mut sum: u32 = 0;
        let mut count: u32 = 0;
        let mut top_level: u8 = 0;

        for player in players.iter() {
            if player.is_on_loan() {
                continue;
            }
            let group = player.position().position_group();
            let level = AbilityEstimator::observable_level(player);
            group_levels.entry(group).or_default().push(level);
            sum += level as u32;
            count += 1;
            top_level = top_level.max(level);
        }
        for levels in group_levels.values_mut() {
            levels.sort_unstable_by(|a, b| b.cmp(a));
        }

        // The incumbent at each position: the best man there, kept with
        // his age so an heir can be recognised.
        let mut incumbents: HashMap<PlayerFieldPositionGroup, (u32, u8, u8)> = HashMap::new();
        for player in players.iter() {
            if player.is_on_loan() {
                continue;
            }
            let group = player.position().position_group();
            let level = AbilityEstimator::observable_level(player);
            let best = group_levels
                .get(&group)
                .and_then(|l| l.first())
                .copied()
                .unwrap_or(0);
            if level == best {
                incumbents
                    .entry(group)
                    .or_insert((player.id, level, player.age(today)));
            }
        }

        Self {
            group_levels,
            incumbents,
            squad_level: if count > 0 { (sum / count) as u8 } else { 0 },
            top_level,
        }
    }

    pub fn squad_level(&self) -> u8 {
        self.squad_level
    }

    /// How many squad-mates at his position the coach rates above him.
    pub fn rank_of(&self, player: &Player, level: u8) -> usize {
        self.group_levels
            .get(&player.position().position_group())
            .map(|levels| levels.iter().filter(|&&l| l > level).count())
            .unwrap_or(0)
    }

    /// Within reach of the best man in the whole squad.
    pub fn is_squad_leader(&self, level: u8) -> bool {
        const LEADER_GAP: u8 = 5;
        level.saturating_add(LEADER_GAP) >= self.top_level
    }

    /// The incumbent this player would be groomed to replace.
    pub fn incumbent_of(&self, player: &Player) -> Option<u32> {
        self.incumbents
            .get(&player.position().position_group())
            .map(|(id, _, _)| *id)
            .filter(|id| *id != player.id)
    }

    /// A young player the staff believe will reach the level of an
    /// ageing incumbent ahead of him.
    pub fn is_succession_heir(&self, player: &Player, age: u8, ceiling: u8) -> bool {
        if age > Self::HEIR_MAX_AGE {
            return false;
        }
        let Some(&(incumbent_id, incumbent_level, incumbent_age)) =
            self.incumbents.get(&player.position().position_group())
        else {
            return false;
        };
        if incumbent_id == player.id || incumbent_age < Self::INCUMBENT_AGEING_FROM {
            return false;
        }
        ceiling >= incumbent_level
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerClubContract, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills, PlayerSquadStatus, PlayerStatCompetitionKind,
        PlayerStatLedgerEntry, PlayerStatistics,
    };
    use chrono::Datelike;

    /// Fixtures for the squad-plan derivation.
    struct Fx;

    impl Fx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()
        }

        fn player(id: u32, pos: PlayerPositionType, ability: u8, age: u8) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ability;
            attrs.potential_ability = ability;
            let mut contract =
                PlayerClubContract::new(50_000, NaiveDate::from_ymd_opt(2030, 6, 30).unwrap());
            contract.squad_status = PlayerSquadStatus::MainBackupPlayer;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("T".into(), format!("P{id}")))
                .birth_date(
                    NaiveDate::from_ymd_opt(Self::date().year() - age as i32, 1, 1).unwrap(),
                )
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::flat_for_ability(ability))
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: pos,
                        level: 18,
                    }],
                })
                .player_attributes(attrs)
                .contract(Some(contract))
                .build()
                .unwrap()
        }

        /// Give a player N consecutive seasons of bench duty on the
        /// canonical ledger, so the stuck-career scan sees a stalled man.
        fn with_stalled_seasons(mut p: Player, seasons: u16) -> Player {
            for back in 0..seasons {
                p.statistics_history
                    .season_ledger
                    .push(PlayerStatLedgerEntry {
                        seq_id: 0,
                        season_start_year: 2025 - back,
                        team_slug: "t".into(),
                        team_name: "T".into(),
                        team_reputation: 8_000,
                        league_slug: "l".into(),
                        league_name: "L".into(),
                        competition_kind: PlayerStatCompetitionKind::League,
                        competition_slug: "l".into(),
                        is_loan: false,
                        transfer_fee: None,
                        coverage_days: None,
                        statistics: PlayerStatistics {
                            played: 1,
                            ..Default::default()
                        },
                    });
            }
            p
        }
    }

    /// Only one keeper can play the league, so a settled senior deputy
    /// has a real role rather than a stalled career — and naming it is
    /// what gets him the cup ties he was never given.
    #[test]
    fn the_second_keeper_is_the_cup_keeper() {
        let squad = PlayerCollection::new(vec![
            Fx::player(1, PlayerPositionType::Goalkeeper, 150, 30),
            Fx::with_stalled_seasons(Fx::player(2, PlayerPositionType::Goalkeeper, 130, 28), 4),
        ]);
        let mut plan = CoachSquadPlan::new();
        plan.revise(&squad, Fx::date());
        assert!(
            plan.role_of(1)
                .is_some_and(|r| matches!(r, PlannedRole::Cornerstone | PlannedRole::Starter)),
            "the first-choice keeper is central to the plan"
        );
        assert_eq!(
            plan.role_of(2),
            Some(PlannedRole::CupKeeper),
            "a senior deputy keeper has a competition of his own, not a stalled career"
        );
    }

    /// An outfield player with the same stalled record has no such role —
    /// eleven men play every week, so there is no honest place for him.
    #[test]
    fn a_stalled_outfield_player_goes_in_the_shop_window() {
        let squad = PlayerCollection::new(vec![
            Fx::player(1, PlayerPositionType::MidfielderCenter, 150, 27),
            Fx::player(2, PlayerPositionType::MidfielderCenter, 148, 26),
            Fx::with_stalled_seasons(
                Fx::player(3, PlayerPositionType::MidfielderCenter, 140, 27),
                4,
            ),
        ]);
        let mut plan = CoachSquadPlan::new();
        plan.revise(&squad, Fx::date());
        assert!(
            plan.role_of(3).is_some_and(|r| r.is_exit_path()),
            "a prime-age midfielder the coach has not picked for four seasons is on his way out"
        );
    }

    /// The plan is the coach's opinion, and it changes — a role change is
    /// what the club then has to tell the player about.
    #[test]
    fn revision_reports_role_changes_only() {
        let squad = PlayerCollection::new(vec![
            Fx::player(1, PlayerPositionType::Goalkeeper, 150, 30),
            Fx::player(2, PlayerPositionType::Goalkeeper, 130, 28),
        ]);
        let mut plan = CoachSquadPlan::new();
        let first = plan.revise(&squad, Fx::date());
        assert_eq!(first.len(), 2, "a fresh plan is all new opinions");

        let later = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let second = plan.revise(&squad, later);
        assert!(
            second.is_empty(),
            "nothing changed, so the coach has nothing new to tell anyone"
        );
    }

    #[test]
    fn a_new_manager_inherits_a_squad_not_a_plan() {
        let squad =
            PlayerCollection::new(vec![Fx::player(1, PlayerPositionType::Goalkeeper, 150, 30)]);
        let mut plan = CoachSquadPlan::new();
        plan.revise(&squad, Fx::date());
        assert!(!plan.is_empty());
        plan.clear();
        assert!(plan.is_empty());
        assert!(plan.due_for_revision(Fx::date()));
    }

    #[test]
    fn players_who_leave_drop_out_of_the_plan() {
        let squad = PlayerCollection::new(vec![
            Fx::player(1, PlayerPositionType::Goalkeeper, 150, 30),
            Fx::player(2, PlayerPositionType::Goalkeeper, 130, 28),
        ]);
        let mut plan = CoachSquadPlan::new();
        plan.revise(&squad, Fx::date());
        assert!(plan.role_of(2).is_some());

        let smaller =
            PlayerCollection::new(vec![Fx::player(1, PlayerPositionType::Goalkeeper, 150, 30)]);
        plan.revise(&smaller, NaiveDate::from_ymd_opt(2026, 8, 1).unwrap());
        assert!(
            plan.role_of(2).is_none(),
            "a departed player is no longer the coach's problem"
        );
    }
}
