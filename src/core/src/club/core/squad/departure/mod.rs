//! The one place a club says "this player is leaving us".
//!
//! Players left clubs by seven different routes and none of them told
//! anybody: a transfer took him out of the squad vector, a released contract
//! swept him up at the end of the season, a loan moved him sideways, a
//! retirement pushed him onto the country's retired list. The teammates had
//! a reaction pass. The manager had nothing — his memory of the man, his
//! plan for him and his read of him simply stayed behind, pointing at
//! somebody who was no longer there.
//!
//! So this is the choke point. Every removal route calls it *before* taking
//! the player out, while the squad, the staff and the player are all still
//! in one place, and it does two things:
//!
//! * closes the working relationship with whoever was picking him, writing
//!   what is left of it into a dossier;
//! * gathers the parting report from the player himself, because the coach's
//!   record cannot see how many promises were made to him or how the man
//!   felt about working for him.
//!
//! It deliberately does **not** decide *why* he is going. The cause is a
//! fact the caller has and this module does not — a sale the board forced
//! and a sale the manager asked for leave a coach feeling very differently,
//! and guessing between them would be worse than not recording it.

use crate::club::Club;
use crate::club::mind::organs::memory::ActorRef;
use crate::club::person::Person;
use crate::club::player::interaction::InteractionOutcome;
use crate::club::player::player::Player;
use crate::club::staff::coach::dossier::closer::PartingReport;
use crate::club::staff::coach::{Dossiers, SeparationCause};
use crate::club::staff::perception::AbilityEstimator;
use crate::club::team::Team;
use crate::{HappinessEventType, PlayerStatusType};
use chrono::NaiveDate;

/// Tells the dugout that a player is going.
pub struct SquadDepartures;

impl SquadDepartures {
    /// A player is leaving `club`. Close his spell with whoever has been
    /// picking him, and record the parting.
    ///
    /// Safe to call for a player the coach never worked with, for a club
    /// with nobody in the dugout, and twice for the same departure.
    pub fn notify(club: &mut Club, player_id: u32, cause: SeparationCause, today: NaiveDate) {
        let club_id = club.id;
        let Some(report) = Self::report(club, player_id, club_id, today) else {
            return;
        };
        let Some(main) = club.teams.main_mut() else {
            return;
        };
        let Some(coach) = main.staffs.head_coach_mut() else {
            return;
        };
        if coach.id == 0 {
            return;
        }
        if let Some(kind) = coach.player_left(&report, cause, today) {
            coach.remember(kind, ActorRef::player(player_id), today, club_id);
        }
    }

    /// As [`Self::notify`], for the routes that hold a team rather than the
    /// whole club — the end-of-season sweeps walk teams directly.
    pub fn notify_team(
        team: &mut Team,
        player_id: u32,
        cause: SeparationCause,
        today: NaiveDate,
    ) {
        let club_id = team.club_id;
        let Some(report) = Self::report_from_team(team, player_id, club_id, today) else {
            return;
        };
        let Some(coach) = team.staffs.head_coach_mut() else {
            return;
        };
        if coach.id == 0 {
            return;
        }
        if let Some(kind) = coach.player_left(&report, cause, today) {
            coach.remember(kind, ActorRef::player(player_id), today, club_id);
        }
    }

    /// Every squad at the club, so a reserve or an academy player leaving
    /// is a parting too — the manager's record covers whoever he has
    /// actually been watching, not whoever is in the first team today.
    fn report(
        club: &Club,
        player_id: u32,
        club_id: u32,
        today: NaiveDate,
    ) -> Option<PartingReport> {
        let main = club.teams.main()?;
        let coach_id = main.staffs.head_coach().id;
        if coach_id == 0 {
            return None;
        }
        let was_captain = main.captain_id == Some(player_id);
        let player = club
            .teams
            .teams
            .iter()
            .find_map(|team| team.players.players.iter().find(|p| p.id == player_id))?;
        Some(Self::compile(player, coach_id, club_id, was_captain, today))
    }

    fn report_from_team(
        team: &Team,
        player_id: u32,
        club_id: u32,
        today: NaiveDate,
    ) -> Option<PartingReport> {
        let coach_id = team.staffs.head_coach().id;
        if coach_id == 0 {
            return None;
        }
        let was_captain = team.captain_id == Some(player_id);
        let player = team.players.players.iter().find(|p| p.id == player_id)?;
        Some(Self::compile(player, coach_id, club_id, was_captain, today))
    }

    /// What the coach cannot see from his own record: what the player
    /// thought of him, what he was promised, and what his body did.
    fn compile(
        player: &Player,
        coach_id: u32,
        club_id: u32,
        was_captain: bool,
        today: NaiveDate,
    ) -> PartingReport {
        let regard = player
            .relations
            .get_staff(coach_id)
            .map(|relation| {
                // The same blend `CoachPlayerBond` reads, on 0..1.
                ((relation.level + 100.0) / 200.0 * 0.4
                    + relation.authority_respect / 100.0 * 0.3
                    + relation.trust_in_abilities / 100.0 * 0.3)
                    .clamp(0.0, 1.0)
            })
            .unwrap_or(0.5);

        let (kept, broken) = Self::promise_record(player, coach_id);

        PartingReport {
            player_id: player.id,
            club_id,
            age: player.age(today),
            his_regard: regard,
            promises_kept: kept,
            promises_broken: broken,
            level_now: AbilityEstimator::observable_level(player) as f32 / 200.0,
            was_my_captain: was_captain,
            // What he was when the spell opened is the coach's own view of
            // him, and the judgement organ is where that lives — the closer
            // reads it there rather than being told a number here.
            level_at_start: AbilityEstimator::observable_level(player) as f32 / 200.0,
            // Fitness history is not on the player in a form this pass can
            // read; left at nothing rather than guessed, so the `FRAGILE`
            // scar only ever comes from somewhere that actually knows.
            injury_days: 0,
            red_cards: player.statistics.red_cards,
            discipline_events: u8::from(player.statuses.has(PlayerStatusType::Sus)),
            errors: 0,
        }
    }

    /// Promises this coach made him, and how they went.
    ///
    /// Two logs and neither is complete on its own: the interaction log
    /// knows *who* made a promise but not whether it was honoured, and the
    /// happiness feed knows the outcome but not the man. So the coach is
    /// only credited or blamed for outcomes that fall in a window where he
    /// was the one doing the promising.
    fn promise_record(player: &Player, coach_id: u32) -> (u8, u8) {
        let promised_by_him = player
            .interactions
            .entries
            .iter()
            .any(|entry| entry.staff_id == coach_id && entry.promise_created)
            || player
                .promises
                .iter()
                .any(|promise| promise.made_by_staff_id == Some(coach_id));
        if !promised_by_him {
            return (0, 0);
        }

        let mut kept = 0u8;
        let mut broken = 0u8;
        for event in player.happiness.recent_events.iter() {
            match event.event_type {
                HappinessEventType::PromiseKept => kept = kept.saturating_add(1),
                HappinessEventType::PromiseBroken => broken = broken.saturating_add(1),
                _ => {}
            }
        }
        // A talk that ended badly with an assurance on the table is the
        // fudged promise the verifier has not caught up with yet.
        for entry in player.interactions.entries.iter() {
            if entry.staff_id == coach_id
                && entry.promise_created
                && entry.outcome == InteractionOutcome::Negative
            {
                broken = broken.saturating_add(1);
            }
        }
        (kept, broken)
    }

    /// Why a sale reads the way it does to the man in the dugout.
    ///
    /// The same transfer is three different events depending on whose idea
    /// it was, and none of the three is guessable after the fact — so it is
    /// decided here, once, from the state that is still standing: did the
    /// player ask to go, had the coach already written him off, or is this
    /// the board taking the money for a man he was picking.
    pub fn sale_cause(club: &Club, player_id: u32) -> SeparationCause {
        let Some(main) = club.teams.main() else {
            return SeparationCause::SoldByBoard;
        };
        let player = club
            .teams
            .teams
            .iter()
            .find_map(|team| team.players.players.iter().find(|p| p.id == player_id));

        if let Some(player) = player {
            if player.statuses.has(PlayerStatusType::Req)
                || player.mind.wants_to_leave() >= Self::MOVE_WAS_HIS_IDEA
            {
                return SeparationCause::HeRequestedOut;
            }
        }

        let coach = main.staffs.head_coach();
        if coach.id != 0
            && coach
                .squad_plan
                .role_of(player_id)
                .is_some_and(|role| role.is_exit_path())
        {
            return SeparationCause::SoldOnMyCall;
        }
        SeparationCause::SoldByBoard
    }

    /// Why a release reads the way it does. The same shape as a sale, minus
    /// the fee — and with running a contract down as its own answer,
    /// because a player who declines terms and walks for nothing has made a
    /// decision the coach takes personally.
    pub fn release_cause(club: &Club, player_id: u32) -> SeparationCause {
        let Some(main) = club.teams.main() else {
            return SeparationCause::ReleasedByBoard;
        };
        let coach = main.staffs.head_coach();
        if coach.id != 0
            && coach
                .squad_plan
                .role_of(player_id)
                .is_some_and(|role| role.is_exit_path())
        {
            return SeparationCause::ReleasedOnMyCall;
        }
        SeparationCause::ReleasedByBoard
    }

    /// Net pull out of the club at which a move counts as the player's own
    /// idea. Mirrors the threshold the player's own transfer memory uses,
    /// so the two sides of a move cannot disagree about whose it was.
    const MOVE_WAS_HIS_IDEA: f32 = 0.45;

    /// True when the coach has an open spell with this player — the guard
    /// every caller can use to skip the work entirely.
    pub fn is_working_with(club: &Club, player_id: u32) -> bool {
        club.teams
            .main()
            .map(|team| team.staffs.head_coach())
            .is_some_and(|coach| Dossiers::is_working_with(&coach.dossiers, player_id))
    }
}
