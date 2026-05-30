use crate::club::team::behaviour::TeamBehaviour;
use crate::club::team::{
    Achievement, CaptaincyAssigner, ChemistryContextBuilder, CompetitionType, MatchOutcome,
    MatchResultInfo, MentorshipProcessor, PreventiveRestPass, SquadSocialViewBuilder,
    SquadStatusUpdater, TeamBuilder, TeamFixtureWindow, TeamType,
};
use crate::context::GlobalContext;
use crate::shared::CurrencyValue;
use crate::{
    MatchHistory, MatchTacticType, Player, PlayerCollection, StaffCollection, Tactics,
    TacticsSelector, TeamInfo, TeamReputation, TeamResult, TeamTraining, TrainingSchedule,
    TransferItem, Transfers,
};
use chrono::NaiveDate;
use std::borrow::Cow;

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

    /// Cached upcoming-fixture window written by the league/country
    /// pipeline before `simulate` runs. Lets training read real
    /// calendar distance to the next match instead of guessing a
    /// Saturday fixture week. Refreshed once per simulation tick.
    pub fixture_window: TeamFixtureWindow,

    /// Appointed captain — wears the armband. Selected monthly by
    /// `CaptaincyAssigner::assign` based on leadership, loyalty, and
    /// tenure. Distinct from the emergent "influence leader" used
    /// elsewhere.
    pub captain_id: Option<u32>,
    /// Stand-in captain when the captain is unavailable (injured / benched).
    pub vice_captain_id: Option<u32>,
}

impl Team {
    pub fn builder() -> TeamBuilder {
        TeamBuilder::new()
    }

    /// Lightweight `TeamInfo` for stats-history rows where the caller
    /// has no league-lookup access. The web layer fills league info
    /// back in by inspecting the team's current league at render time,
    /// so leaving `league_name` / `league_slug` empty is correct.
    pub fn history_info(&self) -> TeamInfo {
        TeamInfo {
            name: self.name.clone(),
            slug: self.slug.clone(),
            reputation: self.reputation.world,
            league_name: String::new(),
            league_slug: String::new(),
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> TeamResult {
        if ctx.simulation.is_month_beginning() {
            self.run_monthly_pass(ctx.simulation.date.date());
        }

        if ctx.simulation.is_week_beginning() {
            self.run_weekly_pass(ctx.simulation.date.date());
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

        TeamResult::new(
            self.id,
            self.players.simulate(player_ctx.with_player(None)),
            self.staffs.simulate(ctx.with_staff(None)),
            self.behaviour
                .simulate(&mut self.players, &mut self.staffs, ctx.with_team(self.id)),
            TeamTraining::train(self, ctx.simulation.date, ctx.club_facilities_training()),
        )
    }

    /// Monthly tick — squad statuses and captaincy reappointment. Runs
    /// on the 1st of each month so the armband never drifts off a
    /// retiring veteran or onto a newcomer who hasn't earned it yet.
    fn run_monthly_pass(&mut self, date: NaiveDate) {
        SquadStatusUpdater::apply(self, date);
        CaptaincyAssigner::assign(self, date);
    }

    /// Weekly tick — mentorship, social decay, chemistry refresh,
    /// preventive rest, and the squad social-view snapshot. Runs before
    /// any per-player development so today's mentoring drift is already
    /// visible when weekly skill growth is computed.
    fn run_weekly_pass(&mut self, week_date: NaiveDate) {
        let hoy_wwy = self.staffs.best_youth_development_wwy(10);
        let _pairings = MentorshipProcessor::process(&mut self.players.players, week_date, hoy_wwy);

        // Without this, every relationship and rapport entry that ever
        // fired stays at its peak forever — squads wouldn't naturally
        // drift toward neutral when contact fades. Relations decay toward
        // neutral if interaction was light; rapport drifts to 0 after 21+
        // days of no training contact. Runs before any new weekly
        // relationship event so today's events overwrite the decayed
        // baseline.
        for player in self.players.players.iter_mut() {
            player.relations.process_weekly_update(week_date);
            player.rapport.decay(week_date);
        }

        let chem_ctx = ChemistryContextBuilder::build(self, week_date);
        for player in self.players.players.iter_mut() {
            player
                .relations
                .recalculate_chemistry_with_context(&chem_ctx);
        }

        let best_sports_sci = self.staffs.best_sports_science();
        PreventiveRestPass::apply(&mut self.players.players, best_sports_sci, week_date);

        SquadSocialViewBuilder::refresh(&mut self.players.players);
    }

    pub fn players(&self) -> Vec<&Player> {
        self.players.players()
    }

    /// Reappoint the captain & vice-captain. See [`CaptaincyAssigner`]
    /// for ranking logic and event-emission guards. Kept as a thin
    /// passthrough so existing call sites (and tests) read naturally.
    pub fn assign_captaincy(&mut self, date: NaiveDate) {
        CaptaincyAssigner::assign(self, date);
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

#[cfg(test)]
mod payroll_tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerClubContract, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills, PlayerSquadStatus,
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
        let mut loan = PlayerClubContract::new_loan(120_000, d(2027, 6, 30), 1, 1, 2);
        loan.salary = 120_000;
        p.contract_loan = Some(loan);
        let team = build_team_with(vec![p]);
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
        assert_eq!(snap.top_earner, 100_000);
        assert_eq!(snap.current_bill, 200_000);
    }
}

#[cfg(test)]
mod captaincy_tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::team::{LeadershipCandidate, MatchdayLeadership};
    use crate::shared::fullname::FullName;
    use crate::{
        HappinessEventType, PersonAttributes, PlayerAttributes, PlayerClubContract, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills,
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
    fn bootstrap_assignment_creates_award_event() {
        // A fresh team starts with no captain; the first monthly appointment
        // is narrated like any other, so the new captain gets a visible,
        // positive award on his events page (no silent bootstrap).
        let p1 = build_leader(1, 18.0, 5_000);
        let p2 = build_leader(2, 14.0, 3_000);
        let mut team = build_team_with(vec![p1, p2]);

        assert_eq!(team.captain_id, None);
        team.assign_captaincy(d(2026, 7, 1));

        let captain = team.captain_id.expect("a captain should be appointed");
        let cap = team
            .players
            .players
            .iter()
            .find(|p| p.id == captain)
            .unwrap();
        assert_eq!(
            captaincy_event_count(cap, &HappinessEventType::CaptaincyAwarded),
            1
        );
        let award = cap
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::CaptaincyAwarded)
            .unwrap();
        assert!(award.magnitude > 0.0, "appointment must be a positive event");
        // Nobody is stripped on a first appointment.
        for player in team.players.players.iter() {
            assert_eq!(
                captaincy_event_count(player, &HappinessEventType::CaptaincyRemoved),
                0
            );
        }
    }

    #[test]
    fn captain_replacement_creates_removed_and_awarded_events() {
        let p1 = build_leader(1, 14.0, 3_000);
        let p2 = build_leader(2, 10.0, 2_000);
        let mut team = build_team_with(vec![p1, p2]);

        team.assign_captaincy(d(2026, 7, 1));
        let original_captain = team.captain_id.unwrap();

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

        team.assign_captaincy(d(2026, 8, 1));

        for player in team.players.players.iter() {
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

        team.assign_captaincy(d(2026, 7, 1)); // first appointment (awards p1)
        let first_captain = team.captain_id.unwrap();

        for p in team.players.players.iter_mut() {
            if p.id == first_captain {
                p.skills.mental.leadership = 9.0;
            } else {
                p.skills.mental.leadership = 20.0;
                p.player_attributes.current_reputation = 9_000;
            }
        }
        team.assign_captaincy(d(2026, 8, 1));

        for p in team.players.players.iter_mut() {
            if p.id == first_captain {
                p.skills.mental.leadership = 20.0;
                p.player_attributes.current_reputation = 9_000;
            } else {
                p.skills.mental.leadership = 9.0;
            }
        }
        team.assign_captaincy(d(2026, 9, 1));

        for player in team.players.players.iter() {
            let awarded = captaincy_event_count(player, &HappinessEventType::CaptaincyAwarded);
            assert!(
                awarded <= 1,
                "expected ≤1 award, got {} for player {}",
                awarded,
                player.id
            );
        }
    }

    #[test]
    fn high_reputation_removed_captain_takes_bigger_hit() {
        // Two parallel teams: one star captain (reputation 9000), one
        // anonymous captain (reputation 500). Both get displaced; star's
        // hit should be more negative due to reputation amplification.
        fn run(rep: i16) -> f32 {
            let mut p1 = build_leader(1, 14.0, rep);
            p1.attributes.controversy = 10.0;
            p1.attributes.temperament = 10.0;
            p1.attributes.professionalism = 10.0;
            let p2 = build_leader(2, 10.0, 1_000);
            let mut team = build_team_with(vec![p1, p2]);
            team.assign_captaincy(d(2026, 7, 1));
            let captain = team.captain_id.unwrap();
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
        assert!(
            star < anon,
            "star {} should be more negative than anon {}",
            star,
            anon
        );
    }

    #[test]
    fn first_active_captain_appointment_creates_event() {
        // No eligible leader at first (everyone below the leadership floor),
        // so the team runs with no captain and no event. When a player later
        // grows into leadership, his appointment fires a `CaptaincyAwarded`.
        let p1 = build_leader(1, 5.0, 4_000); // below MIN_LEADERSHIP_FOR_CAPTAINCY
        let p2 = build_leader(2, 6.0, 2_000);
        let mut team = build_team_with(vec![p1, p2]);

        team.assign_captaincy(d(2026, 7, 1));
        assert_eq!(team.captain_id, None, "no eligible leader yet");
        for player in team.players.players.iter() {
            assert_eq!(
                captaincy_event_count(player, &HappinessEventType::CaptaincyAwarded),
                0
            );
        }

        // p1 grows into a leader and is appointed during active play.
        team.players
            .players
            .iter_mut()
            .find(|p| p.id == 1)
            .unwrap()
            .skills
            .mental
            .leadership = 16.0;
        team.assign_captaincy(d(2026, 8, 1));

        assert_eq!(team.captain_id, Some(1));
        let appointee = team.players.players.iter().find(|p| p.id == 1).unwrap();
        assert_eq!(
            captaincy_event_count(appointee, &HappinessEventType::CaptaincyAwarded),
            1
        );
        for player in team.players.players.iter() {
            assert_eq!(
                captaincy_event_count(player, &HappinessEventType::CaptaincyRemoved),
                0
            );
        }
    }

    #[test]
    fn unchanged_captain_does_not_duplicate_event() {
        // The first appointment fires one award; subsequent monthly reviews
        // that keep the same captain must NOT add further awards.
        let p1 = build_leader(1, 16.0, 4_000);
        let p2 = build_leader(2, 11.0, 2_000);
        let mut team = build_team_with(vec![p1, p2]);

        team.assign_captaincy(d(2026, 7, 1)); // first appointment → 1 award
        let captain = team.captain_id.unwrap();
        team.assign_captaincy(d(2026, 8, 1));
        team.assign_captaincy(d(2026, 9, 1));

        assert_eq!(team.captain_id, Some(captain));
        let cap = team
            .players
            .players
            .iter()
            .find(|p| p.id == captain)
            .unwrap();
        assert_eq!(
            captaincy_event_count(cap, &HappinessEventType::CaptaincyAwarded),
            1,
            "kept captain keeps the single original award, no duplicates"
        );
        for player in team.players.players.iter() {
            assert_eq!(
                captaincy_event_count(player, &HappinessEventType::CaptaincyRemoved),
                0
            );
        }
    }

    #[test]
    fn captain_removed_without_replacement_creates_removed_event() {
        // The only qualifying leader's leadership later drops below the
        // captaincy threshold while he stays in the squad, so the review
        // finds no eligible captain. He must be stripped even though no
        // replacement exists.
        let p1 = build_leader(1, 14.0, 3_000);
        let mut team = build_team_with(vec![p1]);

        team.assign_captaincy(d(2026, 7, 1)); // first appointment (awards p1)
        assert_eq!(team.captain_id, Some(1));

        // Drop below MIN_LEADERSHIP_FOR_CAPTAINCY (8.0) while staying in squad.
        team.players.players[0].skills.mental.leadership = 5.0;

        team.assign_captaincy(d(2026, 8, 1));

        assert_eq!(team.captain_id, None, "no eligible captain remains");
        let stripped = team.players.players.iter().find(|p| p.id == 1).unwrap();
        assert_eq!(
            captaincy_event_count(stripped, &HappinessEventType::CaptaincyRemoved),
            1
        );
        // He still carries the award from his original appointment.
        assert_eq!(
            captaincy_event_count(stripped, &HappinessEventType::CaptaincyAwarded),
            1
        );
    }

    #[test]
    fn set_official_captain_emits_on_change_only() {
        // Direct exercise of the chokepoint. Installing a captain on a team
        // with none fires a single award; a redundant write with the same
        // captain emits nothing; a genuine change strips the old and awards
        // the new.
        let p1 = build_leader(1, 14.0, 3_000);
        let p2 = build_leader(2, 14.0, 3_000);
        let mut team = build_team_with(vec![p1, p2]);

        // None -> A: award A only.
        CaptaincyAssigner::set_official_captain(&mut team, Some(1), Some(2));
        assert_eq!(team.captain_id, Some(1));
        {
            let a = team.players.players.iter().find(|p| p.id == 1).unwrap();
            assert_eq!(
                captaincy_event_count(a, &HappinessEventType::CaptaincyAwarded),
                1
            );
            assert_eq!(
                captaincy_event_count(a, &HappinessEventType::CaptaincyRemoved),
                0
            );
        }

        // A -> A (redundant): no new events.
        CaptaincyAssigner::set_official_captain(&mut team, Some(1), Some(2));
        {
            let a = team.players.players.iter().find(|p| p.id == 1).unwrap();
            assert_eq!(
                captaincy_event_count(a, &HappinessEventType::CaptaincyAwarded),
                1,
                "redundant write must not re-award"
            );
        }

        // A -> B: strip A, award B.
        CaptaincyAssigner::set_official_captain(&mut team, Some(2), Some(1));
        assert_eq!(team.captain_id, Some(2));
        let a = team.players.players.iter().find(|p| p.id == 1).unwrap();
        let b = team.players.players.iter().find(|p| p.id == 2).unwrap();
        assert_eq!(
            captaincy_event_count(a, &HappinessEventType::CaptaincyRemoved),
            1
        );
        assert_eq!(
            captaincy_event_count(b, &HappinessEventType::CaptaincyAwarded),
            1
        );
    }

    #[test]
    fn matchday_captain_does_not_change_official_captaincy() {
        // The official captain is rotated out for a match; the matchday
        // armband resolver picks a stand-in from the XI. This must NOT
        // touch the official captaincy state or emit any captaincy events.
        let p1 = build_leader(1, 18.0, 5_000); // official captain
        let p2 = build_leader(2, 14.0, 3_000);
        let p3 = build_leader(3, 12.0, 2_000);
        let mut team = build_team_with(vec![p1, p2, p3]);

        team.assign_captaincy(d(2026, 7, 1));
        let official = team.captain_id.unwrap();
        assert_eq!(official, 1);

        // Snapshot official-captaincy event totals before matchday resolution
        // (the bootstrap appointment already awarded the official captain).
        let count_all = |team: &Team, kind: &HappinessEventType| -> usize {
            team.players
                .players
                .iter()
                .map(|p| captaincy_event_count(p, kind))
                .sum()
        };
        let awards_before = count_all(&team, &HappinessEventType::CaptaincyAwarded);
        let removed_before = count_all(&team, &HappinessEventType::CaptaincyRemoved);

        // On-field pool EXCLUDING the official captain (he was benched).
        let xi: Vec<LeadershipCandidate> = team
            .players
            .players
            .iter()
            .filter(|p| p.id != official)
            .map(LeadershipCandidate::from_player)
            .collect();
        let armband = MatchdayLeadership::resolve(team.captain_id, team.vice_captain_id, &xi);

        // A stand-in wears the armband...
        assert!(armband.captain_id.is_some());
        assert_ne!(armband.captain_id, Some(official));
        // ...but the official captaincy is untouched and no NEW events fired.
        assert_eq!(team.captain_id, Some(official));
        assert_eq!(
            count_all(&team, &HappinessEventType::CaptaincyAwarded),
            awards_before,
            "matchday resolution must not add official captaincy awards"
        );
        assert_eq!(
            count_all(&team, &HappinessEventType::CaptaincyRemoved),
            removed_before
        );
    }
}
