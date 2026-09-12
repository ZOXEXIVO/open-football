//! Where the manager stands, as far as the club can see.

use chrono::{Duration, NaiveDate};

use crate::Club;
use crate::club::context::ClubContext;
use crate::club::mind::organs::memory::ActorRef;
use crate::club::person::Person;
use crate::club::staff::mind::StaffSituation;
use crate::club::staff::perception::AbilityEstimator;
use crate::utils::DateUtils;

/// The four table numbers a manager's situation reads.
///
/// Lifted out of [`ClubContext`] as plain `Copy` scalars: the context
/// borrows the club's own name, and [`Club::run_manager_mind`] needs a
/// mutable borrow of the club at the same time.
#[derive(Debug, Clone, Copy, Default)]
pub struct LeagueStanding {
    pub position: u8,
    pub size: u8,
    pub played: u8,
    pub total: u8,
}

impl LeagueStanding {
    pub fn from_context(ctx: &ClubContext<'_>) -> Self {
        LeagueStanding {
            position: ctx.league_position,
            size: ctx.league_size,
            played: ctx.league_matches_played,
            total: ctx.total_league_matches,
        }
    }
}

impl Club {
    /// Days a newly-appointed head coach gets to look at the whole squad
    /// before the club's routine listing sweeps resume.
    const MANAGER_REVIEW_DAYS: i64 = 45;

    /// Where the manager actually is, as far as the club can see.
    ///
    /// The club is the only place that holds the board, the squad and
    /// the table at once, which is why the situation is assembled here
    /// rather than inside `Staff::simulate`. Read-only, and built before
    /// the mutable borrow of the manager himself; the axes that are
    /// facts about *him* — age, load, contract, tenure — are filled in
    /// by [`Self::run_manager_mind`], which has him in hand.
    fn manager_situation(&self, table: LeagueStanding) -> StaffSituation {
        let mut situation = StaffSituation::neutral();

        // ── The people above him ────────────────────────────────
        situation.board_trust = self.board.relationship.overall_trust() as f32 / 100.0;
        situation.board_pressure = (self.board.pressure.supporter_pressure as f32 * 0.4
            + self.board.pressure.media_pressure as f32 * 0.3
            + self.board.pressure.dressing_room_pressure as f32 * 0.3)
            / 100.0;
        // Whether they have actually been backing him, as distinct from
        // whether they say they trust him. Communication is the promises
        // they kept; squad-building is whether he was allowed to build.
        situation.board_backing = (self.board.relationship.trust_communication as f32 * 0.6
            + self.board.relationship.trust_squad_building as f32 * 0.4)
            / 100.0;

        // ── Results ─────────────────────────────────────────────
        if let Some(targets) = &self.board.season_targets {
            situation.expected_position = targets.expected_position;
        }
        // The stands, read from the pressure the supporters are putting
        // on the board. There is no separate crowd-mood field on the
        // club, and this is the same signal from the other side.
        situation.terraces =
            (1.0 - self.board.pressure.supporter_pressure as f32 / 100.0).clamp(0.0, 1.0);

        if let Some(main) = self.teams.main() {
            situation.club_standing = (main.reputation.world as f32 / 10_000.0).clamp(0.0, 1.0);
        }
        situation.league_position = table.position;
        situation.league_size = table.size;
        if table.total > 0 {
            situation.season_progress = (table.played as f32 / table.total as f32).clamp(0.0, 1.0);
        }
        // The dressing room, as the coach's own decision state already
        // measures it. It lives on the man since S2, so this is now the
        // manager reading his own state rather than the club's.
        if let Some(state) = self.teams.head_coach_decision_state() {
            situation.dressing_room = state.squad_satisfaction.clamp(0.0, 1.0);
        }

        situation
    }

    /// Let the manager think, once a week.
    ///
    /// The situated pass: the five faculties reflect on where he
    /// actually is, form or advance what he wants, and revise their
    /// reading of the board, the room and the stands. `Staff::simulate`
    /// runs the quiet pass daily for every member of staff; this is the
    /// one that needs the whole club in hand.
    ///
    /// Nothing downstream reads the result yet — the mind accumulates in
    /// parallel with `job_satisfaction` and `CoachMemoryStore`, exactly
    /// as `PlayerMind` accumulates alongside `PlayerHappiness`.
    /// Age at which what a player is now is what he is going to be, so a
    /// view of his ceiling can be marked right or wrong.
    const VERDICT_AGE: u8 = 25;
    /// Or, for a view formed long enough ago, the waiting is the answer.
    const VERDICT_DAYS: u16 = 365 * 3;

    pub fn run_manager_mind(&mut self, today: NaiveDate, table: LeagueStanding) {
        let mut situation = self.manager_situation(table);
        let club_id = self.id;

        let Some(main) = self.teams.main_mut() else {
            return;
        };
        let squad_size = main.players.players.len();
        let Some(manager) = main.staffs.head_coach_mut() else {
            return;
        };
        if manager.id == 0 {
            return;
        }

        let context = manager.mind_context(today, club_id);
        let day = context.day();

        situation.age = DateUtils::age(manager.birth_date, today) as f32;
        situation.strain = (manager.fatigue / 100.0).clamp(0.0, 1.0);
        situation.standing = manager.manager_standing();
        situation.contract_months_left = manager
            .contract
            .as_ref()
            .map(|c| ((c.expired - today).num_days() / 30).clamp(-120, 600) as i16)
            .unwrap_or(0);

        // Tenure comes from the mind, because nothing else knows it —
        // `StaffClubContract` carries an expiry and no start date. The
        // ambition faculty writes the day down when it hears about the
        // appointment.
        let months = manager.mind.ambition.months_in_the_job(day);
        situation.months_in_the_job = months;
        situation.trophies_here = manager.mind.ambition.honours_here;

        // How much of the side is his: the players he has actually
        // signed, over the size of the squad he picks from. Counted at
        // the arrival chokepoint (`TransferExecution::sign_into_main_team`)
        // rather than inferred from tenure, so a manager backed in two
        // windows reads as further along than one given four quiet
        // seasons — which is the difference the counterweight is for.
        situation.squad_is_his = if squad_size == 0 {
            0.0
        } else {
            (manager.mind.ambition.signings as f32 / squad_size as f32).clamp(0.0, 1.0)
        };

        manager.mind.tick_with(&context, &situation);
    }

    /// The monthly audit: which of the manager's views about his players
    /// the careers in front of him have now answered.
    ///
    /// This is the loop that closes. A coach forms a judgement about every
    /// player he watches, and until now nothing ever scored one — so
    /// `JudgementOutcome` never left `Open`, `IWasWrongAboutHim` could
    /// never form, and a manager's patience and self-belief sat at whatever
    /// they were seeded with for a thirty-year career.
    ///
    /// Only settles questions a career has actually answered: a man old
    /// enough that what he is now is what he is going to be, or a view held
    /// long enough that the waiting is itself the answer. And only ones the
    /// coach was committed enough to be right or wrong about — an opinion
    /// he was never sure of teaches him nothing, which
    /// [`PlayerJudgement::settle`] enforces on its own.
    pub fn audit_manager_judgements(&mut self, today: NaiveDate) {
        let club_id = self.id;
        let Some(main) = self.teams.main_mut() else {
            return;
        };
        let squad: Vec<(u32, u8, f32)> = main
            .players
            .players
            .iter()
            .map(|player| {
                (
                    player.id,
                    player.age(today),
                    AbilityEstimator::observable_level(player) as f32 / 200.0,
                )
            })
            .collect();

        let Some(manager) = main.staffs.head_coach_mut() else {
            return;
        };
        if manager.id == 0 {
            return;
        }
        let context = manager.mind_context(today, club_id);
        let day = context.day();

        for (player_id, age, true_level) in squad {
            let player = ActorRef::player(player_id);
            let Some(view) = manager.mind.judgement_of(player) else {
                continue;
            };
            if view.outcome.is_settled() {
                continue;
            }
            let old_enough = age >= Self::VERDICT_AGE;
            let long_enough = day.saturating_sub(view.formed) >= Self::VERDICT_DAYS;
            if !old_enough && !long_enough {
                continue;
            }
            manager
                .mind
                .settle_judgement(player, true_level, &context);
        }
    }

    /// Pause club-driven listings so a just-appointed head coach can form
    /// his own view before the previous regime's exit decisions are acted
    /// on. Player-initiated departures (a formal request, hardened
    /// unhappiness) are unaffected — a new manager cannot make a player
    /// un-ask to leave.
    ///
    /// Deliberately does NOT extend a window that is already open. Re-arming
    /// unconditionally meant a club cycling through managers faster than the
    /// review lasts never listed anyone at all: each sacking pushed the
    /// deadline out again and the freeze became permanent, which is the
    /// opposite of how a crisis club behaves — there it is the board, not
    /// the coach, driving players out.
    pub(in crate::club::core) fn open_manager_review_window(&mut self, date: NaiveDate) {
        let review_in_progress = self
            .transfer_plan
            .manager_review_until
            .map(|until| date < until)
            .unwrap_or(false);
        if review_in_progress {
            return;
        }
        self.transfer_plan.manager_review_until =
            Some(date + Duration::days(Self::MANAGER_REVIEW_DAYS));
    }
}
