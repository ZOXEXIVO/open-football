//! The competitive mind — his belief in himself as a player, and his
//! standing in the side.
//!
//! The faculty that thinks in matches. It owns self-belief, the run of
//! form, the drought, and the wants that are about the shirt: winning a
//! place back, getting first-team football, ending a barren spell.
//!
//! This is also the seam to the match engine. `PsychologyState`
//! (`match/engine/psychology/`) is correctly separate — it models one
//! afternoon — but it should be *seeded* by a player who has been
//! dropped four times running rather than starting neutral every week.
//! [`CompetitiveMind::self_belief`] is that seed.

use super::organs::MindOrgans;
use super::organs::goals::{GoalBlocker, GoalDomain, GoalEvidence, GoalKind, GoalOrigin};
use super::organs::memory::{EpisodeKind, EpochDay, MindEpisode};
use super::situation::MindSituation;
use super::submind::{MindOption, MindView, MoodContribution, ReasonSet, SubMind};
use crate::club::player::player::Player;
use crate::club::player::statistics::PlayerStatisticsHistoryItem;
use crate::PlayerSquadStatus;
use crate::r#match::engine::teamplay::standard::MatchStandard;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// His belief in himself as a player.
#[derive(Debug, Clone, Copy)]
pub struct CompetitiveMind {
    /// −100..=100. Built from what actually happens on the pitch.
    belief_pct: i8,
    /// The standard of football he is used to, on the scale the match
    /// reads from both sides at kickoff. Learnt only by playing; how the
    /// football went is left to `belief_pct`.
    assurance: f32,
    /// Fade taken off `assurance` since his last match, capped so a long
    /// absence never turns a veteran back into a debutant.
    absent_fade: f32,
    played_this_week: bool,
    /// His assurance has been set from something he lived — a recorded
    /// career or the squad he joined — rather than left at the neutral.
    seeded: bool,
    /// How far the evidence of his latest match moves his belief: the
    /// step he took up to play it. Staged by `on_match_played` before that
    /// match's episodes are filed.
    match_steadiness: f32,
    /// Consecutive matches started, or (negative) consecutive matches
    /// watched. The run, not the season total — a man who has started
    /// the last six feels different from one who started six in August.
    pub run: i8,
    /// Big matches he rose to, minus the ones he shrank in.
    pub big_match_record: i8,
    /// Weeks since anything went right. Saturates.
    pub barren_weeks: u8,
    /// How many of his injuries he has already filed away.
    ///
    /// Compared against `PlayerAttributes::injury_count` so the daily
    /// injury pass can spot a *new* one without a second severity model
    /// of its own — the count is incremented by `set_injury` at every
    /// site that can injure a man, which is the gate already trusted.
    pub injuries_seen: u8,
}

impl Default for CompetitiveMind {
    fn default() -> Self {
        CompetitiveMind {
            belief_pct: 0,
            assurance: Self::NEUTRAL_ASSURANCE,
            absent_fade: 0.0,
            played_this_week: false,
            seeded: false,
            match_steadiness: 1.0,
            run: 0,
            big_match_record: 0,
            barren_weeks: 0,
            injuries_seen: 0,
        }
    }
}

/// The football one match gave him, as the competitive mind reads it.
#[derive(Debug, Clone, Copy)]
pub struct CompetitiveMatchRead {
    pub standard: f32,
    pub minutes: f32,
    pub competitive: bool,
    /// A keeper who started and finished a competitive match without
    /// conceding.
    pub kept_clean_sheet: bool,
}

/// The state of mind a player brings to kickoff — never his statistics.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct KickoffMind {
    /// `None` for a player with no football behind him to step up from —
    /// a synthetic squad — who is at home at any standard.
    pub assurance: Option<f32>,
    pub self_belief: f32,
    /// 0..100, as `PlayerHappiness::morale`.
    pub morale: f32,
    pub big_match_record: i8,
}

impl KickoffMind {
    /// At home whatever the standard, believing nothing either way.
    pub fn neutral() -> Self {
        KickoffMind {
            assurance: None,
            self_belief: 0.0,
            morale: 50.0,
            big_match_record: 0,
        }
    }
}

impl Player {
    /// Seed his assurance by replaying his recorded career, each season at
    /// the standard `standard_of` reads for the club he played it for.
    /// False when he has no recorded season.
    pub fn seed_assurance_from_career(
        &mut self,
        standard_of: impl Fn(&PlayerStatisticsHistoryItem) -> f32,
    ) -> bool {
        let mut seasons: Vec<&PlayerStatisticsHistoryItem> =
            self.statistics_history.items.iter().collect();
        let Some(first) = seasons.iter().min_by_key(|i| (i.season.start_year, i.seq_id)) else {
            return false;
        };
        let start = standard_of(first) - CompetitiveMind::ROOKIE_GAP;
        seasons.sort_by_key(|i| (i.season.start_year, i.seq_id));
        let pace = CompetitiveMind::settling_pace(
            self.skills.mental.composure,
            self.attributes.adaptability,
        );
        let mind = &mut self.mind.competitive;
        mind.seed_assurance(start);
        for season in seasons {
            mind.replay_season(
                standard_of(season),
                season.statistics.played,
                season.statistics.played_subs,
                pace,
            );
        }
        true
    }

    /// Seed his assurance from his place in the side he plays for, when
    /// nothing of his career is recorded.
    pub fn seed_assurance_from_role(&mut self, team_standard: f32) {
        let gap = match self.contract.as_ref().map(|c| &c.squad_status) {
            Some(PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular) => 0.0,
            Some(
                PlayerSquadStatus::FirstTeamSquadRotation | PlayerSquadStatus::MainBackupPlayer,
            ) => CompetitiveMind::ROLE_GAP,
            _ => CompetitiveMind::ROLE_GAP * 2.0,
        };
        self.mind.competitive.seed_assurance(team_standard - gap);
    }

    /// The state of mind he brings to kickoff on `now`. A synthetic squad
    /// with no calendar, and every player under `OF_MIND_OFF`, arrives
    /// neutral.
    pub fn kickoff_mind(&self, now: Option<NaiveDate>) -> KickoffMind {
        if now.is_none() || !MindSwitch::armed() {
            return KickoffMind::neutral();
        }
        let c = &self.mind.competitive;
        KickoffMind {
            assurance: Some(c.assurance()),
            self_belief: c.self_belief(),
            morale: self.happiness.morale,
            big_match_record: c.big_match_record,
        }
    }
}

/// `OF_MIND_OFF=1` turns the whole assurance-and-nerves mechanism off:
/// neutral kickoff minds, no seeding, no keeper lapses and the two-keeper
/// standard. Read once per process. The A/B control every calibration of
/// the mechanism is measured against.
pub struct MindSwitch;

impl MindSwitch {
    pub fn armed() -> bool {
        use std::sync::OnceLock;
        static ARMED: OnceLock<bool> = OnceLock::new();
        *ARMED.get_or_init(|| {
            std::env::var("OF_MIND_OFF")
                .map(|v| v.is_empty() || v == "0")
                .unwrap_or(true)
        })
    }
}

impl CompetitiveMind {
    /// A player nobody has told us about is at home in the division every
    /// engine constant was fitted in.
    pub const NEUTRAL_ASSURANCE: f32 = MatchStandard::CALIBRATION;

    /// Share of the gap to a match's standard one full competitive match
    /// closes for a median temperament: about 70% over twelve matches.
    const RISE: f32 = 0.095;
    /// Playing below his level unlearns it far more slowly than playing
    /// above it teaches.
    const FALL_SHARE: f32 = 0.2;
    const FRIENDLY_WEIGHT: f32 = 0.5;
    const FADE_PER_WEEK: f32 = 0.002;
    const FADE_CAP: f32 = 0.015;
    /// A step of this size or more leaves his belief as exposed as it ever
    /// is; at or below his assurance a single match moves it by
    /// `STEADY_FLOOR` of that.
    const STEP_SCALE: f32 = 0.10;
    const STEADY_FLOOR: f32 = 0.5;
    const CLEAN_SHEET_SHARE: f32 = 0.25;
    /// Minutes a recorded substitute appearance stands for.
    const SEEDED_SUB_MINUTES: f32 = 25.0;
    /// Where a career replay starts: below the first club he is recorded
    /// at, so a youngster who never got on is not assured at his parent
    /// club's standard by having been on its books.
    const ROOKIE_GAP: f32 = 0.15;
    /// How far below his side's standard a squad player and a youngster
    /// sit when nothing of their career is recorded.
    const ROLE_GAP: f32 = 0.03;

    #[inline]
    pub fn assurance(&self) -> f32 {
        self.assurance
    }

    /// Place his assurance directly — world-start seeding and new players
    /// only. Everything after that goes through `on_match_played`.
    pub fn seed_assurance(&mut self, standard: f32) {
        self.assurance = standard.clamp(0.0, 1.0);
        self.absent_fade = 0.0;
        self.seeded = true;
    }

    #[inline]
    pub fn is_seeded(&self) -> bool {
        self.seeded
    }

    /// A recorded season, learnt as the matches it was made of.
    pub fn replay_season(&mut self, standard: f32, starts: u16, sub_apps: u16, pace: f32) {
        let full = CompetitiveMatchRead {
            standard,
            minutes: 90.0,
            competitive: true,
            kept_clean_sheet: false,
        };
        let cameo = CompetitiveMatchRead {
            minutes: Self::SEEDED_SUB_MINUTES,
            ..full
        };
        for _ in 0..starts {
            self.absorb(&full, pace);
        }
        for _ in 0..sub_apps {
            self.absorb(&cameo, pace);
        }
        self.played_this_week = false;
    }

    /// How quickly he settles at a new standard: composure and
    /// adaptability on the 1..20 scale, about 1.0 for a median man.
    pub fn settling_pace(composure: f32, adaptability: f32) -> f32 {
        let t = (((composure + adaptability) * 0.5 - 1.0) / 19.0).clamp(0.0, 1.0);
        0.6 + 0.8 * t
    }

    /// 1.0 for a step far above him, `STEADY_FLOOR` at his own level.
    fn steadiness_at(&self, standard: f32) -> f32 {
        let step = ((standard - self.assurance).max(0.0) / Self::STEP_SCALE).clamp(0.0, 1.0);
        Self::STEADY_FLOOR + (1.0 - Self::STEADY_FLOOR) * step
    }

    /// The match is in: stage how exposed his belief was to it, take the
    /// clean sheet, then learn the standard he played at.
    pub fn on_match_played(&mut self, read: &CompetitiveMatchRead, pace: f32) {
        self.match_steadiness = self.steadiness_at(read.standard);
        if read.kept_clean_sheet && read.competitive {
            self.shift(Self::LIFT * Self::CLEAN_SHEET_SHARE * self.match_steadiness);
        }
        self.absorb(read, pace);
    }

    fn absorb(&mut self, read: &CompetitiveMatchRead, pace: f32) {
        let weight = (read.minutes / 90.0).clamp(0.0, 1.5)
            * if read.competitive {
                1.0
            } else {
                Self::FRIENDLY_WEIGHT
            };
        if weight <= 0.0 {
            return;
        }
        let gap = read.standard - self.assurance;
        let rate = Self::RISE * pace * if gap > 0.0 { 1.0 } else { Self::FALL_SHARE };
        self.assurance = (self.assurance + gap * (rate * weight).min(1.0)).clamp(0.0, 1.0);
        self.absent_fade = 0.0;
        self.played_this_week = true;
    }

    /// The weekly think's half of assurance: a week without football fades
    /// it, up to `FADE_CAP` since his last match.
    fn fade_if_idle(&mut self) {
        if !self.played_this_week && self.absent_fade < Self::FADE_CAP {
            let step = Self::FADE_PER_WEEK.min(Self::FADE_CAP - self.absent_fade);
            self.assurance = (self.assurance - step).max(0.0);
            self.absent_fade += step;
        }
        self.played_this_week = false;
    }

    /// What the match evidence of his latest match is worth to his belief.
    /// Selection, injury and press episodes are not match evidence and
    /// always land whole.
    fn evidence_weight(&self, kind: EpisodeKind) -> f32 {
        match kind {
            EpisodeKind::DecisiveGoal
            | EpisodeKind::ManOfTheMatch
            | EpisodeKind::CostlyError
            | EpisodeKind::MissedDecisivePenalty
            | EpisodeKind::PenaltySaved
            | EpisodeKind::MatchSavingDisplay
            | EpisodeKind::SentOff
            | EpisodeKind::HeavyDefeat
            | EpisodeKind::DerbyWin
            | EpisodeKind::DerbyDefeat => self.match_steadiness,
            _ => 1.0,
        }
    }

    /// How far one event moves self-belief, as a fraction of the gap to
    /// the extreme. Asymmetric on purpose: a mistake costs more belief
    /// than a good game buys, which is both true and what makes a slump
    /// something a player has to climb out of.
    pub const LIFT: f32 = 0.12;
    pub const KNOCK: f32 = 0.20;

    /// Consecutive matches out of the side after which he stops assuming
    /// it is temporary.
    pub const DROPPED_RUN: i8 = -4;

    #[inline]
    pub fn self_belief(&self) -> f32 {
        self.belief_pct as f32 / 100.0
    }

    fn shift(&mut self, delta: f32) {
        let belief = self.self_belief();
        let gap = if delta > 0.0 {
            1.0 - belief
        } else {
            belief + 1.0
        };
        let next = (belief + delta * gap).clamp(-1.0, 1.0);
        self.belief_pct = (next * 100.0).round() as i8;
    }

    fn extend_run(&mut self, started: bool) {
        self.run = if started {
            self.run.max(0).saturating_add(1)
        } else {
            self.run.min(0).saturating_sub(1)
        };
    }

    /// Is he out of the side long enough to have concluded something
    /// about it?
    #[inline]
    pub fn is_out_of_the_side(&self) -> bool {
        self.run <= Self::DROPPED_RUN
    }

    /// Not behind somebody — excluded.
    ///
    /// The tell is the combination: a long run of watching *and* being
    /// ranked below men he is plainly better than. One of those on its
    /// own is a bad spell or a strong squad; both together is a manager
    /// who has decided, and a player who can feel it.
    fn is_frozen_out(&self, s: &MindSituation) -> bool {
        self.run <= Self::DROPPED_RUN * 2
            && s.has_squad_view()
            && s.pecking_rank as u16 > (s.rivals_at_position as u16).div_ceil(2)
            && s.rival_gap > 0
    }

    /// The man in possession, hearing footsteps.
    ///
    /// The mirror of losing a place, and the more interesting half: what
    /// he does about it is decided by character rather than by form. A
    /// professional trains harder and takes the boy under his wing; a
    /// careless one just hangs on. Both are held as the same want — the
    /// fork lives in [`super::social::SocialMind`], which owns what
    /// happens between the two men.
    fn guard_the_shirt(&mut self, s: &MindSituation, organs: &mut MindOrgans, today: EpochDay) {
        if !s.is_first_choice() || s.top_rival.is_none() {
            return;
        }
        // Only when the man behind him is genuinely close, and young
        // enough that time is on his side rather than his own.
        let closing = (10 - s.rival_gap.clamp(0, 10)) as f32 / 10.0;
        let younger = s.top_rival_age > 0 && (s.top_rival_age as i16) < s.age as i16 - 3;
        if !younger || closing < 0.3 {
            return;
        }
        // How much it presses is how much of his own career is behind
        // him. A twenty-six-year-old barely notices; a thirty-three-
        // year-old thinks about nothing else.
        let threat = closing * s.career_spent().max(0.2);
        organs.goals.pursue(
            GoalKind::HoldOntoMyPlace,
            GoalOrigin::Circumstance,
            GoalEvidence::of(&[GoalEvidence::A_YOUNGSTER_IS_COMING]),
            threat,
            today,
        );
    }
}

impl SubMind for CompetitiveMind {
    fn domain(&self) -> GoalDomain {
        GoalDomain::Competitive
    }

    fn observe(&mut self, episode: &MindEpisode, _organs: &mut MindOrgans) {
        let w = self.evidence_weight(episode.kind);
        match episode.kind {
            EpisodeKind::DecisiveGoal
            | EpisodeKind::ManOfTheMatch
            | EpisodeKind::PenaltySaved
            | EpisodeKind::MatchSavingDisplay => {
                self.shift(Self::LIFT * w);
                self.barren_weeks = 0;
            }
            EpisodeKind::StartedBigMatch | EpisodeKind::DerbyWin => {
                self.shift(Self::LIFT * w);
                self.big_match_record = self.big_match_record.saturating_add(1);
                self.extend_run(true);
                self.barren_weeks = 0;
            }
            EpisodeKind::WonStartingPlace => {
                self.shift(Self::LIFT);
                self.extend_run(true);
            }
            EpisodeKind::CostlyError | EpisodeKind::MissedDecisivePenalty => {
                self.shift(-Self::KNOCK * w);
            }
            EpisodeKind::LeftOutOfBigMatch => {
                self.shift(-Self::KNOCK);
                self.big_match_record = self.big_match_record.saturating_sub(1);
                self.extend_run(false);
            }
            EpisodeKind::DroppedToBench | EpisodeKind::LostStartingPlace => {
                self.shift(-Self::KNOCK);
                self.extend_run(false);
            }
            EpisodeKind::SentOff | EpisodeKind::DerbyDefeat | EpisodeKind::HeavyDefeat => {
                self.shift(-Self::LIFT * w);
            }
            // The body speaks to this faculty too — a long lay-off is a
            // blow to a player's belief in himself, not just his fitness.
            EpisodeKind::CareerThreateningInjury => self.shift(-Self::KNOCK * 1.5),
            EpisodeKind::SeriousInjury => self.shift(-Self::KNOCK),
            EpisodeKind::ReturnedFromLongInjury => self.shift(Self::LIFT),
            _ => {}
        }
    }

    fn reflect(&mut self, view: &MindView<'_>, organs: &mut MindOrgans) {
        let s = view.situation;
        let today = view.today();

        self.fade_if_idle();

        // A goal drought, for the players it means anything to.
        if s.apps_since_goal >= 8 {
            self.barren_weeks = self.barren_weeks.saturating_add(1);
            organs.goals.pursue(
                GoalKind::EndTheDrought,
                GoalOrigin::SelfDrive,
                GoalEvidence::EMPTY,
                (s.apps_since_goal as f32 / 20.0).clamp(0.0, 1.0),
                today,
            );
        }

        // Playing less than the role he was given implies. The gap is
        // measured against what he was told, so identical minutes are a
        // grievance for a key player and nothing at all for a backup.
        let gap = s.playing_time_gap();

        // The run, kept honest against the rolling share.
        //
        // Episodes move it match to match, but ordinary rotation files
        // none — being left out of one league game is not a memory, and
        // tapping it would fill a 32-slot store with Tuesdays. So the
        // weekly think, which is roughly one matchday, nudges it from
        // where the minutes actually are. Without this a player nobody
        // ever drops for a *big* match reads as though he were still in
        // the side after a season on the bench, and both
        // `is_out_of_the_side` and `is_frozen_out` are unreachable.
        if s.starter_ratio < 0.15 {
            self.extend_run(false);
        } else if s.starter_ratio > 0.60 {
            self.extend_run(true);
        }

        // Being frozen out is re-read every week, not remembered. The
        // moment he is back in the manager's thinking the want is his
        // to pursue again — and `weigh` stops reading it as the reason
        // to go. The freeze itself is asserted further down, while it
        // holds.
        if !self.is_frozen_out(s) {
            organs.goals.unblock(GoalBlocker::FrozenOut);
        }

        if gap >= 0.0 {
            // He is playing. Whatever he wanted about his place is
            // answered, gradually.
            organs.goals.advance(GoalKind::WinBackMyPlace, 0.15);
            organs.goals.advance(GoalKind::PlayFirstTeamFootball, 0.10);
            organs.goals.advance(GoalKind::HoldOntoMyPlace, 0.10);
            self.guard_the_shirt(s, organs, today);
            return;
        }

        // A settling-in period is not a grievance yet.
        if !s.is_settled() {
            return;
        }

        let shortfall = (-gap).clamp(0.0, 1.0);
        let mut evidence = GoalEvidence::of(&[GoalEvidence::LOST_HIS_PLACE]);
        if self.is_out_of_the_side() {
            evidence.insert(GoalEvidence::NO_FIRST_TEAM_FOOTBALL);
        }
        if s.career_spent() > 0.5 {
            evidence.insert(GoalEvidence::PRIME_YEARS_PASSING);
        }

        // Being kept out by a man he does not rate above himself is a
        // different complaint from simply not playing, and it is the one
        // that makes a peak-age professional impossible to placate.
        let unfairly = s.blocked_unfairly();
        if unfairly > 0.15 {
            evidence.insert(GoalEvidence::BLOCKED_BY_A_PEER);
        }

        // Frozen out is not benched. There is no competition to win,
        // because he is not in it — so however hard he trains, fighting
        // for his place is not a thing he can do. The goal is held and
        // blocked rather than pursued, which keeps it colouring his mood
        // while routing the escalation somewhere it can actually go.
        if self.is_frozen_out(s) {
            evidence.insert(GoalEvidence::FROZEN_OUT);
            organs.goals.pursue(
                GoalKind::WinBackMyPlace,
                GoalOrigin::Grievance,
                evidence,
                shortfall,
                today,
            );
            organs
                .goals
                .block(GoalKind::WinBackMyPlace, GoalBlocker::FrozenOut);
            organs.goals.pursue(
                GoalKind::BeAllowedToLeave,
                GoalOrigin::Grievance,
                evidence,
                shortfall.max(0.4),
                today,
            );
            return;
        }

        // First he means to win it back. That is what a player does —
        // and how hard he means it is the clearest thing professionalism
        // decides about a footballer. A diligent man who has lost his
        // place trains; a careless one sulks and waits for the club to
        // solve it for him.
        organs.goals.pursue(
            GoalKind::WinBackMyPlace,
            GoalOrigin::SelfDrive,
            evidence,
            shortfall * (0.55 + 0.75 * s.diligence()),
            today,
        );

        // A good young player behind an ageing one is not in trouble; he
        // can see the shirt coming to him. Nothing else about his week
        // says so, which is why the flat minutes number gets him wrong.
        if s.can_wait_for_the_shirt() {
            return;
        }

        // Only once he has stopped believing he will — a long run out of
        // the side, and a career burning down — does it become wanting to
        // be somewhere he would play. This is the one want in the whole
        // catalog that points at a *smaller* club, and it must be earned
        // rather than assumed.
        let resigned = self.is_out_of_the_side() && self.self_belief() < 0.0;
        // …or he is plainly the better player and is still not being
        // picked, which gets a man there far quicker than losing a fair
        // fight ever does.
        let wronged = unfairly > 0.45;

        if resigned || wronged {
            let resignation = shortfall * s.career_spent().max(0.3) + unfairly * 0.4;
            organs.goals.pursue(
                GoalKind::PlayFirstTeamFootball,
                GoalOrigin::Survival,
                evidence,
                resignation.clamp(0.0, 1.0),
                today,
            );
            organs.goals.set_urgency(
                GoalKind::PlayFirstTeamFootball,
                // A tournament on the horizon does what nothing else in
                // the model does: it puts a hard date on needing to play.
                s.career_spent().max(s.tournament_pressure()),
            );
        }
    }

    fn weigh(&self, option: MindOption, organs: &MindOrgans) -> ReasonSet {
        self.weigh_option(option, organs)
    }

    fn appraise(&self, _organs: &MindOrgans) -> MoodContribution {
        // Self-belief is the axis; the run gives the confidence, because
        // a player who has not been near the side has less to go on than
        // one in the middle of a run either way.
        let evidence = (self.run.unsigned_abs() as f32 / 6.0).clamp(0.0, 1.0);
        MoodContribution::new(
            GoalDomain::Competitive,
            self.self_belief() * 6.0,
            evidence.max(0.3),
        )
    }
}

impl CompetitiveMind {
    /// What the shirt says about a decision. The loudest voice a player
    /// has, and the one that decides most moves.
    pub(super) fn weigh_option(&self, option: MindOption, organs: &MindOrgans) -> ReasonSet {
        let mut reasons = ReasonSet::new();

        match option {
            MindOption::StayAndFight => {
                let win_back = organs.goals.pressure_of(GoalKind::WinBackMyPlace);
                if win_back > 0.1 {
                    // Blocked, it argues the other way: a want he cannot
                    // act on is not a reason to stay, it is the reason he
                    // is going.
                    let blocked = organs
                        .goals
                        .get(GoalKind::WinBackMyPlace)
                        .map(|g| g.blocked_by.is_blocked())
                        .unwrap_or(false);
                    reasons.push(
                        GoalKind::WinBackMyPlace,
                        if blocked { -win_back } else { win_back },
                    );
                }
                if self.is_out_of_the_side() && self.self_belief() < 0.0 {
                    reasons.push(GoalKind::PlayFirstTeamFootball, self.self_belief());
                }
                let holding_on = organs.goals.pressure_of(GoalKind::HoldOntoMyPlace);
                if holding_on > 0.1 {
                    reasons.push(GoalKind::HoldOntoMyPlace, holding_on);
                }
            }

            MindOption::RequestTransfer | MindOption::AcceptLoan(_) => {
                let needs_games = organs.goals.pressure_of(GoalKind::PlayFirstTeamFootball);
                if needs_games > 0.1 {
                    reasons.push(GoalKind::PlayFirstTeamFootball, needs_games);
                }
                let national = organs.goals.pressure_of(GoalKind::GetIntoTheNationalSquad);
                if national > 0.3 {
                    // A tournament is coming and he is not playing. This
                    // is the argument that empties benches every January
                    // of a World Cup year.
                    reasons.push(GoalKind::GetIntoTheNationalSquad, national);
                }
            }

            MindOption::JoinClub(_) => {
                let needs_games = organs.goals.pressure_of(GoalKind::PlayFirstTeamFootball);
                if needs_games > 0.1 {
                    reasons.push(GoalKind::PlayFirstTeamFootball, needs_games);
                }
                if self.self_belief() > 0.4 {
                    // A man in form backs himself anywhere.
                    reasons.push(GoalKind::StepUpToABiggerClub, self.self_belief() * 0.5);
                }
            }

            MindOption::SignContract => {
                // Signing on for more years of what he has now. Wanting to
                // be somewhere he plays is the argument against, and the
                // only one this faculty has.
                let needs_games = organs.goals.pressure_of(GoalKind::PlayFirstTeamFootball);
                if needs_games > 0.1 {
                    reasons.push(GoalKind::PlayFirstTeamFootball, -needs_games);
                }
            }

            MindOption::Retire if self.self_belief() < -0.5 => {
                reasons.push(GoalKind::RetireOnMyTerms, -self.self_belief() * 0.6);
            }

            _ => {}
        }

        reasons
    }
}

#[cfg(test)]
mod tests {
    use super::super::MindTickContext;
    use super::super::organs::memory::ActorRef;
    use super::super::situation::MindSituation;
    use super::*;
    use crate::club::person::PersonAttributes;
    use chrono::NaiveDate;

    fn attrs() -> PersonAttributes {
        PersonAttributes {
            adaptability: 12.0,
            ambition: 12.0,
            controversy: 5.0,
            loyalty: 12.0,
            pressure: 12.0,
            professionalism: 12.0,
            sportsmanship: 12.0,
            temperament: 10.0,
            consistency: 12.0,
            important_matches: 12.0,
            dirtiness: 5.0,
        }
    }

    fn reflect(mind: &mut CompetitiveMind, situation: &MindSituation, organs: &mut MindOrgans) {
        let tick = MindTickContext::new(
            NaiveDate::from_ymd_opt(2030, 6, 1).unwrap(),
            7,
            &attrs(),
            50.0,
        );
        let view = MindView {
            tick: &tick,
            situation,
        };
        mind.reflect(&view, organs);
    }

    fn episode(kind: EpisodeKind) -> MindEpisode {
        MindEpisode::new(kind, ActorRef::NONE, 7, 100, kind.spec().valence, 0.8)
    }

    fn settled_but_benched() -> MindSituation {
        MindSituation {
            days_at_club: 400,
            starter_ratio: 0.05,
            expected_start_share: 0.70,
            age: 29,
            ..MindSituation::neutral()
        }
    }

    /// The same man, ranked in the bottom half at his position behind
    /// players he is plainly better than — the read of a manager who
    /// has decided.
    fn frozen_out() -> MindSituation {
        MindSituation {
            pecking_rank: 4,
            rivals_at_position: 4,
            rival_gap: 12,
            ..settled_but_benched()
        }
    }

    #[test]
    fn coming_back_into_the_reckoning_lifts_the_freeze() {
        let mut mind = CompetitiveMind::default();
        let mut organs = MindOrgans::new();

        // The run has to lengthen before watching reads as a freeze.
        for _ in 0..10 {
            reflect(&mut mind, &frozen_out(), &mut organs);
        }
        let held = organs
            .goals
            .get(GoalKind::WinBackMyPlace)
            .expect("he still wants his place");
        assert_eq!(held.blocked_by, GoalBlocker::FrozenOut);

        // Then he is starting again — the manager changed his mind, or
        // the man in front got hurt. Nothing about the want is answered
        // by one start; it is simply his to pursue again.
        let back_in = MindSituation {
            starter_ratio: 0.80,
            ..frozen_out()
        };
        reflect(&mut mind, &back_in, &mut organs);
        let freed = organs
            .goals
            .get(GoalKind::WinBackMyPlace)
            .expect("one start does not answer the want");
        assert_eq!(
            freed.blocked_by,
            GoalBlocker::None,
            "the freeze is a weekly read, not a life sentence"
        );
    }

    #[test]
    fn a_mistake_costs_more_belief_than_a_good_game_buys() {
        let mut organs = MindOrgans::new();

        let mut lifted = CompetitiveMind::default();
        lifted.observe(&episode(EpisodeKind::ManOfTheMatch), &mut organs);

        let mut knocked = CompetitiveMind::default();
        knocked.observe(&episode(EpisodeKind::CostlyError), &mut organs);

        assert!(knocked.self_belief().abs() > lifted.self_belief().abs());
    }

    #[test]
    fn belief_saturates_rather_than_running_away() {
        let mut organs = MindOrgans::new();
        let mut mind = CompetitiveMind::default();
        for _ in 0..80 {
            mind.observe(&episode(EpisodeKind::ManOfTheMatch), &mut organs);
        }
        assert!(mind.self_belief() <= 1.0);
        assert!(mind.self_belief() > 0.9);
    }

    #[test]
    fn the_run_tracks_the_last_stretch_not_the_season() {
        let mut organs = MindOrgans::new();
        let mut mind = CompetitiveMind::default();

        for _ in 0..5 {
            mind.observe(&episode(EpisodeKind::StartedBigMatch), &mut organs);
        }
        assert_eq!(mind.run, 5);

        mind.observe(&episode(EpisodeKind::DroppedToBench), &mut organs);
        assert_eq!(mind.run, -1, "the run breaks, it does not decrement");
    }

    #[test]
    fn a_benched_player_first_means_to_win_his_place_back() {
        let mut mind = CompetitiveMind::default();
        let mut organs = MindOrgans::new();
        reflect(&mut mind, &settled_but_benched(), &mut organs);

        assert!(organs.goals.pressure_of(GoalKind::WinBackMyPlace) > 0.0);
        assert_eq!(
            organs.goals.pressure_of(GoalKind::PlayFirstTeamFootball),
            0.0,
            "he has not given up yet"
        );
    }

    #[test]
    fn only_a_man_who_has_stopped_believing_looks_for_a_smaller_club() {
        let mut mind = CompetitiveMind::default();
        let mut organs = MindOrgans::new();

        // A long run out of the side, and the belief to match.
        for _ in 0..6 {
            mind.observe(&episode(EpisodeKind::DroppedToBench), &mut organs);
        }
        assert!(mind.is_out_of_the_side());
        assert!(mind.self_belief() < 0.0);

        reflect(&mut mind, &settled_but_benched(), &mut organs);
        assert!(
            organs.goals.pressure_of(GoalKind::PlayFirstTeamFootball) > 0.0,
            "the only want in the catalog that points downward, and it is earned"
        );
    }

    #[test]
    fn identical_minutes_are_a_grievance_for_one_man_and_not_another() {
        let build = |expected: f32| {
            let mut mind = CompetitiveMind::default();
            let mut organs = MindOrgans::new();
            reflect(
                &mut mind,
                &MindSituation {
                    days_at_club: 400,
                    starter_ratio: 0.20,
                    expected_start_share: expected,
                    ..MindSituation::neutral()
                },
                &mut organs,
            );
            organs.goals.pressure_of(GoalKind::WinBackMyPlace)
        };

        assert!(build(0.70) > 0.0, "a key player left out has a grievance");
        assert_eq!(
            build(0.15),
            0.0,
            "a backup playing a fifth of the games does not"
        );
    }

    #[test]
    fn a_new_signing_is_given_time_before_it_is_a_grievance() {
        let mut mind = CompetitiveMind::default();
        let mut organs = MindOrgans::new();
        reflect(
            &mut mind,
            &MindSituation {
                days_at_club: 20,
                ..settled_but_benched()
            },
            &mut organs,
        );
        assert!(organs.goals.is_empty());
    }

    #[test]
    fn playing_again_answers_the_want() {
        let mut mind = CompetitiveMind::default();
        let mut organs = MindOrgans::new();
        reflect(&mut mind, &settled_but_benched(), &mut organs);
        let goal = organs.goals.get(GoalKind::WinBackMyPlace).unwrap();
        assert_eq!(goal.progress(), 0.0);

        // He gets back in the side.
        reflect(
            &mut mind,
            &MindSituation {
                starter_ratio: 0.9,
                ..settled_but_benched()
            },
            &mut organs,
        );
        assert!(
            organs
                .goals
                .get(GoalKind::WinBackMyPlace)
                .unwrap()
                .progress()
                > 0.0
        );
    }

    #[test]
    fn a_drought_becomes_a_want_of_its_own() {
        let mut mind = CompetitiveMind::default();
        let mut organs = MindOrgans::new();
        reflect(
            &mut mind,
            &MindSituation {
                apps_since_goal: 14,
                starter_ratio: 0.9,
                expected_start_share: 0.7,
                ..MindSituation::neutral()
            },
            &mut organs,
        );
        assert!(organs.goals.pressure_of(GoalKind::EndTheDrought) > 0.0);
    }

    #[test]
    fn an_injury_dents_belief_and_coming_back_restores_some_of_it() {
        let mut organs = MindOrgans::new();
        let mut mind = CompetitiveMind::default();
        mind.observe(&episode(EpisodeKind::CareerThreateningInjury), &mut organs);
        let hurt = mind.self_belief();
        assert!(hurt < 0.0);

        mind.observe(&episode(EpisodeKind::ReturnedFromLongInjury), &mut organs);
        assert!(mind.self_belief() > hurt);
    }

    fn read(standard: f32, minutes: f32) -> CompetitiveMatchRead {
        CompetitiveMatchRead {
            standard,
            minutes,
            competitive: true,
            kept_clean_sheet: false,
        }
    }

    fn seeded(assurance: f32) -> CompetitiveMind {
        let mut mind = CompetitiveMind::default();
        mind.seed_assurance(assurance);
        mind
    }

    fn closed_share(from: f32, to: f32, matches: usize, pace: f32) -> f32 {
        let mut mind = seeded(from);
        for _ in 0..matches {
            mind.on_match_played(&read(to, 90.0), pace);
        }
        (mind.assurance() - from) / (to - from)
    }

    #[test]
    fn the_mind_switch_is_armed_unless_switched_off() {
        assert!(MindSwitch::armed());
    }

    #[test]
    fn a_median_keeper_settles_over_a_dozen_matches() {
        let median = CompetitiveMind::settling_pace(11.0, 11.0);
        let closed = closed_share(0.55, 0.67, 12, median);
        assert!(
            (0.5..=0.9).contains(&closed),
            "twelve full matches close half to nine-tenths of the step: {closed}"
        );
    }

    #[test]
    fn how_the_match_went_does_not_change_assurance() {
        let mut organs = MindOrgans::new();
        let pace = CompetitiveMind::settling_pace(11.0, 11.0);

        let mut hero = seeded(0.55);
        hero.on_match_played(
            &CompetitiveMatchRead {
                kept_clean_sheet: true,
                ..read(0.67, 90.0)
            },
            pace,
        );
        hero.observe(&episode(EpisodeKind::ManOfTheMatch), &mut organs);

        let mut culprit = seeded(0.55);
        culprit.on_match_played(&read(0.67, 90.0), pace);
        culprit.observe(&episode(EpisodeKind::CostlyError), &mut organs);
        culprit.observe(&episode(EpisodeKind::CostlyError), &mut organs);

        assert_eq!(hero.assurance(), culprit.assurance());
        assert!(hero.self_belief() > culprit.self_belief());
    }

    #[test]
    fn a_composed_player_settles_faster() {
        let composed = CompetitiveMind::settling_pace(18.0, 18.0);
        let nervous = CompetitiveMind::settling_pace(4.0, 4.0);
        assert!(closed_share(0.55, 0.67, 12, composed) > closed_share(0.55, 0.67, 12, nervous));
    }

    #[test]
    fn a_loan_down_lowers_assurance_slowly() {
        let pace = CompetitiveMind::settling_pace(11.0, 11.0);
        let run = |to: f32| {
            let mut mind = seeded(0.70);
            for _ in 0..38 {
                mind.on_match_played(&read(to, 90.0), pace);
            }
            (mind.assurance() - 0.70).abs()
        };
        assert!(run(0.60) < run(0.80));
    }

    #[test]
    fn a_cameo_counts_for_less_than_a_full_match() {
        let pace = CompetitiveMind::settling_pace(11.0, 11.0);
        let mut full = seeded(0.55);
        full.on_match_played(&read(0.67, 90.0), pace);
        let mut cameo = seeded(0.55);
        cameo.on_match_played(&read(0.67, 15.0), pace);
        assert!(full.assurance() > cameo.assurance());
        assert!(cameo.assurance() > 0.55);
    }

    #[test]
    fn a_friendly_teaches_less_than_a_competitive_match() {
        let pace = CompetitiveMind::settling_pace(11.0, 11.0);
        let mut competitive = seeded(0.55);
        competitive.on_match_played(&read(0.67, 90.0), pace);
        let mut friendly = seeded(0.55);
        friendly.on_match_played(
            &CompetitiveMatchRead {
                competitive: false,
                ..read(0.67, 90.0)
            },
            pace,
        );
        assert!(competitive.assurance() > friendly.assurance());
    }

    #[test]
    fn a_long_absence_fades_assurance_by_a_bounded_amount() {
        let mut mind = seeded(0.67);
        let mut organs = MindOrgans::new();
        let situation = MindSituation {
            days_at_club: 400,
            starter_ratio: 0.4,
            expected_start_share: 0.4,
            ..MindSituation::neutral()
        };
        for _ in 0..13 {
            reflect(&mut mind, &situation, &mut organs);
        }
        let faded = 0.67 - mind.assurance();
        assert!(faded > 0.0, "three months out leaves a mark");
        assert!(
            faded <= CompetitiveMind::FADE_CAP + 1e-6,
            "and never more than the cap: {faded}"
        );

        mind.on_match_played(&read(0.67, 90.0), 1.0);
        assert_eq!(mind.absent_fade, 0.0, "playing again resets the fade");
    }

    #[test]
    fn a_debutant_takes_a_costly_error_harder_than_a_veteran() {
        let mut organs = MindOrgans::new();
        let pace = CompetitiveMind::settling_pace(11.0, 11.0);

        let mut veteran = seeded(0.67);
        veteran.on_match_played(&read(0.67, 90.0), pace);
        veteran.observe(&episode(EpisodeKind::CostlyError), &mut organs);

        let mut debutant = seeded(0.52);
        debutant.on_match_played(&read(0.67, 90.0), pace);
        debutant.observe(&episode(EpisodeKind::CostlyError), &mut organs);

        assert!(debutant.self_belief() < veteran.self_belief());
    }

    #[test]
    fn a_penalty_save_lifts_a_keeper() {
        let mut organs = MindOrgans::new();
        let mut mind = seeded(0.67);
        mind.on_match_played(&read(0.67, 90.0), 1.0);
        mind.observe(&episode(EpisodeKind::PenaltySaved), &mut organs);
        assert!(mind.self_belief() > 0.0);
    }

    #[test]
    fn a_clean_sheet_lifts_belief_a_little() {
        let mut clean = seeded(0.67);
        clean.on_match_played(
            &CompetitiveMatchRead {
                kept_clean_sheet: true,
                ..read(0.67, 90.0)
            },
            1.0,
        );
        let mut organs = MindOrgans::new();
        let mut motm = seeded(0.67);
        motm.on_match_played(&read(0.67, 90.0), 1.0);
        motm.observe(&episode(EpisodeKind::ManOfTheMatch), &mut organs);

        assert!(clean.self_belief() > 0.0);
        assert!(clean.self_belief() < motm.self_belief());
    }

    #[test]
    fn appraisal_follows_belief() {
        let mut organs = MindOrgans::new();
        let mut confident = CompetitiveMind::default();
        for _ in 0..6 {
            confident.observe(&episode(EpisodeKind::StartedBigMatch), &mut organs);
        }
        assert!(confident.appraise(&organs).weighted() > 0.0);

        let mut shot = CompetitiveMind::default();
        for _ in 0..6 {
            shot.observe(&episode(EpisodeKind::DroppedToBench), &mut organs);
        }
        assert!(shot.appraise(&organs).weighted() < 0.0);
    }
}
