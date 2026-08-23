//! The authority mind — his standing with the board, the room and the
//! stands.
//!
//! The faculty that closes `docs/staff_mind.md` §2.5. `ManagerRelationship`
//! models the board's five trust facets beautifully and lives on the
//! **board**; nothing anywhere records what the manager thinks of
//! *them*. `PromiseLedger` records what was promised; nothing records
//! whether he believes it.
//!
//! This is also the structural difference from the player mind: a player
//! reads one manager. A manager reads three constituencies, and they
//! move independently — a dressing room can be with him while the
//! terraces are not, and that is a different job from having lost both.

use super::organs::StaffOrgans;
use super::submind::{MindOption, ReasonSet, StaffSubMind, StaffView};
use crate::club::mind::organs::goals::{GoalDomain, GoalEvidence, GoalKind, GoalOrigin};
use crate::club::mind::organs::memory::{ActorKind, ActorRef, EpisodeKind, FactClaim, MindEpisode};
use crate::club::mind::verdict::MoodContribution;

/// His read of the people around him.
#[derive(Debug, Clone, Copy, Default)]
pub struct AuthorityMind {
    /// The board he answers to. Adopted on first contact.
    pub board: ActorRef,
    /// −100..=100: does he believe they back him? A belief about the
    /// people, distinct from `ManagerRelationship`, which is their
    /// belief about him.
    board_faith_pct: i8,
    /// −100..=100: his standing in the dressing room.
    room_pct: i8,
    /// −100..=100: his standing with the supporters.
    terraces_pct: i8,
    /// Promises the board has broken to him. Never resets while he is at
    /// the club — this is the count that turns into a conviction.
    pub promises_broken: u8,
    /// Targets he asked for and did not get.
    pub refusals: u8,
}

impl AuthorityMind {
    /// How far one event moves a standing.
    pub const SHIFT: f32 = 0.16;

    /// Faith below which he stops believing anything they tell him.
    pub const DISILLUSIONED: f32 = -0.40;

    /// Refusals after which asking publicly is the only lever left.
    pub const REFUSALS_BEFORE_HE_SAYS_IT: u8 = 3;

    #[inline]
    pub fn board_faith(&self) -> f32 {
        self.board_faith_pct as f32 / 100.0
    }

    #[inline]
    pub fn room(&self) -> f32 {
        self.room_pct as f32 / 100.0
    }

    #[inline]
    pub fn terraces(&self) -> f32 {
        self.terraces_pct as f32 / 100.0
    }

    /// Has he lost the dressing room? Not a flag — the reading a manager
    /// actually has, which is a continuous one right up until it is not.
    #[inline]
    pub fn has_lost_the_room(&self) -> bool {
        self.room() < -0.5
    }

    fn shift(axis: &mut i8, delta: f32) {
        let current = *axis as f32 / 100.0;
        *axis = ((current + delta).clamp(-1.0, 1.0) * 100.0).round() as i8;
    }

    /// A new board, or none.
    pub fn on_club_change(&mut self, board: ActorRef) {
        self.board = board;
        self.board_faith_pct = 0;
        self.room_pct = 0;
        self.terraces_pct = 0;
        self.promises_broken = 0;
        self.refusals = 0;
    }
}

impl StaffSubMind for AuthorityMind {
    fn domain(&self) -> GoalDomain {
        GoalDomain::Boardroom
    }

    fn observe(&mut self, episode: &MindEpisode, _organs: &mut StaffOrgans) {
        if self.board.is_none() && episode.who.kind == ActorKind::Board {
            self.board = episode.who;
        }

        match episode.kind {
            EpisodeKind::BoardKeptItsPromise => Self::shift(&mut self.board_faith_pct, Self::SHIFT),
            EpisodeKind::BoardBackedMeInTheWindow => {
                Self::shift(&mut self.board_faith_pct, Self::SHIFT * 1.2);
                self.refusals = 0;
            }
            EpisodeKind::BoardBrokeItsPromise => {
                self.promises_broken = self.promises_broken.saturating_add(1);
                Self::shift(&mut self.board_faith_pct, -Self::SHIFT * 2.0);
            }
            EpisodeKind::BoardRefusedMyTarget => {
                self.refusals = self.refusals.saturating_add(1);
                Self::shift(&mut self.board_faith_pct, -Self::SHIFT * 0.6);
            }
            EpisodeKind::BoardSoldMyBestPlayer => {
                Self::shift(&mut self.board_faith_pct, -Self::SHIFT * 1.5)
            }
            EpisodeKind::ChairmanUndercutMePublicly => {
                Self::shift(&mut self.board_faith_pct, -Self::SHIFT * 1.6);
                Self::shift(&mut self.terraces_pct, -Self::SHIFT * 0.5);
            }
            // Not a kindness. He knows what it means, and so does
            // everyone else — which is why it costs him with the
            // supporters as well.
            EpisodeKind::GivenAVoteOfConfidence => {
                Self::shift(&mut self.board_faith_pct, -Self::SHIFT * 0.5);
                Self::shift(&mut self.terraces_pct, -Self::SHIFT * 0.3);
            }

            EpisodeKind::LostTheDressingRoom => Self::shift(&mut self.room_pct, -Self::SHIFT * 5.0),
            EpisodeKind::SquadFoughtForMe => Self::shift(&mut self.room_pct, Self::SHIFT * 1.5),
            EpisodeKind::PlayerRefusedToPlayForMe => {
                Self::shift(&mut self.room_pct, -Self::SHIFT * 1.2)
            }
            EpisodeKind::PlayerRepaidMyFaith => Self::shift(&mut self.room_pct, Self::SHIFT * 0.6),

            EpisodeKind::SupportersSangMyName => {
                Self::shift(&mut self.terraces_pct, Self::SHIFT * 1.4)
            }
            EpisodeKind::SupportersTurnedOnMe => {
                Self::shift(&mut self.terraces_pct, -Self::SHIFT * 1.6)
            }
            EpisodeKind::MediaWroteMeOff => Self::shift(&mut self.terraces_pct, -Self::SHIFT * 0.4),
            EpisodeKind::WonManagerOfTheMonth => {
                Self::shift(&mut self.terraces_pct, Self::SHIFT * 0.5)
            }
            EpisodeKind::AppointedManager | EpisodeKind::PromotedFromWithin => {
                let board = if episode.where_club == 0 {
                    ActorRef::NONE
                } else {
                    ActorRef::board(episode.where_club)
                };
                self.on_club_change(board);
            }
            _ => {}
        }
    }

    fn reflect(&mut self, view: &StaffView<'_>, organs: &mut StaffOrgans) {
        let s = view.situation;
        let today = view.today();

        let board = if view.tick.club_id == 0 {
            ActorRef::NONE
        } else {
            ActorRef::board(view.tick.club_id)
        };
        if board.is_some() && board != self.board {
            self.on_club_change(board);
            return;
        }
        if self.board.is_none() {
            return;
        }

        // The three standings drift toward what they actually are. He is
        // not blind — he just finds out slowly, and a conviction he
        // already holds slows him down further.
        let word_is_worthless = organs
            .memory()
            .believes(FactClaim::TheirWordIsWorthless, self.board);
        let stubbornness = 1.0 - word_is_worthless * 0.5;

        let faith_truth = (s.board_backing * 0.6 + s.board_trust * 0.4) * 2.0 - 1.0;
        let faith_gap = (faith_truth - self.board_faith()) * 0.12 * stubbornness;
        let room_gap = (s.dressing_room * 2.0 - 1.0 - self.room()) * 0.14;
        let terraces_gap = (s.terraces * 2.0 - 1.0 - self.terraces()) * 0.14;
        Self::shift(&mut self.board_faith_pct, faith_gap);
        Self::shift(&mut self.room_pct, room_gap);
        Self::shift(&mut self.terraces_pct, terraces_gap);

        // ── What he wants from them ─────────────────────────────
        if self.refusals >= Self::REFUSALS_BEFORE_HE_SAYS_IT || s.board_backing < 0.35 {
            let mut evidence = GoalEvidence::EMPTY;
            if self.promises_broken > 0 {
                evidence.insert(GoalEvidence::PROMISE_BROKEN);
            }
            if s.squad_is_his < 0.3 {
                evidence.insert(GoalEvidence::SQUAD_BELOW_HIS_LEVEL);
            }
            organs.shared.goals.pursue(
                GoalKind::BeBackedInTheMarket,
                GoalOrigin::Grievance,
                evidence,
                0.28,
                today,
            );
        }

        // Time is the thing a rebuilding manager asks for, and the thing
        // a board under pressure is least able to give.
        if s.squad_is_his > 0.25 && s.against_expectation() < 0.0 && s.months_in_the_job < 36 {
            organs.shared.goals.pursue(
                GoalKind::BeGivenTime,
                GoalOrigin::SelfDrive,
                GoalEvidence::EMPTY,
                0.20,
                today,
            );
        }

        if self.room() < -0.2 {
            organs.shared.goals.pursue(
                GoalKind::RestoreOrderInTheRoom,
                GoalOrigin::Circumstance,
                GoalEvidence::of(&[GoalEvidence::ISOLATED_IN_THE_SQUAD]),
                -self.room() * 0.5,
                today,
            );
        } else if self.room() > 0.3 {
            organs
                .shared
                .goals
                .advance(GoalKind::RestoreOrderInTheRoom, 0.25);
        }

        if s.board_backing > 0.65 {
            organs
                .shared
                .goals
                .advance(GoalKind::BeBackedInTheMarket, 0.30);
        }
    }

    fn appraise(&self, organs: &StaffOrgans) -> MoodContribution {
        if self.board.is_none() {
            return MoodContribution::silent(GoalDomain::Boardroom);
        }

        // Weighted the way a manager actually experiences it: the board
        // can end him, the room decides whether the football works, the
        // supporters set the noise level.
        let value = self.board_faith() * 5.0 + self.room() * 3.5 + self.terraces() * 1.5
            - organs.pressure_in(GoalDomain::Boardroom) * 2.5;

        // He knows where he stands with people he has dealt with. A
        // manager one week into a job does not.
        let dealings = (self.promises_broken + self.refusals) as f32;
        let confidence = (0.25 + dealings * 0.15).clamp(0.0, 1.0);
        MoodContribution::new(GoalDomain::Boardroom, value, confidence)
    }

    fn weigh(&self, option: MindOption, organs: &StaffOrgans) -> ReasonSet {
        let mut reasons = ReasonSet::new();

        match option {
            MindOption::TakeTheJob(club_id) => {
                // The board specifically, not the club. A manager judges
                // the people who will or will not back him separately
                // from the badge — which is why a change of chairman
                // makes a place worth another look.
                let board = ActorRef::board(club_id);
                let worthless = organs
                    .memory()
                    .believes(FactClaim::TheirWordIsWorthless, board);
                let stood_by = organs.memory().believes(FactClaim::TheyStoodByMe, board);

                if worthless > 0.1 {
                    reasons.push(GoalKind::BeBackedInTheMarket, -worthless);
                }
                if stood_by > 0.1 {
                    reasons.push(GoalKind::BeGivenTime, stood_by);
                }
            }

            MindOption::Resign => {
                if self.board_faith() < Self::DISILLUSIONED {
                    reasons.push(GoalKind::BeBackedInTheMarket, -self.board_faith());
                }
                if self.has_lost_the_room() {
                    reasons.push(GoalKind::RestoreOrderInTheRoom, -self.room());
                }
                if self.terraces() > 0.4 {
                    // Supporters on his side are a reason to stay and
                    // fight it out.
                    reasons.push(GoalKind::BeGivenTime, -self.terraces() * 0.6);
                }
            }

            _ => {}
        }

        reasons
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::person::PersonAttributes;
    use crate::club::staff::mind::{StaffMind, StaffSituation, StaffTickContext};
    use chrono::NaiveDate;

    /// Fixture builders. Grouped on a type rather than left loose, so
    /// the file reads as `Fixture::context()` at every call site.
    struct Fixture;

    impl Fixture {
        fn attributes() -> PersonAttributes {
            PersonAttributes {
                adaptability: 12.0,
                ambition: 12.0,
                controversy: 5.0,
                loyalty: 12.0,
                pressure: 12.0,
                professionalism: 13.0,
                sportsmanship: 12.0,
                temperament: 10.0,
                consistency: 12.0,
                important_matches: 12.0,
                dirtiness: 4.0,
            }
        }

        fn date(year: i32, month: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(year, month, day).expect("valid fixture date")
        }

        fn context(date: NaiveDate, club: u32) -> StaffTickContext {
            StaffTickContext::new(date, club, &Self::attributes(), 50.0)
        }
    }

    #[test]
    fn refusals_accumulate_into_a_public_plea_for_signings() {
        let mut mind = StaffMind::new();
        let c = Fixture::context(Fixture::date(2030, 7, 10), 7);
        for _ in 0..4 {
            mind.remember(EpisodeKind::BoardRefusedMyTarget, ActorRef::board(7), &c);
        }

        let mut situation = StaffSituation::neutral();
        situation.board_backing = 0.2;
        for week in 0..10 {
            let date = Fixture::date(2030, 7, 10) + chrono::Duration::days(week * 7);
            mind.tick_with(&Fixture::context(date, 7), &situation);
        }

        assert!(mind.pressure_of(GoalKind::BeBackedInTheMarket) > 0.2);
        assert!(mind.authority.board_faith() < 0.0);
        assert_eq!(mind.authority.refusals, 4);
    }

    #[test]
    fn the_three_standings_move_independently() {
        let mut mind = StaffMind::new();
        let c = Fixture::context(Fixture::date(2030, 9, 1), 7);
        mind.remember(EpisodeKind::SupportersSangMyName, ActorRef::fans(7), &c);
        mind.remember(EpisodeKind::BoardBrokeItsPromise, ActorRef::board(7), &c);

        assert!(mind.authority.terraces() > 0.0);
        assert!(mind.authority.board_faith() < 0.0);
        assert_eq!(
            mind.authority.room(),
            0.0,
            "neither of those tells him anything about the dressing room"
        );
    }

    #[test]
    fn losing_the_dressing_room_is_not_recoverable_in_an_afternoon() {
        let mut mind = StaffMind::new();
        let c = Fixture::context(Fixture::date(2030, 10, 1), 7);
        mind.remember(EpisodeKind::LostTheDressingRoom, ActorRef::club(7), &c);
        assert!(mind.authority.has_lost_the_room());

        mind.remember(EpisodeKind::SquadFoughtForMe, ActorRef::club(7), &c);
        assert!(
            mind.authority.has_lost_the_room(),
            "one good afternoon does not undo it"
        );
    }

    #[test]
    fn a_broken_promise_is_worth_three_refusals() {
        let mut broken = StaffMind::new();
        let mut refused = StaffMind::new();
        let c = Fixture::context(Fixture::date(2030, 7, 10), 7);

        broken.remember(EpisodeKind::BoardBrokeItsPromise, ActorRef::board(7), &c);
        for _ in 0..3 {
            refused.remember(EpisodeKind::BoardRefusedMyTarget, ActorRef::board(7), &c);
        }

        assert!(
            broken.authority.board_faith() <= refused.authority.board_faith(),
            "being told no is a disagreement; being lied to is not"
        );
    }

    #[test]
    fn a_manager_with_no_board_has_nothing_to_say_about_one() {
        let mind = StaffMind::new();
        assert!(mind.authority.appraise(&mind.organs).is_silent());
    }
}
