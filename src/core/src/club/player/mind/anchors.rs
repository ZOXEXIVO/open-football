//! What a career leaves behind: how high he has been, what he is reaching for,
//! and the dated turns of it.
//!
//! Anchors last a career, plans last seasons, wants last weeks. Anchors feed
//! plans and plans feed wants; a want never writes an anchor, and nothing about
//! a change of club touches one.

use super::organs::MindOrgans;
use super::organs::memory::{ActorRef, EpochDay, FactClaim, FixedStore, MindClock};
use super::situation::MindSituation;
use crate::{ClubLevelAnchor, PlayerFieldPositionGroup};

/// The strongest division he has been a regular in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CareerSummit {
    /// On the league-reputation scale, 0..10000.
    pub league_reputation: u16,
    pub club_id: u32,
    /// The club's standing then, 0..1.
    pub club_standing: f32,
    pub reached_on: EpochDay,
    pub last_regular_on: EpochDay,
    /// What he looked like the last time he was a regular at this level:
    /// the proof he could play there, against which a man whose ability has
    /// gone reads as no longer holding it.
    pub level_then: u8,
}

impl CareerSummit {
    /// A division's reputation drifts by this much between seasons without
    /// being a different level of football.
    const SAME_LEVEL_MARGIN: u16 = 250;
    /// A new record only reads as a turn in his career past this.
    const NEW_LEVEL_STEP: u16 = 500;
    /// Stands in full for a year after he was last a regular there …
    const FULL_HOLD_DAYS: f32 = 365.0;
    /// … then fades to nothing over the following years …
    const FADE_DAYS: f32 = 4.0 * 365.0;
    /// … faster from this age, by this much a year.
    const FADES_FASTER_FROM: u8 = 30;
    const AGE_PACE: f32 = 0.25;
    /// Observable levels he can lose before anybody thinks less of him,
    /// and how many more until he no longer looks like that player at all.
    const LEVEL_SLACK: f32 = 5.0;
    const LEVEL_SPAN: f32 = 20.0;

    /// How firmly he still holds it, 0..1. `own_level` 0 is no view.
    pub fn hold(&self, today: EpochDay, age: u8, own_level: u8) -> f32 {
        let since = MindClock::elapsed_f32(self.last_regular_on, today);
        let pace = 1.0 + age.saturating_sub(Self::FADES_FASTER_FROM) as f32 * Self::AGE_PACE;
        let by_time = 1.0 - (since - Self::FULL_HOLD_DAYS).max(0.0) * pace / Self::FADE_DAYS;
        let by_level = if own_level == 0 {
            1.0
        } else {
            let lost = self.level_then as f32 - own_level as f32 - Self::LEVEL_SLACK;
            1.0 - lost.max(0.0) / Self::LEVEL_SPAN
        };
        by_time.min(by_level).clamp(0.0, 1.0)
    }

    /// What decisions read: the record, faded by how firmly he holds it.
    pub fn held(&self, today: EpochDay, age: u8, own_level: u8) -> u16 {
        (self.league_reputation as f32 * self.hold(today, age, own_level)) as u16
    }
}

#[cfg(test)]
impl CareerDream {
    /// A dream held from `today`, for tests in the modules that read one.
    pub(crate) fn default_on(today: EpochDay) -> Self {
        CareerDream {
            kind: DreamKind::MakeACareer,
            target_standing: 0.0,
            target_club: 0,
            conviction: 0.5,
            arrived: false,
            arrived_on: 0,
            last_there_on: today,
            formed_on: today,
            reviewed_on: today,
            windows_missed: 0,
            standing_at_review: 0.0,
        }
    }
}

/// What he is reaching for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DreamKind {
    /// The strongest level he can see himself starting at.
    ReachTheTop,
    /// A named club he comes from or has adored, when he is not there.
    PlayForHisClub,
    /// To be the man at the club where he is.
    BelongHere,
    /// To end his career in his own country.
    FinishAtHome,
    /// To keep playing at a level that pays, with no higher aim.
    #[default]
    MakeACareer,
}

impl DreamKind {
    pub const ALL: [DreamKind; 5] = [
        DreamKind::ReachTheTop,
        DreamKind::PlayForHisClub,
        DreamKind::BelongHere,
        DreamKind::FinishAtHome,
        DreamKind::MakeACareer,
    ];

    pub fn as_i18n_key(self) -> &'static str {
        match self {
            DreamKind::ReachTheTop => "career_dream_reach_the_top",
            DreamKind::PlayForHisClub => "career_dream_play_for_his_club",
            DreamKind::BelongHere => "career_dream_belong_here",
            DreamKind::FinishAtHome => "career_dream_finish_at_home",
            DreamKind::MakeACareer => "career_dream_make_a_career",
        }
    }

    /// The same dream, as the words a chapter uses for it.
    pub fn short_i18n_key(self) -> &'static str {
        match self {
            DreamKind::ReachTheTop => "career_dream_short_reach_the_top",
            DreamKind::PlayForHisClub => "career_dream_short_play_for_his_club",
            DreamKind::BelongHere => "career_dream_short_belong_here",
            DreamKind::FinishAtHome => "career_dream_short_finish_at_home",
            DreamKind::MakeACareer => "career_dream_short_make_a_career",
        }
    }
}

/// Where he stands with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DreamState {
    /// He is still reaching for it.
    Alive,
    /// He is living it.
    Arrived,
    /// He has had it, and wants it back.
    WantsItBack,
}

impl DreamState {
    pub fn as_i18n_key(self) -> &'static str {
        match self {
            DreamState::Alive => "career_dream_state_alive",
            DreamState::Arrived => "career_dream_state_arrived",
            DreamState::WantsItBack => "career_dream_state_wants_it_back",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CareerDream {
    pub kind: DreamKind,
    /// Club standing he is reaching for, 0..1, where the dream names a level.
    pub target_standing: f32,
    /// The club the dream names, 0 when it names none.
    pub target_club: u32,
    /// How much he believes it, 0..1.
    pub conviction: f32,
    /// Is he living it now?
    pub arrived: bool,
    /// When he first did, 0 when he never has.
    pub arrived_on: EpochDay,
    /// The last day he read as living it.
    pub last_there_on: EpochDay,
    pub formed_on: EpochDay,
    pub reviewed_on: EpochDay,
    /// Windows he wanted to move in and nobody came, since the last review.
    pub windows_missed: u8,
    /// The standing of the club he was at when he last looked at it, so a
    /// review can say whether he has come closer.
    pub standing_at_review: f32,
}

impl CareerDream {
    /// The club standing at which he is at the top of the game. Football
    /// has no single summit: a target beyond this is met at any of the
    /// dozen clubs up here, or nobody but one squad would ever arrive.
    pub const TOP_OF_THE_GAME: f32 = 0.88;
    /// A level is met a little under its number, and kept a little further:
    /// a club's reputation moves a little every week.
    const ARRIVAL_SLACK: f32 = 0.02;
    const STAYING_SLACK: f32 = 0.05;
    /// Service before a man reads as belonging somewhere.
    const BELONGS_AFTER_DAYS: u16 = 3 * 365;
    /// A regular who has gone this long without being one has lost his place.
    const PLACE_LOST_AFTER_DAYS: u16 = 120;

    pub fn state(&self) -> DreamState {
        if self.arrived {
            DreamState::Arrived
        } else if self.arrived_on > 0 {
            DreamState::WantsItBack
        } else {
            DreamState::Alive
        }
    }

    fn is_where_he_is(&self, s: &MindSituation, club_id: u32, slack: f32) -> bool {
        match self.kind {
            DreamKind::ReachTheTop | DreamKind::MakeACareer => {
                s.club_reputation >= self.target_standing.min(Self::TOP_OF_THE_GAME) - slack
            }
            DreamKind::PlayForHisClub => club_id == self.target_club,
            DreamKind::BelongHere => {
                club_id == self.target_club && s.days_at_club >= Self::BELONGS_AFTER_DAYS
            }
            DreamKind::FinishAtHome => club_id != 0 && !s.is_abroad,
        }
    }

    fn is_living_it(&self, s: &MindSituation) -> bool {
        match self.kind {
            // He is home. Nobody asks a man how often he starts there.
            DreamKind::FinishAtHome => true,
            _ => s.is_regular_here(),
        }
    }
}

/// A turning point in his career, for a reader. Nothing in the simulation
/// decides anything by reading one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ChapterKind {
    #[default]
    None,
    DreamFormed,
    DreamRaised,
    DreamLowered,
    DreamLetGo,
    Arrived,
    Regained,
    LostIt,
    NewSummit,
    FirstSeniorClub,
    SteppedUp,
    Fell,
    Homecoming,
    ReunitedWithManager,
    FirstMajorHonour,
}

impl ChapterKind {
    pub const ALL: [ChapterKind; 14] = [
        ChapterKind::DreamFormed,
        ChapterKind::DreamRaised,
        ChapterKind::DreamLowered,
        ChapterKind::DreamLetGo,
        ChapterKind::Arrived,
        ChapterKind::Regained,
        ChapterKind::LostIt,
        ChapterKind::NewSummit,
        ChapterKind::FirstSeniorClub,
        ChapterKind::SteppedUp,
        ChapterKind::Fell,
        ChapterKind::Homecoming,
        ChapterKind::ReunitedWithManager,
        ChapterKind::FirstMajorHonour,
    ];

    /// Which give way when the list is full: the least significant, and of
    /// those the oldest, so his summit and his dream's last turn outlast
    /// ordinary entries.
    fn significance(self) -> u8 {
        match self {
            ChapterKind::None => 0,
            ChapterKind::ReunitedWithManager => 2,
            ChapterKind::SteppedUp | ChapterKind::Fell => 3,
            ChapterKind::FirstSeniorClub | ChapterKind::Homecoming => 4,
            ChapterKind::Regained | ChapterKind::LostIt | ChapterKind::FirstMajorHonour => 5,
            ChapterKind::DreamRaised | ChapterKind::DreamLowered => 6,
            ChapterKind::DreamFormed | ChapterKind::DreamLetGo => 7,
            ChapterKind::NewSummit | ChapterKind::Arrived => 8,
        }
    }

    pub fn as_i18n_key(self) -> &'static str {
        match self {
            ChapterKind::None => "career_chapter_none",
            ChapterKind::DreamFormed => "career_chapter_dream_formed",
            ChapterKind::DreamRaised => "career_chapter_dream_raised",
            ChapterKind::DreamLowered => "career_chapter_dream_lowered",
            ChapterKind::DreamLetGo => "career_chapter_dream_let_go",
            ChapterKind::Arrived => "career_chapter_arrived",
            ChapterKind::Regained => "career_chapter_regained",
            ChapterKind::LostIt => "career_chapter_lost_it",
            ChapterKind::NewSummit => "career_chapter_new_summit",
            ChapterKind::FirstSeniorClub => "career_chapter_first_senior_club",
            ChapterKind::SteppedUp => "career_chapter_stepped_up",
            ChapterKind::Fell => "career_chapter_fell",
            ChapterKind::Homecoming => "career_chapter_homecoming",
            ChapterKind::ReunitedWithManager => "career_chapter_reunited_with_manager",
            ChapterKind::FirstMajorHonour => "career_chapter_first_major_honour",
        }
    }

    /// Which way the turn reads.
    pub fn valence(self) -> i8 {
        match self {
            ChapterKind::None | ChapterKind::DreamFormed => 0,
            ChapterKind::DreamLowered
            | ChapterKind::DreamLetGo
            | ChapterKind::LostIt
            | ChapterKind::Fell => -1,
            ChapterKind::DreamRaised
            | ChapterKind::Arrived
            | ChapterKind::Regained
            | ChapterKind::NewSummit
            | ChapterKind::FirstSeniorClub
            | ChapterKind::SteppedUp
            | ChapterKind::Homecoming
            | ChapterKind::ReunitedWithManager
            | ChapterKind::FirstMajorHonour => 1,
        }
    }

    /// The turns a career is remembered by, not a season's news.
    pub fn is_major(self) -> bool {
        matches!(
            self,
            ChapterKind::Arrived
                | ChapterKind::Regained
                | ChapterKind::LostIt
                | ChapterKind::NewSummit
                | ChapterKind::DreamLetGo
                | ChapterKind::FirstMajorHonour
        )
    }

    fn is_about_his_dream(self) -> bool {
        matches!(
            self,
            ChapterKind::DreamFormed
                | ChapterKind::DreamRaised
                | ChapterKind::DreamLowered
                | ChapterKind::DreamLetGo
                | ChapterKind::Arrived
                | ChapterKind::Regained
                | ChapterKind::LostIt
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CareerChapter {
    pub kind: ChapterKind,
    /// The dream it is about, when it is about one.
    pub dream: DreamKind,
    pub on: EpochDay,
    pub club_id: u32,
    pub league_reputation: u16,
}

impl CareerChapter {
    fn rank(&self) -> f32 {
        self.kind.significance() as f32 * 100_000.0 + self.on as f32
    }
}

/// The long-term facts a player's decisions rest on.
#[derive(Debug, Clone, Copy, Default)]
pub struct CareerAnchors {
    pub summit: Option<CareerSummit>,
    /// The level he has most lately been a regular at. His record can sit
    /// a division above it and fade, and a man dropped from a side at this
    /// level still claims this level.
    latest_regular: Option<CareerSummit>,
    pub dream: Option<CareerDream>,
    pub chapters: FixedStore<CareerChapter, 14>,
    /// The share of his prime he read as still ahead of him at his last
    /// think, 0..1 — what a long deal would take out of a climb.
    pub(super) runway: f32,
    /// A dream he let go, and how far he could reach when he did. He does
    /// not talk himself back into it unless something has changed.
    given_up: Option<(DreamKind, f32)>,
    last_club: u32,
    /// The level he last read himself as playing at: see [`Self::level_of`].
    last_level: f32,
    last_manager: ActorRef,
    was_regular: bool,
    was_abroad: bool,
    was_on_loan: bool,
    pub(super) told_first_club: bool,
}

/// What a move did to his career, for the trajectory he reads of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CareerMoveRead {
    pub stepped_up: bool,
    pub fell: bool,
}

impl CareerAnchors {
    #[cfg(test)]
    pub(crate) fn set_runway_for_test(&mut self, runway: f32) {
        self.runway = runway;
    }

    /// How far a move has to change his level to read as a step up or a fall.
    const MOVE_STEP: f32 = 0.05;

    /// The football he plays, 0..1: the club and the division together. Paris
    /// from mid-table in a stronger league is no fall, and a famous name in a
    /// far weaker one is no step up.
    fn level_of(s: &MindSituation) -> f32 {
        (s.division_reputation as f32 / 10_000.0 + s.club_reputation) / 2.0
    }

    /// When his dream last turned.
    pub fn dream_last_turned(&self) -> Option<EpochDay> {
        self.chapters
            .iter()
            .filter(|chapter| chapter.kind.is_about_his_dream())
            .map(|chapter| chapter.on)
            .max()
    }

    /// The last dream he gave up on.
    pub fn last_given_up(&self) -> Option<CareerChapter> {
        self.chapters
            .iter()
            .filter(|chapter| chapter.kind == ChapterKind::DreamLetGo)
            .max_by_key(|chapter| chapter.on)
            .copied()
    }

    /// Writes a turn. A new one that matters less than all he holds gives
    /// way itself, and otherwise the least of them does.
    pub fn tell(&mut self, chapter: CareerChapter) {
        if self.chapters.is_full()
            && self
                .chapters
                .iter()
                .all(|held| held.kind.significance() > chapter.kind.significance())
        {
            return;
        }
        self.chapters
            .push_evicting(chapter, CareerChapter::rank, |_| true);
    }

    fn tell_on(
        &mut self,
        kind: ChapterKind,
        dream: DreamKind,
        s: &MindSituation,
        club_id: u32,
        today: EpochDay,
    ) {
        self.tell(CareerChapter {
            kind,
            dream,
            on: today,
            club_id,
            league_reputation: s.division_reputation,
        });
    }

    /// A regular, over a sample anybody would believe, and never lower for
    /// a fall.
    pub(super) fn record_summit(&mut self, s: &MindSituation, club_id: u32, today: EpochDay) {
        if !s.is_regular_here() || s.division_reputation == 0 {
            return;
        }
        let division = s.division_reputation;
        self.latest_regular = Some(CareerSummit {
            league_reputation: division,
            club_id,
            club_standing: s.club_reputation,
            reached_on: today,
            last_regular_on: today,
            level_then: s.own_level,
        });
        if let Some(summit) = self.summit.as_mut()
            && division <= summit.league_reputation
        {
            if division + CareerSummit::SAME_LEVEL_MARGIN >= summit.league_reputation {
                summit.last_regular_on = today;
                if s.own_level > 0 {
                    summit.level_then = s.own_level;
                }
            }
            return;
        }
        let a_new_level = self
            .summit
            .is_none_or(|was| division >= was.league_reputation + CareerSummit::NEW_LEVEL_STEP);
        let (reached_on, reached_at, standing) = match self.summit {
            Some(was) if !a_new_level => (was.reached_on, was.club_id, was.club_standing),
            _ => (today, club_id, s.club_reputation),
        };
        self.summit = Some(CareerSummit {
            league_reputation: division,
            club_id: reached_at,
            club_standing: standing,
            reached_on,
            last_regular_on: today,
            level_then: s.own_level,
        });
        if a_new_level {
            self.tell_on(
                ChapterKind::NewSummit,
                DreamKind::default(),
                s,
                club_id,
                today,
            );
        }
    }

    /// Is he living it? Recorded once, and lost when he stops being a
    /// regular there or leaves.
    pub(super) fn read_arrival(&mut self, s: &MindSituation, club_id: u32, today: EpochDay) {
        let Some(mut dream) = self.dream else {
            return;
        };
        if !dream.arrived {
            if dream.is_where_he_is(s, club_id, CareerDream::ARRIVAL_SLACK) && dream.is_living_it(s)
            {
                let kind = if dream.arrived_on == 0 {
                    dream.arrived_on = today;
                    ChapterKind::Arrived
                } else {
                    ChapterKind::Regained
                };
                dream.arrived = true;
                dream.last_there_on = today;
                self.tell_on(kind, dream.kind, s, club_id, today);
            }
        } else if !dream.is_where_he_is(s, club_id, CareerDream::STAYING_SLACK) {
            dream.arrived = false;
            self.tell_on(ChapterKind::LostIt, dream.kind, s, club_id, today);
        } else if dream.is_living_it(s) {
            dream.last_there_on = today;
        } else if s.has_playing_view()
            && MindClock::elapsed(dream.last_there_on, today) >= CareerDream::PLACE_LOST_AFTER_DAYS
        {
            dream.arrived = false;
            self.tell_on(ChapterKind::LostIt, dream.kind, s, club_id, today);
        }
        self.dream = Some(dream);
    }

    /// Where he is against where he was last week: the moves, the
    /// homecoming and the reunion, whichever door they came through. A loan
    /// return is not a step up, it is going home.
    pub(super) fn read_the_move(
        &mut self,
        s: &MindSituation,
        club_id: u32,
        organs: &MindOrgans,
        today: EpochDay,
    ) -> CareerMoveRead {
        let mut read = CareerMoveRead::default();
        let moved = club_id != 0 && self.last_club != 0 && club_id != self.last_club;
        let returned_from_loan = self.was_on_loan && !s.is_on_loan;
        let level = Self::level_of(s);
        if moved && !returned_from_loan && self.last_level > 0.0 && s.division_reputation > 0 {
            if level >= self.last_level + Self::MOVE_STEP && s.in_first_team {
                read.stepped_up = true;
                self.tell_on(
                    ChapterKind::SteppedUp,
                    DreamKind::default(),
                    s,
                    club_id,
                    today,
                );
            } else if level + Self::MOVE_STEP <= self.last_level && self.was_regular {
                read.fell = true;
                self.tell_on(ChapterKind::Fell, DreamKind::default(), s, club_id, today);
            }
        }
        if moved && self.was_abroad && !s.is_abroad {
            self.tell_on(
                ChapterKind::Homecoming,
                DreamKind::default(),
                s,
                club_id,
                today,
            );
        }
        if s.manager != ActorRef::NONE
            && s.manager != self.last_manager
            && organs.memory.believes(FactClaim::MadeMeAPlayer, s.manager) > 0.4
        {
            self.tell_on(
                ChapterKind::ReunitedWithManager,
                DreamKind::default(),
                s,
                club_id,
                today,
            );
        }
        if !self.told_first_club && s.in_first_team && s.appearances_tracked > 0 {
            self.told_first_club = true;
            self.tell_on(
                ChapterKind::FirstSeniorClub,
                DreamKind::default(),
                s,
                club_id,
                today,
            );
        }
        if club_id != 0 {
            self.last_club = club_id;
            self.last_level = level;
            self.was_regular = s.is_regular_here();
            self.was_abroad = s.is_abroad;
            self.was_on_loan = s.is_on_loan;
        }
        if s.manager != ActorRef::NONE {
            self.last_manager = s.manager;
        }
        read
    }

    /// His dream, formed the once. A chapter dates it, and it is left alone
    /// until a review says otherwise.
    pub(super) fn form_dream(&mut self, s: &MindSituation, club_id: u32, today: EpochDay) {
        let dream = CareerDreamReview::form(s, club_id, today);
        self.dream = Some(dream);
        self.tell_on(ChapterKind::DreamFormed, dream.kind, s, club_id, today);
    }

    /// A season on: conviction, the target, and whether it can still be
    /// held. `improvement` is his own read of whether he is getting better.
    pub(super) fn review_dream(
        &mut self,
        s: &MindSituation,
        club_id: u32,
        improvement: f32,
        today: EpochDay,
    ) {
        let Some(mut dream) = self.dream else {
            return;
        };
        // Everything the review reads about belonging is about the club he
        // is lent to. It waits until he is home.
        if s.is_on_loan
            || MindClock::elapsed(dream.reviewed_on, today) < CareerDreamReview::REVIEW_DAYS
        {
            return;
        }
        dream.conviction = CareerDreamReview::conviction_after(&dream, s, improvement);
        dream.windows_missed = 0;
        dream.reviewed_on = today;
        dream.standing_at_review = s.club_reputation;

        let turn = CareerDreamReview::revise_target(&mut dream, s);
        if let Some(kind) = turn {
            self.tell_on(kind, dream.kind, s, club_id, today);
        }

        if let Some(next) = CareerDreamReview::replaced_by(&dream, s, club_id, self.given_up) {
            self.given_up = Some((dream.kind, CareerDreamReview::reach_of(s)));
            self.tell_on(ChapterKind::DreamLetGo, dream.kind, s, club_id, today);
            self.dream = Some(CareerDreamReview::formed_as(next, s, club_id, today));
            self.tell_on(ChapterKind::DreamFormed, next, s, club_id, today);
            return;
        }
        self.dream = Some(dream);
    }

    /// He wanted to move this window and nobody came.
    pub fn on_window_missed(&mut self) {
        if let Some(dream) = self.dream.as_mut() {
            dream.windows_missed = dream.windows_missed.saturating_add(1);
        }
    }
}

/// His anchors, flattened for everything outside the mind. Nothing about him
/// has to be reachable to read it, so a cross-border negotiation sees the same
/// man a domestic one does.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AnchorView {
    /// The division he can still claim, 0..10000. 0 when he holds none.
    pub held_summit: u16,
    /// How firmly he holds it, 0..1.
    pub recency: f32,
    pub dream: Option<DreamKind>,
    pub target_standing: f32,
    pub target_club: u32,
    pub conviction: f32,
    pub arrived: bool,
}

impl AnchorView {
    /// Club standing a move has to cover to count for all it can.
    const CAREER_STEP: f32 = 0.30;

    pub fn of(anchors: &CareerAnchors, today: EpochDay, age: u8, own_level: u8) -> Self {
        let (held_summit, recency) = [anchors.summit, anchors.latest_regular]
            .into_iter()
            .flatten()
            .map(|record| {
                (
                    record.held(today, age, own_level),
                    record.hold(today, age, own_level),
                )
            })
            .max_by_key(|(held, _)| *held)
            .unwrap_or((0, 0.0));
        let dream = anchors.dream;
        AnchorView {
            held_summit,
            recency,
            dream: dream.map(|d| d.kind),
            target_standing: dream.map_or(0.0, |d| d.target_standing),
            target_club: dream.map_or(0, |d| d.target_club),
            conviction: dream.map_or(0.0, |d| d.conviction),
            arrived: dream.is_some_and(|d| d.arrived),
        }
    }

    /// Is he reaching for a level above the club at this standing?
    pub fn reaches_above(&self, club_standing: f32) -> bool {
        self.dream == Some(DreamKind::ReachTheTop)
            && !self.arrived
            && self.target_standing.min(CareerDream::TOP_OF_THE_GAME)
                > club_standing + CareerDream::ARRIVAL_SLACK
    }

    /// Is the club he is at the one he means to belong to?
    pub fn belongs_at(&self, club_id: u32) -> bool {
        self.dream == Some(DreamKind::BelongHere) && club_id != 0 && self.target_club == club_id
    }

    /// What a permanent move does for the dream he holds, -1..1: how much
    /// nearer the level it names, times how much he believes in it. Going
    /// past that level pays nothing. A dream to stay, go home or earn a
    /// living names no level, and has its say through the wants it feeds.
    pub fn career_pull(&self, from_standing: f32, buyer_standing: f32, buyer_club: u32) -> f32 {
        match self.dream {
            Some(DreamKind::PlayForHisClub)
                if buyer_club != 0 && buyer_club == self.target_club =>
            {
                1.0
            }
            Some(DreamKind::PlayForHisClub) if self.arrived => -self.conviction,
            Some(DreamKind::ReachTheTop) if from_standing > 0.0 && buyer_standing > 0.0 => {
                let target = self.target_standing.min(CareerDream::TOP_OF_THE_GAME);
                let nearer = (target - from_standing).max(0.0) - (target - buyer_standing).max(0.0);
                (nearer / Self::CAREER_STEP).clamp(-1.0, 1.0) * self.conviction
            }
            _ => 0.0,
        }
    }
}

/// One rule for every dream, from a Lithuanian teenager to a Ballon d'Or
/// contender: score each kind, continuously, and hold the strongest.
pub struct CareerDreamReview;

impl CareerDreamReview {
    /// Old enough to have a senior career ahead of him.
    pub const MIN_AGE: u8 = 16;
    /// Once a season, and otherwise left alone.
    pub const REVIEW_DAYS: u16 = 365;

    /// How much a clear margin has to be before a review changes his mind.
    const SWITCH_MARGIN: f32 = 0.10;
    /// Below this he can no longer tell himself he believes it.
    const LET_GO_BELOW: f32 = 0.15;
    /// What a window nobody came in costs him, for a man with no ambition
    /// to hold him up.
    const WINDOW_COST: f32 = 0.12;
    const IMPROVEMENT_GAIN: f32 = 0.15;
    const CLOSER_GAIN: f32 = 1.0;
    const CLOSER_BOUND: f32 = 0.10;
    const LIVING_IT_GAIN: f32 = 0.05;
    /// How far ambition and youth lift what he reaches for, past what his
    /// projected level says.
    const AMBITION_LIFT: f32 = 0.08;
    /// Standing moves smaller than this are not a turn in his career.
    const TARGET_TURN: f32 = 0.03;
    /// Share of a target's fall that ambition refuses, at full ambition.
    const AMBITION_REFUSES: f32 = 0.6;
    /// Years at a club at which he has been there as long as it counts for.
    const SETTLED_YEARS: f32 = 6.0;
    const OUT_OF_THE_SIDE: f32 = 0.6;
    const HOME_AT_HIS_OWN_CLUB: f32 = 0.15;
    /// Spent career times low adaptability, abroad, at full reading.
    const HOME_GAIN: f32 = 1.4;
    /// Spread between two equal drives, read from his id.
    const INDIVIDUAL_SPREAD: f32 = 0.03;
    /// What is left of a dream he has let go, in the argument for it …
    const GIVEN_UP_SHARE: f32 = 0.5;
    /// … until his reach has grown this far past where it was.
    const REACH_REGROWN: f32 = 0.05;

    /// The strongest club standing he can see himself starting at.
    fn standing_of(level: u8, group: PlayerFieldPositionGroup) -> f32 {
        if level == 0 {
            return 0.0;
        }
        ClubLevelAnchor::highest_standing_for(level, group)
    }

    pub(super) fn reach_of(s: &MindSituation) -> f32 {
        let lift = Self::AMBITION_LIFT * s.ambition_drive() * s.career_runway();
        (Self::standing_of(s.ceiling.max(s.own_level), s.position_group) + lift).clamp(0.0, 1.0)
    }

    fn individual_spread(player_id: u32, kind: DreamKind) -> f32 {
        let salt = DreamKind::ALL.iter().position(|k| *k == kind).unwrap_or(0) as u32;
        let hash = player_id.wrapping_add(salt).wrapping_mul(0x9E37_79B1) >> 24;
        hash as f32 / 255.0 * Self::INDIVIDUAL_SPREAD
    }

    /// How strongly his situation argues for a dream, on one scale.
    fn score(kind: DreamKind, s: &MindSituation, club_id: u32) -> f32 {
        let drive = s.ambition_drive();
        let loyal = s.loyalty_drive();
        let runway = s.career_runway();
        let spent = s.career_spent();
        let raw = match kind {
            DreamKind::ReachTheTop => {
                let level = Self::standing_of(s.own_level, s.position_group);
                let room = (level - s.club_reputation).clamp(0.0, 0.5);
                drive * (0.25 + 0.75 * runway) + room * 1.2 * runway
            }
            DreamKind::PlayForHisClub => {
                if s.heart_club == 0 || s.heart_club == club_id {
                    0.0
                } else {
                    (0.2 + 0.7 * loyal) * (0.5 + 0.5 * spent)
                }
            }
            DreamKind::BelongHere => {
                let years = s.days_at_club as f32 / 365.0;
                let settled = (years / Self::SETTLED_YEARS).clamp(0.0, 1.0);
                let in_the_side = if s.has_playing_view() && !s.is_regular_here() {
                    Self::OUT_OF_THE_SIDE
                } else {
                    1.0
                };
                let his_own = if s.heart_club != 0 && s.heart_club == club_id {
                    Self::HOME_AT_HIS_OWN_CLUB
                } else {
                    0.0
                };
                loyal * settled * in_the_side + his_own
            }
            DreamKind::FinishAtHome => {
                if s.is_abroad {
                    let settled_abroad = (s.adaptability / 20.0).clamp(0.0, 1.0);
                    spent * (1.0 - settled_abroad) * Self::HOME_GAIN
                } else {
                    0.0
                }
            }
            DreamKind::MakeACareer => 0.2 + 0.55 * (1.0 - drive),
        };
        raw + Self::individual_spread(s.player_id, kind)
    }

    /// The strongest of the dreams `kinds` admits, with what he has let go
    /// marked down until his reach has grown past where it was.
    fn strongest(
        s: &MindSituation,
        club_id: u32,
        given_up: Option<(DreamKind, f32)>,
        admits: impl Fn(DreamKind) -> bool,
    ) -> (DreamKind, f32) {
        let reach = Self::reach_of(s);
        DreamKind::ALL
            .iter()
            .filter(|kind| admits(**kind))
            .map(|kind| {
                let score = Self::score(*kind, s, club_id);
                let spent = given_up
                    .is_some_and(|(was, then)| was == *kind && reach < then + Self::REACH_REGROWN);
                let score = if spent {
                    score * Self::GIVEN_UP_SHARE
                } else {
                    score
                };
                (*kind, score)
            })
            .fold((DreamKind::MakeACareer, f32::MIN), |best, next| {
                if next.1 > best.1 { next } else { best }
            })
    }

    /// The dream his situation argues for. The same man forms the same one.
    pub fn form(s: &MindSituation, club_id: u32, today: EpochDay) -> CareerDream {
        let (kind, _) = Self::strongest(s, club_id, None, |_| true);
        Self::formed_as(kind, s, club_id, today)
    }

    fn formed_as(kind: DreamKind, s: &MindSituation, club_id: u32, today: EpochDay) -> CareerDream {
        let level = Self::standing_of(s.own_level, s.position_group);
        let (target_standing, target_club) = match kind {
            DreamKind::ReachTheTop => (Self::reach_of(s), 0),
            DreamKind::MakeACareer => (level, 0),
            DreamKind::PlayForHisClub => (0.0, s.heart_club),
            DreamKind::BelongHere => (0.0, club_id),
            DreamKind::FinishAtHome => (0.0, 0),
        };
        CareerDream {
            kind,
            target_standing,
            target_club,
            conviction: (0.4 + 0.6 * Self::score(kind, s, club_id)).clamp(0.0, 1.0),
            arrived: false,
            arrived_on: 0,
            last_there_on: today,
            formed_on: today,
            reviewed_on: today,
            windows_missed: 0,
            standing_at_review: s.club_reputation,
        }
    }

    fn conviction_after(dream: &CareerDream, s: &MindSituation, improvement: f32) -> f32 {
        let ambition_holds = 1.0 - 0.5 * s.ambition_drive();
        let nobody_came = dream.windows_missed as f32 * Self::WINDOW_COST * ambition_holds;
        let closer = (s.club_reputation - dream.standing_at_review)
            .clamp(-Self::CLOSER_BOUND, Self::CLOSER_BOUND)
            * Self::CLOSER_GAIN;
        let living_it = if dream.arrived {
            Self::LIVING_IT_GAIN
        } else {
            0.0
        };
        (dream.conviction - nobody_came
            + improvement.max(0.0) * Self::IMPROVEMENT_GAIN
            + closer
            + living_it)
            .clamp(0.0, 1.0)
    }

    /// Lowered when his level and remaining career no longer reach it,
    /// slower the more ambition he has, and raised when his level has
    /// passed it. A career is simply wherever his level says.
    fn revise_target(dream: &mut CareerDream, s: &MindSituation) -> Option<ChapterKind> {
        let level = Self::standing_of(s.own_level, s.position_group);
        match dream.kind {
            DreamKind::MakeACareer => {
                if s.own_level > 0 {
                    dream.target_standing = level;
                }
                None
            }
            DreamKind::ReachTheTop => {
                let was = dream.target_standing;
                let reach = Self::reach_of(s);
                if reach > was {
                    if level > was {
                        dream.target_standing = reach;
                    }
                } else {
                    let refused = Self::AMBITION_REFUSES * s.ambition_drive();
                    dream.target_standing = was - (was - reach) * (1.0 - refused);
                }
                let moved = dream.target_standing - was;
                if moved >= Self::TARGET_TURN {
                    Some(ChapterKind::DreamRaised)
                } else if moved <= -Self::TARGET_TURN {
                    Some(ChapterKind::DreamLowered)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// The dream he holds now, if this one can no longer be held. A man
    /// living his needs a reason to stop believing, not a better offer.
    fn replaced_by(
        dream: &CareerDream,
        s: &MindSituation,
        club_id: u32,
        given_up: Option<(DreamKind, f32)>,
    ) -> Option<DreamKind> {
        let (best, best_score) = Self::strongest(s, club_id, given_up, |kind| kind != dream.kind);
        if dream.conviction < Self::LET_GO_BELOW {
            return Some(best);
        }
        if dream.arrived {
            return None;
        }
        let held = Self::score(dream.kind, s, club_id) * (0.5 + 0.5 * dream.conviction);
        (best_score > held + Self::SWITCH_MARGIN).then_some(best)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::mind::organs::MindOrgans;

    fn day(n: u16) -> EpochDay {
        10_000 + n
    }

    /// A regular, seen often enough to say so.
    fn regular(division: u16, club_standing: f32) -> MindSituation {
        MindSituation {
            starter_ratio: 0.9,
            appearances_tracked: 30,
            division_reputation: division,
            club_reputation: club_standing,
            own_level: 130,
            ceiling: 130,
            days_at_club: 400,
            ..MindSituation::neutral()
        }
    }

    fn a_year_of_weeks(anchors: &mut CareerAnchors, s: &MindSituation, club: u32, from: u16) {
        for week in 0..52 {
            anchors.record_summit(s, club, day(from + week * 7));
        }
    }

    #[test]
    fn starting_every_week_in_la_liga() {
        let mut anchors = CareerAnchors::default();
        anchors.record_summit(&regular(9300, 0.8), 7, day(0));
        let summit = anchors.summit.unwrap();
        assert_eq!(summit.league_reputation, 9300);
        assert_eq!(summit.club_id, 7);
        assert_eq!(summit.reached_on, day(0));
        assert_eq!(anchors.chapters.as_slice()[0].kind, ChapterKind::NewSummit);
    }

    #[test]
    fn a_squad_player_has_no_summit_there() {
        let mut anchors = CareerAnchors::default();
        anchors.record_summit(&regular(6000, 0.5), 3, day(0));

        let bench = MindSituation {
            starter_ratio: 0.1,
            ..regular(9500, 0.9)
        };
        a_year_of_weeks(&mut anchors, &bench, 7, 7);
        assert_eq!(anchors.summit.unwrap().league_reputation, 6000);
    }

    #[test]
    fn nothing_seen_is_no_summit_at_all() {
        let mut anchors = CareerAnchors::default();
        let unseen = MindSituation {
            appearances_tracked: 2,
            ..regular(9500, 0.9)
        };
        anchors.record_summit(&unseen, 7, day(0));
        assert!(anchors.summit.is_none());
    }

    #[test]
    fn a_loan_counts_for_what_it_was() {
        let mut anchors = CareerAnchors::default();
        let on_loan = MindSituation {
            is_on_loan: true,
            age: 20,
            ..regular(7000, 0.4)
        };
        anchors.record_summit(&on_loan, 12, day(0));
        assert_eq!(anchors.summit.unwrap().league_reputation, 7000);
    }

    #[test]
    fn a_fall_does_not_lower_it() {
        let mut anchors = CareerAnchors::default();
        anchors.record_summit(&regular(9000, 0.8), 7, day(0));
        a_year_of_weeks(&mut anchors, &regular(6000, 0.5), 3, 7);
        let summit = anchors.summit.unwrap();
        assert_eq!(summit.league_reputation, 9000);
        assert_eq!(summit.last_regular_on, day(0), "not at that level since");
        assert_eq!(
            summit.held(day(360), 27, 130),
            9000,
            "the first year in full"
        );
        assert!(
            summit.held(day(1000), 27, 130) < 9000,
            "and then it starts to go"
        );
    }

    #[test]
    fn a_regular_under_his_old_record_still_claims_the_level_he_plays_at() {
        let mut anchors = CareerAnchors::default();
        anchors.record_summit(&regular(9200, 0.8), 7, day(0));
        anchors.record_summit(&regular(8800, 0.8), 9, day(3 * 365));
        let later = day(3 * 365 + 120);

        let record = anchors.summit.unwrap();
        assert_eq!(record.league_reputation, 9200, "his record stands");
        assert!(record.held(later, 28, 130) < 8800, "and has begun to fade");

        let view = AnchorView::of(&anchors, later, 28, 130);
        assert_eq!(view.held_summit, 8800, "the level he last played at holds");
        assert_eq!(view.recency, 1.0);
    }

    #[test]
    fn it_fades() {
        let summit = CareerSummit {
            league_reputation: 9000,
            club_id: 7,
            club_standing: 0.8,
            reached_on: day(0),
            last_regular_on: day(0),
            level_then: 130,
        };
        assert_eq!(summit.held(day(5 * 365), 33, 130), 0);
        let (young, old) = (
            summit.held(day(2 * 365), 25, 130),
            summit.held(day(2 * 365), 34, 130),
        );
        assert!(
            old < young,
            "the older man fades faster: {old} against {young}"
        );
    }

    #[test]
    fn a_man_who_no_longer_looks_it_cannot_hold_it() {
        let summit = CareerSummit {
            league_reputation: 9000,
            club_id: 7,
            club_standing: 0.8,
            reached_on: day(0),
            last_regular_on: day(0),
            level_then: 130,
        };
        assert_eq!(summit.held(day(10), 27, 128), 9000);
        assert_eq!(summit.held(day(10), 27, 100), 0);
    }

    fn formed(s: &MindSituation) -> CareerDream {
        CareerDreamReview::form(s, 7, day(0))
    }

    #[test]
    fn an_ambitious_prospect_at_a_giant() {
        let dream = formed(&MindSituation {
            age: 19,
            ambition: 19.0,
            own_level: 130,
            ceiling: 185,
            club_reputation: 0.9,
            ..MindSituation::neutral()
        });
        assert_eq!(dream.kind, DreamKind::ReachTheTop);
        assert!(
            dream.target_standing >= 0.95,
            "target {}",
            dream.target_standing
        );
    }

    #[test]
    fn an_ambitious_boy_in_a_small_league() {
        let dream = formed(&MindSituation {
            age: 19,
            ambition: 19.0,
            own_level: 70,
            ceiling: 90,
            club_reputation: 0.2,
            ..MindSituation::neutral()
        });
        assert_eq!(dream.kind, DreamKind::ReachTheTop);
        assert!(
            dream.target_standing > 0.25 && dream.target_standing < 0.75,
            "target {}",
            dream.target_standing
        );
    }

    #[test]
    fn a_content_professional() {
        let dream = formed(&MindSituation {
            age: 26,
            ambition: 5.0,
            starter_ratio: 0.9,
            appearances_tracked: 30,
            own_level: 100,
            ceiling: 100,
            club_reputation: 0.55,
            days_at_club: 700,
            ..MindSituation::neutral()
        });
        assert_eq!(dream.kind, DreamKind::MakeACareer);
    }

    #[test]
    fn a_loyal_long_server() {
        let dream = formed(&MindSituation {
            age: 29,
            loyalty: 18.0,
            days_at_club: 8 * 365,
            starter_ratio: 0.9,
            appearances_tracked: 30,
            own_level: 110,
            ceiling: 110,
            club_reputation: 0.6,
            ..MindSituation::neutral()
        });
        assert_eq!(dream.kind, DreamKind::BelongHere);
        assert_eq!(dream.target_club, 7);
    }

    #[test]
    fn an_unsettled_man_late_in_his_career_abroad() {
        let dream = formed(&MindSituation {
            age: 31,
            is_abroad: true,
            adaptability: 5.0,
            own_level: 110,
            ceiling: 110,
            club_reputation: 0.6,
            days_at_club: 400,
            ..MindSituation::neutral()
        });
        assert_eq!(dream.kind, DreamKind::FinishAtHome);
    }

    #[test]
    fn a_man_who_left_the_club_he_came_from() {
        let dream = formed(&MindSituation {
            age: 30,
            loyalty: 17.0,
            heart_club: 44,
            own_level: 110,
            ceiling: 110,
            club_reputation: 0.6,
            days_at_club: 400,
            ..MindSituation::neutral()
        });
        assert_eq!(dream.kind, DreamKind::PlayForHisClub);
        assert_eq!(dream.target_club, 44);
    }

    #[test]
    fn the_same_player_forms_the_same_dream_twice() {
        for id in 0..500 {
            let s = MindSituation {
                player_id: id,
                age: 16 + (id % 20) as u8,
                ambition: (id % 21) as f32,
                loyalty: (id % 17) as f32,
                own_level: 90,
                ceiling: 110,
                club_reputation: 0.5,
                days_at_club: (id * 3 % 3000) as u16,
                ..MindSituation::neutral()
            };
            assert_eq!(formed(&s), formed(&s));
        }
    }

    fn reviewed(
        anchors: &mut CareerAnchors,
        s: &MindSituation,
        club: u32,
        improvement: f32,
        year: u16,
    ) {
        anchors.review_dream(s, club, improvement, day(year * 365));
    }

    #[test]
    fn nobody_came() {
        let s = |age: u8| MindSituation {
            age,
            ambition: 16.0,
            own_level: 110,
            ceiling: 110,
            club_reputation: 0.55,
            days_at_club: 500,
            starter_ratio: 0.9,
            appearances_tracked: 30,
            ..MindSituation::neutral()
        };
        let mut anchors = CareerAnchors::default();
        anchors.dream = Some(CareerDream {
            kind: DreamKind::ReachTheTop,
            target_standing: 0.9,
            conviction: 0.8,
            ..CareerDreamReview::formed_as(DreamKind::ReachTheTop, &s(27), 7, day(0))
        });

        let mut lowered = false;
        for year in 1..=8u16 {
            anchors.on_window_missed();
            anchors.on_window_missed();
            let age = 26 + year as u8;
            let before = anchors.dream.unwrap();
            reviewed(&mut anchors, &s(age), 7, 0.0, year);
            let after = anchors.dream.unwrap();
            if after.kind == DreamKind::ReachTheTop {
                lowered |= after.target_standing < before.target_standing;
                assert!(after.conviction <= before.conviction, "year {year}");
            } else {
                assert_eq!(after.kind, DreamKind::MakeACareer, "peace, at year {year}");
                assert!(lowered, "the target came down first");
                let let_go = anchors
                    .chapters
                    .iter()
                    .find(|c| c.kind == ChapterKind::DreamLetGo)
                    .unwrap();
                assert_eq!(let_go.on, day(year * 365));
                assert_eq!(let_go.dream, DreamKind::ReachTheTop);

                // He does not talk himself back into it the next summer.
                reviewed(&mut anchors, &s(age + 1), 7, 0.0, year + 1);
                assert_eq!(anchors.dream.unwrap().kind, DreamKind::MakeACareer);
                return;
            }
        }
        panic!("eight years and he still has not made his peace");
    }

    #[test]
    fn ambition_makes_him_slower_to_lower_it() {
        let after = |ambition: f32| {
            let s = MindSituation {
                ambition,
                own_level: 110,
                ceiling: 110,
                club_reputation: 0.55,
                ..MindSituation::neutral()
            };
            let mut dream = CareerDreamReview::formed_as(DreamKind::ReachTheTop, &s, 7, day(0));
            dream.target_standing = 0.95;
            CareerDreamReview::revise_target(&mut dream, &s);
            dream.target_standing
        };
        assert!(after(19.0) > after(10.0));
    }

    #[test]
    fn the_late_bloomer() {
        let s = MindSituation {
            age: 24,
            ambition: 10.0,
            own_level: 130,
            ceiling: 140,
            club_reputation: 0.45,
            days_at_club: 1000,
            starter_ratio: 0.9,
            appearances_tracked: 30,
            ..MindSituation::neutral()
        };
        let mut anchors = CareerAnchors::default();
        anchors.dream = Some(CareerDreamReview::formed_as(
            DreamKind::MakeACareer,
            &s,
            7,
            day(0),
        ));
        reviewed(&mut anchors, &s, 7, 0.5, 1);
        let dream = anchors.dream.unwrap();
        assert_eq!(dream.kind, DreamKind::ReachTheTop);
        assert!(dream.target_standing > 0.45, "and it is above his club");
    }

    #[test]
    fn a_loan_is_no_time_to_think_again() {
        let at_his_club = MindSituation {
            age: 31,
            loyalty: 18.0,
            own_level: 110,
            ceiling: 110,
            club_reputation: 0.7,
            days_at_club: 2000,
            starter_ratio: 0.9,
            appearances_tracked: 30,
            ..MindSituation::neutral()
        };
        let mut anchors = CareerAnchors::default();
        anchors.form_dream(&at_his_club, 7, day(0));
        let held = anchors.dream.unwrap();
        assert_eq!(held.kind, DreamKind::BelongHere);

        let lent_out = MindSituation {
            is_on_loan: true,
            days_at_club: 100,
            ..at_his_club
        };
        anchors.review_dream(&lent_out, 9, 0.0, day(2 * 365));
        assert_eq!(anchors.dream, Some(held), "untouched, not even dated");
    }

    #[test]
    fn a_move_does_not_change_it() {
        let at_home = MindSituation {
            age: 24,
            ambition: 18.0,
            own_level: 130,
            ceiling: 150,
            club_reputation: 0.7,
            ..MindSituation::neutral()
        };
        let mut anchors = CareerAnchors::default();
        anchors.form_dream(&at_home, 7, day(0));
        let dream = anchors.dream.unwrap();

        let sold = MindSituation {
            club_reputation: 0.45,
            ..at_home
        };
        // A review is a season away: every week until then leaves it alone.
        for week in 1..52 {
            anchors.review_dream(&sold, 9, 0.0, day(week * 7));
        }
        assert_eq!(anchors.dream.unwrap(), dream);
    }

    fn at_the_top(dream_kind: DreamKind, target: f32) -> CareerAnchors {
        let s = regular(9300, 0.5);
        let mut anchors = CareerAnchors::default();
        anchors.dream = Some(CareerDream {
            target_standing: target,
            target_club: 7,
            ..CareerDreamReview::formed_as(dream_kind, &s, 7, day(0))
        });
        anchors
    }

    #[test]
    fn a_star_at_real_madrid() {
        let mut anchors = at_the_top(DreamKind::ReachTheTop, 1.0);
        anchors.read_arrival(&regular(9300, 0.96), 7, day(7));
        let dream = anchors.dream.unwrap();
        assert!(dream.arrived);
        assert_eq!(dream.arrived_on, day(7));
        assert_eq!(dream.state(), DreamState::Arrived);
    }

    #[test]
    fn a_big_fish_in_his_own_pond() {
        let mut anchors = at_the_top(DreamKind::ReachTheTop, 0.4);
        anchors.read_arrival(&regular(3500, 0.42), 7, day(7));
        assert!(anchors.dream.unwrap().arrived);
    }

    #[test]
    fn nobody_arrives_from_the_bench() {
        let mut anchors = at_the_top(DreamKind::ReachTheTop, 0.8);
        let bench = MindSituation {
            starter_ratio: 0.1,
            ..regular(9300, 0.96)
        };
        anchors.read_arrival(&bench, 7, day(7));
        assert!(!anchors.dream.unwrap().arrived);
    }

    #[test]
    fn he_loses_his_place() {
        let mut anchors = at_the_top(DreamKind::ReachTheTop, 0.8);
        anchors.read_arrival(&regular(9300, 0.96), 7, day(7));
        let bench = MindSituation {
            starter_ratio: 0.1,
            ..regular(9300, 0.96)
        };
        anchors.read_arrival(&bench, 7, day(100));
        assert!(
            anchors.dream.unwrap().arrived,
            "a bad few months is not a season"
        );
        anchors.read_arrival(&bench, 7, day(300));
        let dream = anchors.dream.unwrap();
        assert!(!dream.arrived);
        assert_eq!(dream.state(), DreamState::WantsItBack);
        assert_eq!(
            anchors.chapters.iter().last().unwrap().kind,
            ChapterKind::LostIt
        );
    }

    #[test]
    fn leaving_loses_it_at_once_and_coming_back_regains_it() {
        let mut anchors = at_the_top(DreamKind::PlayForHisClub, 0.0);
        anchors.read_arrival(&regular(9300, 0.6), 7, day(7));
        assert!(anchors.dream.unwrap().arrived);
        anchors.read_arrival(&regular(9300, 0.6), 9, day(14));
        assert!(!anchors.dream.unwrap().arrived);
        anchors.read_arrival(&regular(9300, 0.6), 7, day(21));
        let kinds: Vec<ChapterKind> = anchors.chapters.iter().map(|c| c.kind).collect();
        assert_eq!(
            kinds,
            vec![
                ChapterKind::Arrived,
                ChapterKind::LostIt,
                ChapterKind::Regained
            ]
        );
        assert_eq!(anchors.dream.unwrap().arrived_on, day(7), "recorded once");
    }

    #[test]
    fn a_chapter_has_a_date() {
        let mut anchors = at_the_top(DreamKind::ReachTheTop, 0.8);
        anchors.read_arrival(&regular(9300, 0.96), 7, day(7));
        let chapter = anchors.chapters.iter().last().unwrap();
        assert_eq!(chapter.kind, ChapterKind::Arrived);
        assert_eq!(chapter.on, day(7));
        assert_eq!(chapter.club_id, 7);
        assert_eq!(chapter.league_reputation, 9300);
    }

    #[test]
    fn a_long_career_does_not_lose_its_best_moments() {
        let mut anchors = at_the_top(DreamKind::ReachTheTop, 0.8);
        anchors.record_summit(&regular(9000, 0.7), 3, day(1));
        anchors.read_arrival(&regular(9300, 0.96), 7, day(2));
        anchors.tell(CareerChapter {
            kind: ChapterKind::DreamLowered,
            on: day(3),
            ..Default::default()
        });
        for n in 0..40 {
            for kind in [
                ChapterKind::SteppedUp,
                ChapterKind::Fell,
                ChapterKind::Homecoming,
            ] {
                anchors.tell(CareerChapter {
                    kind,
                    on: day(10 + n * 40),
                    ..Default::default()
                });
            }
        }
        assert!(anchors.chapters.is_full());
        let holds = |kind: ChapterKind| anchors.chapters.iter().any(|c| c.kind == kind);
        assert!(holds(ChapterKind::NewSummit));
        assert!(holds(ChapterKind::Arrived));
        assert!(holds(ChapterKind::DreamLowered));
    }

    #[test]
    fn a_new_summit_outlasts_an_old_one_when_the_list_is_full() {
        let mut anchors = CareerAnchors::default();
        for n in 0..30u16 {
            anchors.tell(CareerChapter {
                kind: ChapterKind::NewSummit,
                on: day(n),
                league_reputation: 3000 + n,
                ..Default::default()
            });
        }
        assert!(
            anchors.chapters.iter().any(|c| c.league_reputation == 3029),
            "the highest he reached is the one he keeps"
        );
    }

    #[test]
    fn a_real_step_up_is_read_and_a_loan_return_is_not() {
        let organs = MindOrgans::default();
        let mut anchors = CareerAnchors::default();
        anchors.read_the_move(&regular(6000, 0.5), 3, &organs, day(0));
        let up = anchors.read_the_move(&regular(8000, 0.7), 7, &organs, day(7));
        assert!(up.stepped_up);

        let on_loan = MindSituation {
            is_on_loan: true,
            ..regular(5000, 0.4)
        };
        let down = anchors.read_the_move(&on_loan, 9, &organs, day(14));
        assert!(down.fell, "a regular going down, on loan or not");

        let back = anchors.read_the_move(&regular(8000, 0.7), 7, &organs, day(21));
        assert!(!back.stepped_up, "coming home from a loan is not a step up");
    }

    #[test]
    fn a_move_is_read_on_the_club_and_the_division_together() {
        let organs = MindOrgans::default();
        let mut anchors = CareerAnchors::default();
        anchors.read_the_move(&regular(8775, 0.60), 3, &organs, day(0));
        let to_paris = anchors.read_the_move(&regular(8241, 0.93), 7, &organs, day(7));
        assert!(to_paris.stepped_up && !to_paris.fell);

        let mut anchors = CareerAnchors::default();
        anchors.read_the_move(&regular(9284, 0.62), 3, &organs, day(0));
        let to_montevideo = anchors.read_the_move(&regular(6047, 0.77), 7, &organs, day(7));
        assert!(to_montevideo.fell && !to_montevideo.stepped_up);
    }

    #[test]
    fn a_squad_player_going_down_is_not_a_fall() {
        let organs = MindOrgans::default();
        let mut anchors = CareerAnchors::default();
        let bench = MindSituation {
            starter_ratio: 0.1,
            ..regular(9000, 0.8)
        };
        anchors.read_the_move(&bench, 3, &organs, day(0));
        let down = anchors.read_the_move(&regular(6000, 0.5), 7, &organs, day(7));
        assert!(!down.fell);
    }

    #[test]
    fn coming_home_is_a_chapter() {
        let organs = MindOrgans::default();
        let mut anchors = CareerAnchors::default();
        let abroad = MindSituation {
            is_abroad: true,
            ..regular(7000, 0.6)
        };
        anchors.read_the_move(&abroad, 3, &organs, day(0));
        anchors.read_the_move(&regular(6000, 0.5), 7, &organs, day(7));
        assert!(
            anchors
                .chapters
                .iter()
                .any(|c| c.kind == ChapterKind::Homecoming)
        );
    }

    #[test]
    fn every_chapter_kind_is_named() {
        let mut keys: Vec<&str> = ChapterKind::ALL.iter().map(|k| k.as_i18n_key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), ChapterKind::ALL.len());
    }
}
