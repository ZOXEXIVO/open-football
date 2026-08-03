use super::dugout::DugoutDesk;
use super::facts::{CareerRecord, PlayerStanding, RecentEvents, SquadPulse, WeeklyMatchFacts};
use super::fans::FansDesk;
use super::market::MarketDesk;
use super::rumour::RumourDesk;
use crate::club::news::types::{NewsStory, NewsStoryKind};
use crate::{HappinessEventType, Person, Player, PlayerFieldPositionGroup, Team};
use chrono::{Datelike, Duration, NaiveDate};
use rustc_hash::FxHashSet;

/// Squad news: form, fitness, discipline, milestones, contracts — plus
/// everything a paper's own players say and do off the pitch.
///
/// This desk owns the single walk over the rosters one edition covers.
/// The rumour desk hangs off the same walk rather than repeating it: a
/// world with thousands of clubs cannot afford to iterate every squad
/// twice a week just to keep two detectors in separate files.
pub struct SquadDesk;

impl SquadDesk {
    /// Career appearances are reported at each round hundred, goals at
    /// each round fifty — the numbers a club shop prints on a
    /// commemorative shirt.
    const APPS_STEP: i32 = 100;
    const GOALS_STEP: i32 = 50;
    /// A player is only worth a milestone piece once he has been around
    /// long enough for the number to mean something locally.
    const SERVANT_SEASONS: i32 = 8;

    /// A player this age or younger is a prospect, and the same form
    /// reads as a breakthrough rather than as a good season.
    const PROSPECT_AGE: u8 = 21;
    /// The bar a young player has to clear before the paper calls him
    /// the real thing. Lower than the senior bar on purpose — holding a
    /// place in a senior side at nineteen IS the achievement — but he
    /// still has to have played enough for the number to mean anything.
    const RISING_STAR_RATING: f32 = 7.00;
    /// Senior appearances a prospect needs before there is anything to
    /// write, even when the club has already called it a breakthrough.
    const BREAKTHROUGH_MIN_APPS: i32 = 3;

    /// The single pass over the rosters one edition covers. Every desk
    /// that needs to look at players hangs off this one walk — the
    /// rumour mill, the dugout, the terraces, and the verdict on last
    /// summer's business. Splitting them into their own passes would
    /// multiply the weekly cost across every squad in the world for no
    /// editorial gain, and the squad-wide moods could not be tallied at
    /// all.
    ///
    /// `rosters` is the paper's own side plus any squad with no paper of
    /// its own that it speaks for, so every player in the world is
    /// walked exactly once a week however many editions a club prints.
    ///
    /// `sides` is every team the CLUB owns, which is a wider set than
    /// `rosters` and deliberately so: it is the filter on whose football
    /// the ratings page may report. A youth keeper handed a cup start by
    /// the first team is his club's news wherever his squad registration
    /// sits, while ninety minutes for his country — or for the club that
    /// sold him on Friday — are not, however current the roster entry
    /// is. See [`WeeklyMatchFacts`].
    ///
    /// Returns what the week did to the dressing room as a whole, for
    /// the desks whose stories only exist in aggregate.
    pub fn file(
        out: &mut Vec<NewsStory>,
        rosters: &[&Team],
        sides: &FxHashSet<u32>,
        facts: &WeeklyMatchFacts,
        played_this_week: bool,
        date: NaiveDate,
    ) -> SquadPulse {
        let mut pulse = SquadPulse::default();

        for team in rosters {
            let senior = team.team_type.is_own_team();

            if senior {
                Self::file_dressing_room_state(out, team, date);
                Self::file_shape_change(out, team, date);
            }

            for player in team.players.iter() {
                if player.is_retired() {
                    continue;
                }

                let feed = RecentEvents::week(player);

                Self::file_match_deeds(out, player.id, sides, facts, date);
                Self::file_fitness(out, player, &feed, date);
                Self::file_awards(out, player, &feed, date);
                Self::file_life_events(out, player, &feed, date);

                if senior {
                    pulse.seniors = pulse.seniors.saturating_add(1);

                    // The ratings page. Senior sides only: a reserve
                    // fixture has no correspondent marking it, and a
                    // youth-team afternoon is a development report
                    // rather than a verdict.
                    Self::file_outfield_deeds(
                        out,
                        player,
                        sides,
                        facts,
                        feed.happened(HappinessEventType::DerbyHero),
                        feed.happened(HappinessEventType::SeniorDebut),
                        date,
                    );
                    Self::file_form(out, player, played_this_week, date);
                    Self::file_breakthrough(out, player, &feed, date);
                    Self::file_discipline(out, player, &feed, date);
                    Self::file_dressing_room(out, player, &feed, date);
                    Self::file_career_life(out, player, date);
                    Self::file_adaptation(out, player, date);
                    Self::file_standing_in_the_building(out, player, date);
                    Self::file_off_field(out, player, &feed, date);
                    Self::file_competition_for_places(out, player, &feed, date);
                    Self::file_contract(out, player, &feed, date);
                    Self::file_move_meaning(out, player, &feed, date);
                    Self::file_life_outside_football(out, player, date);
                    Self::file_coach_verdict(out, team, player, date);
                    Self::file_development(out, player, date);
                    Self::file_dugout_ripple(out, player, &feed, date);
                    Self::file_small_beats(out, player, &feed, date);
                    DugoutDesk::file_player(out, player, &mut pulse, date);
                    FansDesk::file_player(out, player, &mut pulse, date);
                    MarketDesk::file_verdict(out, player, date);
                    RumourDesk::file_player(out, player, date);
                }
            }
        }

        pulse
    }

    /// A dressing room that has come together reads this well on the
    /// blended chemistry axis. Below the neutral fifty it is a squad;
    /// up here it is a group.
    const KNIT_CHEMISTRY: f32 = 68.0;
    /// Signings inside the ninety-day window before the churn is the
    /// story rather than the business.
    const TURNOVER_SIGNINGS: u8 = 5;
    /// Factions, and how hostile they are to each other. Both bars
    /// matter: a squad naturally forms friendship groups, and it is
    /// only a problem when those groups dislike one another.
    const CLIQUE_FACTIONS: u8 = 3;
    const CLIQUE_TENSION: f32 = 0.45;

    /// The state of the room, read off the weekly social snapshot the
    /// squad already keeps.
    ///
    /// Every manager in football claims to be building a dressing room
    /// and almost none of them can point at one. The simulation can:
    /// it blends pair harmony, leadership, coach trust, integration and
    /// turnover into a chemistry figure every week, and the press had
    /// no way to see any of it. These are conditions rather than days,
    /// so all three are `Standing` and wait on the back catalogue.
    fn file_dressing_room_state(out: &mut Vec<NewsStory>, team: &Team, date: NaiveDate) {
        let social = &team.social_snapshot;

        // Churn first: it is the one a supporter can verify against the
        // signings page, and it explains the other two.
        if social.recent_signings_90d >= Self::TURNOVER_SIGNINGS {
            out.push(
                NewsStory::new(NewsStoryKind::TurnoverToll, date)
                    .with_numbers(social.recent_signings_90d as i32, 0)
                    .weighted(social.turnover_penalty as i32),
            );
            return;
        }

        // A squad forms friendship groups by nature. It is only news
        // when those groups have stopped getting along, which is why
        // the tension bar is required and not just the count.
        if social.factions.faction_count >= Self::CLIQUE_FACTIONS
            && social.factions.faction_tension >= Self::CLIQUE_TENSION
        {
            out.push(
                NewsStory::new(NewsStoryKind::CliqueConcerns, date)
                    .with_numbers(
                        social.factions.faction_count as i32,
                        social.factions.isolated_players as i32,
                    )
                    .weighted((social.factions.faction_tension * 60.0) as i32),
            );
            return;
        }

        if social.team_chemistry >= Self::KNIT_CHEMISTRY {
            out.push(
                NewsStory::new(NewsStoryKind::SquadKnitsTogether, date)
                    .with_numbers(social.team_chemistry as i32, 0)
                    .weighted((social.team_chemistry - Self::KNIT_CHEMISTRY) as i32),
            );
        }
    }

    /// Movement in current ability, over a mark laid a quarter of a
    /// season ago, before the page will call it improvement or decline.
    ///
    /// Deliberately not one point. Ability drifts by a point on noise
    /// and a paper that reported that would be reporting noise; three
    /// is a change a coaching staff would notice and talk about.
    const DEVELOPMENT_STEP: i32 = 3;
    /// …and the bar for saying somebody has stopped, which needs a
    /// window to have genuinely passed with nothing in it.
    const STALL_WINDOW_DAYS: i64 = 150;
    /// Above this age improvement stops being expected and starts being
    /// a story; below it, the reverse.
    const IMPROVEMENT_EXPECTED_AGE: u8 = 23;
    /// The age at which going backwards is the story rather than a bad
    /// patch.
    const DECLINE_AGE: u8 = 30;

    /// Whether he is actually getting better.
    ///
    /// Nothing in the game stored a historical ability, so the press
    /// could see what a player is worth today and never whether that
    /// number had moved — which made a breakthrough, a plateau and a
    /// decline the same thing on a page. The development tick now
    /// leaves a mark every quarter-season, and this reads it.
    ///
    /// Current ability only. Potential stays hidden from everything
    /// that is not the engine, which is why none of these can say what
    /// a player might become — only what he has actually done.
    fn file_development(out: &mut Vec<NewsStory>, player: &Player, date: NaiveDate) {
        let marked_on = player.player_attributes.ability_marked_on_day;
        if marked_on == 0 {
            // No mark yet: the first tick lays a baseline, and a
            // baseline is not a comparison.
            return;
        }

        let held_for = (date.num_days_from_ce() - marked_on) as i64;
        let moved = player.player_attributes.current_ability as i32
            - player.player_attributes.ability_marker as i32;
        let age = player.age(date);
        let importance = PlayerStanding::importance(player);

        if moved >= Self::DEVELOPMENT_STEP {
            let kind = if age <= Self::IMPROVEMENT_EXPECTED_AGE {
                NewsStoryKind::BreakthroughSeason
            } else {
                // Improving after the age everybody stops expecting it
                // is rarer, and reads as a different story about the
                // same fact.
                NewsStoryKind::TrainingTransformation
            };
            out.push(
                NewsStory::new(kind, date)
                    .about(player.id)
                    .with_numbers(age as i32, moved)
                    .weighted(importance / 2 + moved * 4),
            );
            return;
        }

        if moved <= -Self::DEVELOPMENT_STEP && age >= Self::DECLINE_AGE {
            out.push(
                NewsStory::new(NewsStoryKind::PowersFading, date)
                    .about(player.id)
                    .with_numbers(age as i32, -moved)
                    .weighted(importance / 2),
            );
            return;
        }

        // Standing still. Only news for a young player, and only once
        // enough time has passed that the flat line is a fact rather
        // than a short window.
        if moved == 0
            && age <= Self::IMPROVEMENT_EXPECTED_AGE
            && held_for >= Self::STALL_WINDOW_DAYS
        {
            out.push(
                NewsStory::new(NewsStoryKind::StalledProspect, date)
                    .about(player.id)
                    .with_numbers(age as i32, 0)
                    .weighted(importance / 2),
            );
        }
    }

    /// Matches in the new shape before the change counts as a decision
    /// rather than as an experiment, and matches in the old one before
    /// it counts as having been the shape at all.
    const SHAPE_SETTLED: usize = 2;
    const SHAPE_ESTABLISHED: usize = 3;
    /// Once the new shape has been used this often it is simply the
    /// shape, and a piece about a change is a piece about old news.
    const SHAPE_STALE: usize = 5;

    /// The manager has changed the shape and stuck with it.
    ///
    /// Nothing about this is ever announced. The team sheet simply
    /// stops looking like it did, and every supporter works it out in
    /// the same week — which is exactly what this reads: the match log
    /// records the shape each side started in, so a sustained switch is
    /// visible without anything new being emitted anywhere.
    ///
    /// Both windows are load-bearing. Without the settled bar a single
    /// away-day back three reads as a revolution; without the
    /// established bar a club that has been rotating shapes all season
    /// gets a piece every fortnight about nothing.
    fn file_shape_change(out: &mut Vec<NewsStory>, team: &Team, date: NaiveDate) {
        let shapes: Vec<_> = team
            .match_history
            .items()
            .iter()
            .rev()
            .filter_map(|item| item.tactic_started)
            .take(Self::SHAPE_STALE + Self::SHAPE_ESTABLISHED)
            .collect();

        if shapes.len() < Self::SHAPE_SETTLED + Self::SHAPE_ESTABLISHED {
            return;
        }

        let current = shapes[0];
        let settled = shapes.iter().take_while(|shape| **shape == current).count();

        // Long enough to be a decision, short enough to still be news.
        if !(Self::SHAPE_SETTLED..Self::SHAPE_STALE).contains(&settled) {
            return;
        }

        // …and the thing before it has to have been a shape rather than
        // the previous week's experiment.
        let previous = shapes[settled];
        let established = shapes[settled..]
            .iter()
            .take_while(|shape| **shape == previous)
            .count();
        if established < Self::SHAPE_ESTABLISHED {
            return;
        }

        out.push(
            NewsStory::new(NewsStoryKind::FormationRevolution, date)
                .with_numbers(settled as i32, 0),
        );
    }

    /// What the manager privately thinks of him.
    ///
    /// The coach keeps a memory of every player — who he trusts in a
    /// big match, whose mistake he has not finished forgetting — and it
    /// decides team sheets while never being said out loud anywhere. A
    /// press box infers exactly this from watching who plays when it
    /// matters, so the paper is entitled to it; what it is not entitled
    /// to do is quote anybody on it, which is why neither of these is a
    /// quote piece.
    fn file_coach_verdict(out: &mut Vec<NewsStory>, team: &Team, player: &Player, date: NaiveDate) {
        use crate::club::staff::coach::memory::CoachMemoryFlags;

        let coach = team.staffs.head_coach();
        // A vacant dugout falls back to a stub with no id, and a stub
        // has no opinion about anybody.
        if coach.id == 0 {
            return;
        }

        let Some(memory) = coach.coach_memory.get(player.id) else {
            return;
        };

        let importance = PlayerStanding::importance(player);

        if memory.flags.contains(CoachMemoryFlags::STICKY_DOUBT) {
            out.push(
                NewsStory::new(NewsStoryKind::ManagerDoubtsLinger, date)
                    .about(player.id)
                    .weighted(importance / 2),
            );
            return;
        }

        if memory.flags.contains(CoachMemoryFlags::BIG_MATCH_PROVEN) {
            out.push(
                NewsStory::new(NewsStoryKind::BigMatchTrust, date)
                    .about(player.id)
                    .weighted(importance / 2),
            );
        }
    }

    /// Which injury piece an absence earns.
    ///
    /// "Out for eleven weeks" is a squad note. "He has done his
    /// cruciate" is news, and it is the same fact — the engine has
    /// always recorded the injury by name and the page never printed
    /// one. Only the three families a supporter can picture are split
    /// out; a dead leg and a back spasm stay the generic blow, because
    /// naming those would be precision nobody asked for.
    pub(super) fn injury_kind(player: &Player) -> NewsStoryKind {
        use crate::InjuryType as Hurt;

        match player.player_attributes.injury_type {
            Some(
                Hurt::HamstringStrain
                | Hurt::CalfStrain
                | Hurt::QuadStrain
                | Hurt::GroinStrain
                | Hurt::HipFlexorStrain,
            ) => NewsStoryKind::HamstringBlow,
            Some(Hurt::ACLTear | Hurt::PCLTear | Hurt::MCLSprain | Hurt::TornMeniscus) => {
                NewsStoryKind::KneeLigamentBlow
            }
            Some(Hurt::BrokenLeg | Hurt::StressFracture | Hurt::AchillesRupture) => {
                NewsStoryKind::BrokenBoneBlow
            }
            _ => NewsStoryKind::InjuryBlow,
        }
    }

    /// How a loan spell actually went, which the homecoming piece could
    /// never say.
    ///
    /// The verdict is recorded at the moment of return — it has to be,
    /// because the borrowing season's statistics are frozen and reset
    /// seconds later — and the page printed the same "he is back" for a
    /// player who started every week and one who never got off the
    /// bench. Those are opposite outcomes for the club that sent him.
    pub(super) fn homecoming_kind(verdict: crate::LoanSpellVerdict) -> NewsStoryKind {
        use crate::LoanSpellVerdict as How;

        match verdict {
            How::Standout | How::Successful => NewsStoryKind::LoanReturnTriumph,
            How::Peripheral | How::Struggled => NewsStoryKind::LoanReturnWasted,
            // A steady spell, or one too short to read, is the plain
            // homecoming. Inventing a verdict from a small sample is
            // exactly what the record refuses to do.
            How::Steady | How::Inconclusive => NewsStoryKind::LoanReturn,
        }
    }

    /// What the move actually meant to him.
    ///
    /// The market desk reports a transfer with a fee and a date, which
    /// is everything except the part a reader wants. The dressing room
    /// has always recorded the rest — that this was the club he grew up
    /// wanting, that the wages changed his life, that he arrived as a
    /// senior professional and discovered he is fourth choice — and
    /// none of it had anywhere to be printed.
    fn file_move_meaning(
        out: &mut Vec<NewsStory>,
        player: &Player,
        feed: &RecentEvents<'_>,
        date: NaiveDate,
    ) {
        let importance = PlayerStanding::importance(player);

        if feed
            .any_of(&[
                HappinessEventType::DreamMove,
                HappinessEventType::JoiningElite,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::DreamMoveComplete, date)
                    .about(player.id)
                    .weighted(importance),
            );
        }

        if feed.happened(HappinessEventType::DressingRoomStatusShock) {
            out.push(
                NewsStory::new(NewsStoryKind::StatusShock, date)
                    .about(player.id)
                    .weighted(importance / 2),
            );
        }

        // The money, both ways round. Kept as two kinds rather than one
        // with a sign, because a supporter reads a rise and a cut as
        // completely different stories about the same man.
        if feed.happened(HappinessEventType::SalaryBoost) {
            out.push(
                NewsStory::new(NewsStoryKind::PayWindfall, date)
                    .about(player.id)
                    .weighted(importance / 2),
            );
        } else if feed.happened(HappinessEventType::SalaryShock) {
            out.push(
                NewsStory::new(NewsStoryKind::WageRealityCheck, date)
                    .about(player.id)
                    .weighted(importance / 2),
            );
        }
    }

    /// The life a footballer has when he is not playing football.
    ///
    /// All of it arrives on one event type, so the desk had no way to
    /// tell a bereavement from a request for a language tutor and
    /// printed neither. Reading the kind back off the context is what
    /// makes these printable at all — and the reason this walks the
    /// fortnight rather than the week is that none of it is over in
    /// seven days.
    ///
    /// Nothing here is loud. A paper that only prints the loud things
    /// is a results service; the quiet ones are what make a squad read
    /// like a group of people rather than a list of assets.
    fn file_life_outside_football(out: &mut Vec<NewsStory>, player: &Player, date: NaiveDate) {
        use crate::LifeSimulationDesireKind as Life;

        let Some(kind) = RecentEvents::fortnight(player).life_event() else {
            return;
        };

        let importance = PlayerStanding::importance(player);

        let (story, weight) = match kind {
            // A death in the family is reported early, briefly, and
            // without a cheerful word anywhere near it.
            Life::BereavementLeave => (NewsStoryKind::CompassionateLeave, importance),
            Life::FamilyBirthLeave => (NewsStoryKind::FamilyCelebration, importance / 2),
            Life::FamilyUnsettledAbroad | Life::PartnerSchoolingConcern => {
                (NewsStoryKind::FamilyUnsettled, importance / 2)
            }
            Life::WantsLanguageTutor => (NewsStoryKind::LanguageLessons, importance / 3),
            Life::VeteranHomecomingSeason => (NewsStoryKind::VeteranHomecomingWish, importance),
            Life::ClubLegendRefusesLeave => (NewsStoryKind::LegendWontLeave, importance),
            Life::RefusesRivalMoveDespiteUpgrade => (NewsStoryKind::RefusesRivalMove, importance),
            Life::WantsLowerPressureClub => (NewsStoryKind::SeeksQuieterStage, importance / 2),
            // The rest are contract and selection asks the rumour and
            // dugout desks already tell better, or private enough that
            // a local paper would not have them.
            _ => return,
        };

        out.push(
            NewsStory::new(story, date)
                .about(player.id)
                .weighted(weight),
        );
    }

    /// What a change in the dugout does to the men who have to play for
    /// whoever is next.
    ///
    /// The boardroom desk has always reported the change itself. This is
    /// the half a supporter actually discusses: whether the squad has
    /// picked up, and which of them were signed by the man who has gone.
    fn file_dugout_ripple(
        out: &mut Vec<NewsStory>,
        player: &Player,
        feed: &RecentEvents<'_>,
        date: NaiveDate,
    ) {
        let importance = PlayerStanding::importance(player);

        if feed.happened(HappinessEventType::NewManagerBounce) {
            out.push(
                NewsStory::new(NewsStoryKind::NewManagerBounce, date)
                    .about(player.id)
                    .weighted(importance / 2),
            );
            return;
        }

        if feed
            .any_of(&[
                HappinessEventType::ManagerDeparture,
                HappinessEventType::SensesManagerChange,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::ManagerExitUnsettles, date)
                    .about(player.id)
                    .weighted(importance / 2),
            );
        }
    }

    /// The small human beats a week is mostly made of.
    ///
    /// A paper that only prints the loud things reads as a highlights
    /// reel. These are the lines that make it read like somewhere
    /// people work: a promise honoured, a ban served, a man on a
    /// programme of his own, a midfielder being taught a new job.
    fn file_small_beats(
        out: &mut Vec<NewsStory>,
        player: &Player,
        feed: &RecentEvents<'_>,
        date: NaiveDate,
    ) {
        let importance = PlayerStanding::importance(player);

        if feed.happened(HappinessEventType::PromiseKept) {
            out.push(
                NewsStory::new(NewsStoryKind::PromiseKept, date)
                    .about(player.id)
                    .weighted(importance / 2),
            );
        }

        if feed.happened(HappinessEventType::SuspensionServed) {
            out.push(NewsStory::new(NewsStoryKind::BanServed, date).about(player.id));
        }

        if feed.happened(HappinessEventType::PlayingForNewContract) {
            out.push(
                NewsStory::new(NewsStoryKind::PlayingForContract, date)
                    .about(player.id)
                    .weighted(importance / 2),
            );
        }

        if feed.happened(HappinessEventType::PersonalTrainingPlanSet) {
            out.push(NewsStory::new(NewsStoryKind::PersonalTrainingPlan, date).about(player.id));
        }

        if feed.happened(HappinessEventType::ManagerTacticalInstruction) {
            out.push(NewsStory::new(NewsStoryKind::RoleRetraining, date).about(player.id));
        }

        // He thinks the manager has favourites. Its own piece rather
        // than folded into being left out of one big match: that is a
        // decision about a fixture, and this is a belief about the
        // manager, which is the one that spreads.
        if feed.happened(HappinessEventType::FeelsSelectionFavouritism) {
            out.push(
                NewsStory::new(NewsStoryKind::FavouritismGrumbles, date)
                    .about(player.id)
                    .weighted(importance),
            );
        }
    }

    /// Saves that turn a keeper's afternoon into a piece. Below this he
    /// had a busy game; at it, he is the reason there was a point.
    const KEEPER_MASTERCLASS_SAVES: u16 = 6;
    /// Goals past him before the paper stops calling it one of those
    /// days. Four is the scoreline a supporter argues about all week.
    const KEEPER_OVERRUN_CONCEDED: u16 = 4;

    fn file_match_deeds(
        out: &mut Vec<NewsStory>,
        player_id: u32,
        sides: &FxHashSet<u32>,
        facts: &WeeklyMatchFacts,
        date: NaiveDate,
    ) {
        if let Some(goals) = facts.hat_trick_of(player_id, sides) {
            out.push(
                NewsStory::new(NewsStoryKind::HatTrick, date)
                    .about(player_id)
                    .with_numbers(goals as i32, 0)
                    // Four and five-goal hauls are a different story again.
                    .weighted((goals as i32 - 3) * 60),
            );
        }

        if facts.was_sent_off(player_id, sides) {
            out.push(NewsStory::new(NewsStoryKind::RedCard, date).about(player_id));
        }

        Self::file_keeper_deeds(out, player_id, sides, facts, date);
    }

    /// The bars a ratings column is written to. All calibrated against
    /// the engine's own rating bands rather than picked round: a neutral
    /// afternoon is 6.00 by construction, the good-performer band runs
    /// 7.0-7.4, and a poor display bottoms out in the high fives. See
    /// `match/engine/rating`.
    ///
    /// A mark in the eights is the afternoon a supporter still brings up
    /// a decade later.
    const MASTERCLASS_RATING: i32 = 800;
    /// …and the bottom of the same column. Deliberately NOT the mirror
    /// of the masterclass bar: the distribution is not symmetrical about
    /// 6.00, so 4.00 would print about nobody. 5.80 is the bottom tail —
    /// a genuinely poor afternoon rather than a quiet one.
    const STINKER_RATING: i32 = 580;
    /// A starter taken off before this minute was not being rested, and
    /// if he was being marked down while he was on it was not a
    /// tactical switch either. The rating bar sits just under the
    /// neutral 6.00 so a competent hour never reads as a hooking.
    const HOOKED_MINUTES: u16 = 62;
    const HOOKED_RATING: i32 = 615;
    /// Assists in one game before it is the story of his afternoon
    /// rather than a line in it.
    const ASSIST_SHOW: u16 = 3;
    /// Passes that made a chance for somebody else. The archetype wide
    /// midfielder serves 1-3 in a routine game, so four is the
    /// afternoon he actually ran.
    const CREATOR_KEY_PASSES: u16 = 4;
    /// Tackles, interceptions, blocks and clearances in one afternoon.
    /// A centre-half's routine shift is 5-9 and a busy one reaches 15,
    /// so this is the volume bar only — the RATING is what separates a
    /// man in command from a man drowning, and both are required.
    const DEFENSIVE_SHIFT: u16 = 8;
    const DEFENSIVE_ROCK_RATING: i32 = 720;
    /// Five completed dribbles in one game is a top-of-the-division
    /// afternoon; the routine wide man manages one or two.
    const DRIBBLING_DISPLAY: u16 = 5;
    /// Two goals. A hat-trick is the squad desk's; this is the mark
    /// above which the ratings column stops talking about the mark.
    const BRACE_GOALS: u16 = 2;
    /// A debut worth calling a dream needs an afternoon behind it, and
    /// the bar sits below the masterclass on purpose: a competent hour
    /// on a first appearance already is the story.
    const DEBUT_JOY_RATING: i32 = 720;
    /// The age framings. The young bar is below the masterclass because
    /// the achievement is the age; the veteran bar is above it because
    /// at thirty-four the surprise has to be real.
    const TEENAGE_STAR_RATING: i32 = 760;
    const VETERAN_AGE: u8 = 33;
    const VETERAN_TURN_RATING: i32 = 780;
    /// Both halves of a midfielder's job in one afternoon. Each bar is
    /// deliberately below its own single-quality piece — this is not
    /// the best passer or the best tackler on the pitch, it is the man
    /// who did both, which no existing line could say.
    const ENGINE_KEY_PASSES: u16 = 3;
    const ENGINE_DEFENSIVE_ACTIONS: u16 = 6;
    /// Shots, and the expected goals behind them, before an afternoon
    /// counts as squandered. Both bars matter and neither works alone:
    /// six efforts from thirty yards is not a miss (the xG floor throws
    /// it out) and one open goal put over the bar is bad luck rather
    /// than a pattern (the shot floor throws that out). A busy centre
    /// forward runs 2-4 shots at 0.3-0.8 expected goals, so this is the
    /// top of his range with nothing to show for it.
    const WASTEFUL_SHOTS: u16 = 4;
    const WASTEFUL_XG: i32 = 80;
    /// Fouls before the paper stops calling it a competitive edge. A
    /// side gives away about eleven in a match between eleven players,
    /// so five from one man is the afternoon he spent kicking people.
    /// A SECOND booking is deliberately not on this list — that is a
    /// sending-off, and `RedCard` is already its story.
    const FOUL_TROUBLE: u16 = 5;

    /// The ratings page: what ONE outfield player did in ONE afternoon.
    ///
    /// This is the column the paper never had. Every other beat on this
    /// desk reads a season (`StarForm` on an average, `GoalDrought` on a
    /// tally) or a career (the milestones), and the only match-level
    /// stories an outfield player could earn were a hat-trick and a
    /// sending-off. So the striker who burned four clear chances, the
    /// defender whose error gifted the winner, the midfielder who made
    /// three for other people and the full-back marked 5.1 in a hiding
    /// were all — in a newspaper about football — nothing at all.
    ///
    /// At most one line per player per week, and the order below is the
    /// order a reader remembers an afternoon in: what he did wrong first
    /// when it decided something, then what he did that won it, then the
    /// quieter verdicts. A paper does not print "he was the best man on
    /// the pitch" and "he was dreadful" about one player on one page, and
    /// where both are true of two different afternoons the louder one is
    /// the story.
    pub(super) fn file_outfield_deeds(
        out: &mut Vec<NewsStory>,
        player: &Player,
        sides: &FxHashSet<u32>,
        facts: &WeeklyMatchFacts,
        derby_hero: bool,
        debut: bool,
        date: NaiveDate,
    ) {
        let player_id = player.id;
        let Some(week) = facts.outfield_of(player_id, sides) else {
            return;
        };
        if week.is_routine() {
            return;
        }

        let age = player.age(date);
        let is_defender = matches!(
            player.position().position_group(),
            PlayerFieldPositionGroup::Defender
        );

        let verdict = |kind: NewsStoryKind, a: i32, b: i32, weight: i32| {
            NewsStory::new(kind, date)
                .about(player_id)
                .with_numbers(a, b)
                .weighted(weight)
        };

        // Winning a derby is the loudest thing a player can do in a
        // shirt, and it outranks anything else the same week did to him.
        if derby_hero {
            out.push(verdict(
                NewsStoryKind::DerbyHero,
                week.goals.max(1) as i32,
                week.best_rating,
                week.goals as i32 * 40,
            ));
            return;
        }

        // Into his own net. Nobody's fault and entirely his, which is
        // exactly why it leads over everything he did well.
        if week.own_goals > 0 {
            out.push(verdict(
                NewsStoryKind::OwnGoalShame,
                week.own_goals as i32,
                week.worst_rating,
                (week.own_goals as i32 - 1) * 50,
            ));
            return;
        }

        // Twelve yards, his to settle, and he missed. The one thing a
        // shoot-out is remembered by from the other end — the keeper's
        // save was already reported and the taker never was.
        if week.penalties_missed > 0 {
            out.push(verdict(
                NewsStoryKind::PenaltyMissed,
                week.penalties_missed as i32,
                week.worst_rating,
                (week.penalties_missed as i32 - 1) * 40,
            ));
            return;
        }

        // His mistake, their goal. Unlike a goalkeeper's error this one
        // had a goalkeeper behind it and still ended up in the net.
        if week.errors_leading_to_goal > 0 {
            out.push(verdict(
                NewsStoryKind::CostlyError,
                week.errors_leading_to_goal as i32,
                week.worst_rating,
                (week.errors_leading_to_goal as i32 - 1) * 45,
            ));
            return;
        }

        // A debut goes both ways and a player only ever gets one, so
        // both versions of it outrank the ordinary marks — including
        // the masterclass, which he may also have had.
        if debut {
            if week.goals > 0 || week.man_of_the_match || week.best_rating >= Self::DEBUT_JOY_RATING
            {
                out.push(verdict(
                    NewsStoryKind::DreamDebut,
                    week.goals as i32,
                    week.best_rating,
                    week.goals as i32 * 40,
                ));
                return;
            }
            if week.worst_rating > 0 && week.worst_rating <= Self::STINKER_RATING {
                out.push(verdict(
                    NewsStoryKind::DebutNightmare,
                    0,
                    week.worst_rating,
                    (Self::STINKER_RATING - week.worst_rating) / 4,
                ));
                return;
            }
        }

        // Two goals. Below a hat-trick, which the squad desk already
        // files, and above every mark the column can otherwise reach
        // for — because it is usually the reason the match was won.
        if week.goals == Self::BRACE_GOALS {
            out.push(verdict(
                NewsStoryKind::BraceHero,
                week.goals as i32,
                week.best_rating,
                (week.best_rating - 700).max(0) / 5,
            ));
            return;
        }

        // The same mark means a different thing at nineteen and at
        // thirty-four, and the column had one sentence for both.
        if age <= Self::PROSPECT_AGE && week.best_rating >= Self::TEENAGE_STAR_RATING {
            out.push(verdict(
                NewsStoryKind::TeenageStarTurn,
                age as i32,
                week.best_rating,
                (week.best_rating - Self::TEENAGE_STAR_RATING) / 4,
            ));
            return;
        }
        if age >= Self::VETERAN_AGE && week.best_rating >= Self::VETERAN_TURN_RATING {
            out.push(verdict(
                NewsStoryKind::RolledBackYears,
                age as i32,
                week.best_rating,
                (week.best_rating - Self::VETERAN_TURN_RATING) / 4,
            ));
            return;
        }

        if week.best_rating >= Self::MASTERCLASS_RATING {
            out.push(verdict(
                NewsStoryKind::MatchMasterclass,
                (week.goals.saturating_add(week.assists)) as i32,
                week.best_rating,
                (week.best_rating - Self::MASTERCLASS_RATING) / 4,
            ));
            return;
        }

        // A centre-half up for a corner. Rare enough that the whole
        // ground remembers who took it.
        if is_defender && week.goals > 0 {
            out.push(verdict(
                NewsStoryKind::GoalFromDefence,
                week.goals as i32,
                week.best_rating,
                week.goals as i32 * 30,
            ));
            return;
        }

        // A hand in two goals without scoring twice in either sense.
        if week.goals > 0 && week.assists > 0 {
            out.push(verdict(
                NewsStoryKind::GoalAndAssistShow,
                (week.goals.saturating_add(week.assists)) as i32,
                week.best_rating,
                (week.goals.saturating_add(week.assists) as i32 - 2) * 30,
            ));
            return;
        }

        // The engine's own verdict on the afternoon, which the newsroom
        // has been recording and throwing away since the match page
        // started showing it.
        if week.man_of_the_match {
            out.push(verdict(
                NewsStoryKind::ManOfTheMatch,
                (week.goals.saturating_add(week.assists)) as i32,
                week.best_rating,
                (week.best_rating - 700).max(0) / 5,
            ));
            return;
        }

        if week.assists >= Self::ASSIST_SHOW {
            out.push(verdict(
                NewsStoryKind::AssistShow,
                week.assists as i32,
                week.best_rating,
                (week.assists as i32 - Self::ASSIST_SHOW as i32) * 40,
            ));
            return;
        }

        // Off the bench and decisive. Half an hour that changed the
        // afternoon, and a story only a substitute can be the subject of.
        if week.impact_off_the_bench > 0 {
            out.push(verdict(
                NewsStoryKind::SuperSub,
                week.impact_off_the_bench as i32,
                week.best_rating,
                (week.impact_off_the_bench as i32 - 1) * 35,
            ));
            return;
        }

        // The chances were there and he put them everywhere but in. Both
        // bars are load-bearing: without the expected-goals floor this
        // fires on a winger who had six shots from distance, and without
        // the shot floor it fires on one missed sitter, which is bad luck
        // rather than a story.
        if week.goals == 0
            && week.shots >= Self::WASTEFUL_SHOTS
            && week.xg_x100 >= Self::WASTEFUL_XG
        {
            out.push(verdict(
                NewsStoryKind::WastefulFinishing,
                week.shots as i32,
                week.worst_rating,
                (week.xg_x100 - Self::WASTEFUL_XG) / 3,
            ));
            return;
        }

        // Taken off before the hour, having been marked down while he
        // was on. A manager's verdict delivered in public — and only if
        // it WAS one: `worst_rating_hooked` is false for an injury swap,
        // which otherwise looks identical from the stat line.
        if week.worst_rating_hooked
            && week.worst_rating_minutes <= Self::HOOKED_MINUTES
            && week.worst_rating <= Self::HOOKED_RATING
        {
            out.push(verdict(
                NewsStoryKind::HookedEarly,
                week.worst_rating_minutes as i32,
                week.worst_rating,
                (Self::HOOKED_RATING - week.worst_rating) / 5,
            ));
            return;
        }

        if week.worst_rating > 0 && week.worst_rating <= Self::STINKER_RATING {
            out.push(verdict(
                NewsStoryKind::MatchStinker,
                week.worst_rating_minutes as i32,
                week.worst_rating,
                (Self::STINKER_RATING - week.worst_rating) / 4,
            ));
            return;
        }

        // A defender's whole afternoon, and the one that never reaches a
        // scoreline. Three gates, and each throws out a different false
        // positive: the clean sheet (the same shift in a 4-0 defeat is a
        // man drowning, not a man in command), the position (a forward
        // who tracked back is not the rearguard story), and the rating —
        // because defensive volume RISES when a side is under the cosh,
        // so the count alone cannot tell command from siege.
        if week.shut_out
            && week.defensive_actions >= Self::DEFENSIVE_SHIFT
            && week.best_rating >= Self::DEFENSIVE_ROCK_RATING
            && matches!(
                player.position().position_group(),
                PlayerFieldPositionGroup::Defender
            )
        {
            out.push(verdict(
                NewsStoryKind::DefensiveRock,
                week.defensive_actions as i32,
                week.best_rating,
                (week.defensive_actions as i32 - Self::DEFENSIVE_SHIFT as i32) * 12,
            ));
            return;
        }

        if week.key_passes >= Self::CREATOR_KEY_PASSES {
            out.push(verdict(
                NewsStoryKind::CreatorInChief,
                week.key_passes as i32,
                week.best_rating,
                (week.key_passes as i32 - Self::CREATOR_KEY_PASSES as i32) * 15,
            ));
            return;
        }

        if week.successful_dribbles >= Self::DRIBBLING_DISPLAY {
            out.push(verdict(
                NewsStoryKind::DribblingDisplay,
                week.successful_dribbles as i32,
                week.best_rating,
                (week.successful_dribbles as i32 - Self::DRIBBLING_DISPLAY as i32) * 12,
            ));
            return;
        }

        // Both jobs in the same ninety minutes. Below the two pieces
        // that celebrate one quality outright, because the man who ran
        // the game is rarely the best at either half of it — which is
        // exactly why no existing line could describe him.
        if week.key_passes >= Self::ENGINE_KEY_PASSES
            && week.defensive_actions >= Self::ENGINE_DEFENSIVE_ACTIONS
        {
            out.push(verdict(
                NewsStoryKind::MidfieldEngine,
                (week.key_passes.saturating_add(week.defensive_actions)) as i32,
                week.best_rating,
                (week.defensive_actions as i32 - Self::ENGINE_DEFENSIVE_ACTIONS as i32) * 10,
            ));
            return;
        }

        // He spent the afternoon fouling people.
        if week.fouls >= Self::FOUL_TROUBLE {
            out.push(verdict(
                NewsStoryKind::FoulTrouble,
                week.fouls as i32,
                week.worst_rating,
                (week.fouls as i32 - Self::FOUL_TROUBLE as i32).max(0) * 10,
            ));
        }
    }

    /// The goalkeeper's week.
    ///
    /// Every other beat on this desk is read off goals, cards and
    /// ratings —
    /// the currency of the ten outfield players. A keeper earns none of
    /// it: his best afternoon and his worst leave the same mark on a
    /// scoreline, and until the press run read the match stat lines
    /// directly, the only thing a paper could say about him was how many
    /// clean sheets he had. These four are what actually happens to a
    /// goalkeeper.
    ///
    /// At most one per keeper per week, sourest first — a paper does not
    /// run "he was magnificent" and "he was at fault" about the same man
    /// on the same page, and the error is the one a reader remembers.
    pub(super) fn file_keeper_deeds(
        out: &mut Vec<NewsStory>,
        player_id: u32,
        sides: &FxHashSet<u32>,
        facts: &WeeklyMatchFacts,
        date: NaiveDate,
    ) {
        let Some(keeper) = facts.keeper_of(player_id, sides) else {
            return;
        };

        // The save the tie turned on outranks everything, including his
        // own mistakes — a shoot-out is remembered by its ending.
        if keeper.penalties_saved > 0 {
            out.push(
                NewsStory::new(NewsStoryKind::KeeperPenaltySave, date)
                    .about(player_id)
                    .with_numbers(keeper.penalties_saved as i32, keeper.saves as i32)
                    .weighted((keeper.penalties_saved as i32 - 1) * 70),
            );
            return;
        }

        if keeper.errors_leading_to_goal > 0 {
            out.push(
                NewsStory::new(NewsStoryKind::KeeperBlunder, date)
                    .about(player_id)
                    .with_numbers(keeper.errors_leading_to_goal as i32, keeper.conceded as i32)
                    .weighted((keeper.errors_leading_to_goal as i32 - 1) * 60),
            );
            return;
        }

        // A shot-stopping display and a hiding are not mutually
        // exclusive — a keeper can make eight saves and still pick the
        // ball out four times, and on that afternoon the saves are the
        // story rather than the scoreline.
        if keeper.saves >= Self::KEEPER_MASTERCLASS_SAVES {
            out.push(
                NewsStory::new(NewsStoryKind::KeeperMasterclass, date)
                    .about(player_id)
                    .with_numbers(keeper.saves as i32, keeper.conceded as i32)
                    .weighted((keeper.saves as i32 - Self::KEEPER_MASTERCLASS_SAVES as i32) * 25),
            );
            return;
        }

        if keeper.conceded >= Self::KEEPER_OVERRUN_CONCEDED {
            out.push(
                NewsStory::new(NewsStoryKind::KeeperOverrun, date)
                    .about(player_id)
                    .with_numbers(keeper.conceded as i32, keeper.saves as i32)
                    .weighted((keeper.conceded as i32 - Self::KEEPER_OVERRUN_CONCEDED as i32) * 30),
            );
        }
    }

    fn file_fitness(
        out: &mut Vec<NewsStory>,
        player: &Player,
        feed: &RecentEvents<'_>,
        date: NaiveDate,
    ) {
        let days_out = player.player_attributes.injury_days_remaining as i32;

        // Two weeks or more is a selection problem worth printing; a
        // knock that clears by Saturday is not news.
        if player.player_attributes.is_injured && days_out >= 14 {
            out.push(
                NewsStory::new(Self::injury_kind(player), date)
                    .about(player.id)
                    .with_numbers(days_out, 0)
                    .weighted(PlayerStanding::importance(player) + (days_out.min(180) / 3)),
            );
        }

        // A setback is a separate, sourer story: the club had a date in
        // mind and has just lost it.
        if feed.happened(HappinessEventType::InjurySetback) {
            out.push(
                NewsStory::new(NewsStoryKind::InjurySetback, date)
                    .about(player.id)
                    .with_numbers(days_out, 0)
                    .weighted(PlayerStanding::importance(player) / 2),
            );
        }
    }

    fn file_awards(
        out: &mut Vec<NewsStory>,
        player: &Player,
        feed: &RecentEvents<'_>,
        date: NaiveDate,
    ) {
        let week_start = date - Duration::days(7);
        // The timeline is chronological and can run to a thousand entries
        // for a decorated career, so walk back from the newest and stop
        // the moment the week closes rather than reading a whole career
        // for every player in the world, every week.
        let mut won_pom = false;
        let mut won_young_pom = false;
        for entry in player
            .awards_count
            .timeline
            .iter()
            .rev()
            .take_while(|entry| entry.date >= week_start)
        {
            if entry.date > date {
                continue;
            }
            match entry.kind {
                crate::AwardReputationKind::PlayerOfTheMonth => won_pom = true,
                crate::AwardReputationKind::YoungPlayerOfTheMonth => won_young_pom = true,
                _ => {}
            }
        }

        // The young award is its own piece rather than a line in the
        // senior one's. A nineteen-year-old beating other
        // nineteen-year-olds and a twenty-eight-year-old beating
        // everybody are different achievements, and a paper that
        // printed the same sentence for both was quietly telling its
        // readers it had not looked.
        if won_young_pom {
            out.push(
                NewsStory::new(NewsStoryKind::YoungPlayerOfMonthAward, date)
                    .about(player.id)
                    .with_numbers(player.age(date) as i32, 0),
            );
        } else if won_pom {
            out.push(NewsStory::new(NewsStoryKind::PlayerOfMonth, date).about(player.id));
        }

        // The rest of the honours ladder, none of which had anywhere to
        // appear. A weekly award is the only one most footballers ever
        // collect, and the young player of the season is one of the few
        // a town remembers the year of.
        if feed.happened(HappinessEventType::YoungPlayerOfTheSeason) {
            out.push(
                NewsStory::new(NewsStoryKind::YoungPlayerOfSeasonAward, date)
                    .about(player.id)
                    .with_numbers(player.age(date) as i32, 0)
                    .weighted(PlayerStanding::importance(player) / 2),
            );
        }

        if feed.happened(HappinessEventType::YoungPlayerOfTheWeek) {
            out.push(
                NewsStory::new(NewsStoryKind::YoungPlayerOfWeek, date)
                    .about(player.id)
                    .with_numbers(player.age(date) as i32, 0),
            );
        } else if feed.happened(HappinessEventType::PlayerOfTheWeek) {
            out.push(NewsStory::new(NewsStoryKind::PlayerOfWeek, date).about(player.id));
        }

        if feed
            .any_of(&[
                HappinessEventType::TeamOfTheMonthSelection,
                HappinessEventType::YoungTeamOfTheMonthSelection,
            ])
            .is_some()
        {
            out.push(NewsStory::new(NewsStoryKind::TeamOfMonthNod, date).about(player.id));
        }

        // The two honours only a goalkeeper can collect. The glove is a
        // season's verdict; the shut-out milestone is the number his
        // career is actually counted in, and neither has ever had
        // anywhere to appear on this page.
        // The award is handed out at the turn of the season, which is
        // also when the live counters are drained — so the figure its
        // copy quotes has to be checked rather than assumed. "On 0
        // clean sheets" under a golden-glove headline is the same
        // mistake as a season average of 0.00.
        if feed.happened(HappinessEventType::LeagueGoldenGlove) {
            let clean_sheets = player.statistics.clean_sheets as i32;
            if clean_sheets > 0 {
                out.push(
                    NewsStory::new(NewsStoryKind::KeeperGoldenGlove, date)
                        .about(player.id)
                        .with_numbers(clean_sheets, 0)
                        .weighted(PlayerStanding::importance(player) / 2),
                );
            }
        }

        if feed.happened(HappinessEventType::CleanSheetMilestone) {
            let clean_sheets = CareerRecord::clean_sheets(player);
            if clean_sheets > 0 {
                out.push(
                    NewsStory::new(NewsStoryKind::KeeperShutoutMilestone, date)
                        .about(player.id)
                        .with_numbers(clean_sheets, CareerRecord::appearances(player))
                        .weighted(clean_sheets / 2),
                );
            }
        }

        if feed
            .any_of(&[
                HappinessEventType::TeamOfTheWeekSelection,
                HappinessEventType::YoungTeamOfTheWeekSelection,
            ])
            .is_some()
        {
            // The rating lands in `b`, which is the slot the `{rating}`
            // placeholder reads from — and every phrasing of this piece
            // is built around it ("a rating of {rating} was enough for
            // the selectors"). The selection is made on last week's
            // football but the figure is a SEASON average, and the two
            // part company at the turn of the season: the XI is picked
            // from the closing week and the counters are drained days
            // later, leaving "a rating of 0.00 was enough for the
            // selectors" under a story about the best player in the
            // division. No figure, no piece — the honour still reaches
            // the page through the player's own event feed.
            let rating = player
                .statistics
                .average_rating_realistic(player.position().position_group());
            if rating > 0.0 {
                out.push(
                    NewsStory::new(NewsStoryKind::TeamOfTheWeek, date)
                        .about(player.id)
                        .with_numbers(0, (rating * 100.0) as i32),
                );
            }
        }
    }

    /// Beats the player's own event feed records: a debut, a recovery, a
    /// career milestone. The feed is the only place these are marked, so
    /// the desk reads it for the trigger and then sources the real number
    /// from the player's career record.
    fn file_life_events(
        out: &mut Vec<NewsStory>,
        player: &Player,
        feed: &RecentEvents<'_>,
        date: NaiveDate,
    ) {
        if feed.happened(HappinessEventType::SeniorDebut) {
            out.push(
                NewsStory::new(NewsStoryKind::YouthDebut, date)
                    .about(player.id)
                    .with_numbers(player.age(date) as i32, 0),
            );
        }

        if feed.happened(HappinessEventType::InjuryReturn) && !player.player_attributes.is_injured {
            out.push(NewsStory::new(NewsStoryKind::InjuryReturn, date).about(player.id));
        }

        if feed.happened(HappinessEventType::AppearanceMilestone) {
            let apps = CareerRecord::appearances(player);
            let milestone = apps - (apps % Self::APPS_STEP);
            if milestone >= Self::APPS_STEP {
                out.push(
                    NewsStory::new(NewsStoryKind::MilestoneApps, date)
                        .about(player.id)
                        .with_numbers(milestone, 0)
                        .weighted(milestone / 10),
                );
            }
        }

        if feed.happened(HappinessEventType::GoalMilestone) {
            let goals = CareerRecord::goals(player);
            let milestone = goals - (goals % Self::GOALS_STEP);
            if milestone >= Self::GOALS_STEP {
                out.push(
                    NewsStory::new(NewsStoryKind::MilestoneGoals, date)
                        .about(player.id)
                        .with_numbers(milestone, 0)
                        .weighted(milestone / 2),
                );
            }
        }

        if feed.happened(HappinessEventType::ClubServantMilestone) {
            let seasons = CareerRecord::seasons_at_current_club(player).max(Self::SERVANT_SEASONS);
            out.push(
                NewsStory::new(NewsStoryKind::ClubServant, date)
                    .about(player.id)
                    .with_numbers(seasons, CareerRecord::appearances(player))
                    .weighted(seasons * 8),
            );
        }

        if feed.happened(HappinessEventType::RetirementAnnounced) {
            // A knee that gave out and a planned farewell are the same
            // line on a squad list and nothing alike on a page: one man
            // chose the moment and the other had it chosen for him.
            let kind = match feed.retirement_reason() {
                Some(crate::RetirementReason::Injury) => NewsStoryKind::ForcedToRetire,
                _ => NewsStoryKind::RetirementAnnounced,
            };
            out.push(
                NewsStory::new(kind, date)
                    .about(player.id)
                    .with_numbers(player.age(date) as i32, CareerRecord::appearances(player))
                    .weighted(PlayerStanding::importance(player)),
            );
        }

        if feed
            .any_of(&[
                HappinessEventType::NationalTeamDebut,
                HappinessEventType::NationalTeamCallup,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::NationalCallUp, date)
                    .about(player.id)
                    .with_numbers(player.player_attributes.international_apps as i32, 0),
            );
        }

        // How the summer ended for the men who were away.
        //
        // The side-scoping invariant holds: this is not a club paper
        // claiming an international performance, which it may never do.
        // It is a club paper reporting what happened to one of its own
        // employees — read from his feed, exactly as it reads a
        // bereavement or a call-up.
        if feed.happened(HappinessEventType::NationalTeamTriumph) {
            out.push(
                NewsStory::new(NewsStoryKind::TournamentTriumph, date)
                    .about(player.id)
                    .with_numbers(player.player_attributes.international_apps as i32, 0)
                    .weighted(PlayerStanding::importance(player)),
            );
        } else if feed.happened(HappinessEventType::NationalTeamHeartbreak) {
            out.push(
                NewsStory::new(NewsStoryKind::TournamentHeartbreak, date)
                    .about(player.id)
                    .with_numbers(player.player_attributes.international_apps as i32, 0)
                    .weighted(PlayerStanding::importance(player) / 2),
            );
        }

        if feed.happened(HappinessEventType::CaptaincyAwarded) {
            out.push(NewsStory::new(NewsStoryKind::CaptainNamed, date).about(player.id));
        }

        // The armband going the other way. A paper that only ever
        // reports the naming of a captain is reporting half of the one
        // job in a dressing room everybody has an opinion about.
        if feed.happened(HappinessEventType::CaptaincyRemoved) {
            out.push(
                NewsStory::new(NewsStoryKind::CaptaincyLost, date)
                    .about(player.id)
                    .weighted(PlayerStanding::importance(player) / 2),
            );
        }

        // Whether there is another season in him. The announcement is
        // its own story; this is the months of speculation before it,
        // which is what a town actually spends the spring talking about.
        if feed.happened(HappinessEventType::RetirementConsidering)
            && !feed.happened(HappinessEventType::RetirementAnnounced)
        {
            out.push(
                NewsStory::new(NewsStoryKind::CareerTwilight, date)
                    .about(player.id)
                    .with_numbers(player.age(date) as i32, CareerRecord::appearances(player))
                    .weighted(PlayerStanding::importance(player) / 2),
            );
        }

        if feed.happened(HappinessEventType::ScoredAgainstFormerClub) {
            out.push(
                NewsStory::new(NewsStoryKind::FormerClubGoal, date)
                    .about(player.id)
                    .weighted(PlayerStanding::importance(player) / 3),
            );
        }

        // The goal every signing is asked about at every press
        // conference until he scores it. Distinct from the milestone
        // beats: this one is not a round number, it is the first.
        if feed.happened(HappinessEventType::FirstClubGoal) {
            out.push(
                NewsStory::new(NewsStoryKind::FirstClubGoal, date)
                    .about(player.id)
                    .weighted(PlayerStanding::importance(player) / 3),
            );
        }

        // Finished with his country, carrying on at his club. A career
        // decision with a date on it, and the one retirement a player
        // gets to make twice.
        if feed.happened(HappinessEventType::InternationalRetirement) {
            out.push(
                NewsStory::new(NewsStoryKind::InternationalRetirement, date)
                    .about(player.id)
                    .with_numbers(
                        player.player_attributes.international_apps as i32,
                        player.age(date) as i32,
                    )
                    .weighted(player.player_attributes.international_apps as i32),
            );
        }

        // The returnee: a spell away has ended and he is back in the
        // building. Which of the four ways it ended decides how loudly
        // the paper says so.
        //
        // Every phrasing of the piece is built around what he did while
        // he was away — "{n} appearances and {m} goals away from home" —
        // so the spell record is required rather than optional. Reading
        // the numbers off the player instead, as this once did, printed
        // the spell he has not started yet: `on_loan_return` freezes the
        // borrowing season into the ledger and resets the live counters
        // on the way through the door, so every homecoming went to print
        // as nought appearances and nought goals.
        if let Some(homecoming) = feed.any_of(&[
            HappinessEventType::BackedAfterLoanReturn,
            HappinessEventType::ReturnedFromLoanProven,
            HappinessEventType::UnsettledAfterLoanReturn,
            HappinessEventType::ReturnedFromLoanDeflated,
        ]) && let Some(spell) = feed.loan_spell()
        {
            let weight = match homecoming {
                HappinessEventType::BackedAfterLoanReturn => 60,
                HappinessEventType::ReturnedFromLoanProven => 40,
                HappinessEventType::UnsettledAfterLoanReturn => 10,
                _ => 0,
            };
            out.push(
                NewsStory::new(Self::homecoming_kind(spell.verdict), date)
                    .about(player.id)
                    .with_numbers(spell.appearances as i32, spell.goals as i32)
                    .weighted(weight),
            );
        }
    }

    fn file_form(
        out: &mut Vec<NewsStory>,
        player: &Player,
        played_this_week: bool,
        date: NaiveDate,
    ) {
        if !played_this_week {
            return;
        }

        let stats = &player.statistics;
        let apps = stats.played as i32 + stats.played_subs as i32;
        if apps < 6 {
            return;
        }

        let group = player.position().position_group();
        let rating = stats.average_rating_realistic(group);

        if matches!(group, PlayerFieldPositionGroup::Goalkeeper) {
            // A goalkeeper's season is measured in shut-outs, not ratings.
            let clean_sheets = stats.clean_sheets as i32;
            if clean_sheets >= 4 && rating >= 6.9 {
                out.push(
                    NewsStory::new(NewsStoryKind::KeeperWall, date)
                        .about(player.id)
                        .with_numbers(clean_sheets, (rating * 100.0) as i32)
                        .weighted(clean_sheets * 12),
                );
            }
            return;
        }

        // A nineteen-year-old holding down a senior place gets the
        // breakthrough piece, not the same "carrying the team" line the
        // paper runs about a 29-year-old. `file_breakthrough` owns him.
        if player.age(date) <= Self::PROSPECT_AGE {
            return;
        }

        if rating >= 7.20 {
            let goals = stats.goals as i32;
            out.push(
                NewsStory::new(NewsStoryKind::StarForm, date)
                    .about(player.id)
                    .with_numbers(goals, (rating * 100.0) as i32)
                    .weighted(((rating - 7.20) * 200.0) as i32 + goals * 8),
            );
        }
    }

    /// The kid who has arrived. Two ways in: the club's own machinery
    /// has already marked the breakthrough, or the numbers speak for
    /// themselves — a real run of senior football at an age when most
    /// of his year group are still in the youth team.
    ///
    /// Deliberately not gated on ability or potential. Clubs cannot read
    /// a player's ceiling and neither can a newspaper; what a reporter
    /// has is his age, his minutes and his ratings.
    fn file_breakthrough(
        out: &mut Vec<NewsStory>,
        player: &Player,
        feed: &RecentEvents<'_>,
        date: NaiveDate,
    ) {
        let age = player.age(date);
        if age > Self::PROSPECT_AGE {
            return;
        }

        let stats = &player.statistics;
        let apps = stats.played as i32 + stats.played_subs as i32;
        let rating = stats.average_rating_realistic(player.position().position_group());

        // The piece is built on a rating, so there has to be one. A
        // fifteen-year-old moved onto the senior roster fires the club's
        // own breakthrough event with no minutes behind it at all, and
        // `average_rating_realistic` answers 0.0 for a player who has
        // never been rated — which is how "a season average of 0.00"
        // reached a front page. Being promoted is `YouthDebut`'s story;
        // this one needs a record.
        if rating <= 0.0 || apps < Self::BREAKTHROUGH_MIN_APPS {
            return;
        }

        // The club marking the breakthrough is evidence in itself, so it
        // lowers the bar — but only the form bar, never the "has he
        // actually played" one.
        let announced = feed.happened(HappinessEventType::YouthBreakthrough);
        let earned = apps >= 6 && stats.played as i32 >= 3 && rating >= Self::RISING_STAR_RATING;

        if !announced && !earned {
            return;
        }

        // The younger he is and the better he has played, the louder it
        // runs — a seventeen-year-old at 7.4 is a back-page lead, a
        // twenty-one-year-old at 7.0 is a column.
        let youth_bonus = (Self::PROSPECT_AGE as i32 - age as i32) * 18;
        let form_bonus = ((rating - Self::RISING_STAR_RATING) * 120.0).max(0.0) as i32;

        out.push(
            NewsStory::new(NewsStoryKind::RisingStar, date)
                .about(player.id)
                .with_numbers(age as i32, (rating * 100.0) as i32)
                .weighted(youth_bonus + form_bonus + if announced { 40 } else { 0 }),
        );
    }

    /// Bans, and the goals that will not come. Both are selection
    /// problems the manager has to answer for, which is what makes them
    /// print.
    fn file_discipline(
        out: &mut Vec<NewsStory>,
        player: &Player,
        feed: &RecentEvents<'_>,
        date: NaiveDate,
    ) {
        if feed.happened(HappinessEventType::BannedThroughAccumulation) {
            out.push(
                NewsStory::new(NewsStoryKind::Suspension, date)
                    .about(player.id)
                    .with_numbers(player.statistics.yellow_cards as i32, 0)
                    .weighted(PlayerStanding::importance(player) / 2),
            );
        }

        if feed.happened(HappinessEventType::ScoringDroughtConcern) {
            out.push(
                NewsStory::new(NewsStoryKind::GoalDrought, date)
                    .about(player.id)
                    .with_numbers(
                        player.statistics.goals as i32,
                        player.statistics.played as i32 + player.statistics.played_subs as i32,
                    )
                    .weighted(PlayerStanding::importance(player) / 3),
            );
        }

        if feed.happened(HappinessEventType::GoalDroughtEnded) {
            out.push(
                NewsStory::new(NewsStoryKind::DroughtEnded, date)
                    .about(player.id)
                    .with_numbers(player.statistics.goals as i32, 0),
            );
        }
    }

    /// What the training ground and the terraces are saying. Colour
    /// pieces, deliberately low-priority: they fill the briefs column on
    /// a quiet week and never crowd out real football.
    fn file_dressing_room(
        out: &mut Vec<NewsStory>,
        player: &Player,
        feed: &RecentEvents<'_>,
        date: NaiveDate,
    ) {
        if feed
            .any_of(&[
                HappinessEventType::TrainingGroundBustUp,
                HappinessEventType::PublicApology,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::TrainingBustUp, date)
                    .about(player.id)
                    .weighted(PlayerStanding::importance(player) / 2),
            );
            return;
        }

        // A voice where there was not one. The dressing room's quiet
        // hierarchy changing is a story a paper only ever gets to write
        // about the armband — this is the version that happens first,
        // and it is how a club finds its next captain.
        if feed.happened(HappinessEventType::LeadershipEmergence) {
            out.push(
                NewsStory::new(NewsStoryKind::LeaderEmerging, date)
                    .about(player.id)
                    .with_numbers(player.age(date) as i32, 0)
                    .weighted(PlayerStanding::importance(player) / 3),
            );
            return;
        }

        // Somebody he was close to has gone. The transfer itself was
        // reported as the buying club's business; this is what it did to
        // the man left behind, which is the half a local paper cares
        // about.
        if feed
            .any_of(&[
                HappinessEventType::CloseFriendSold,
                HappinessEventType::MentorDeparted,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::TeammateFarewell, date)
                    .about(player.id)
                    .weighted(PlayerStanding::importance(player) / 3),
            );
        }
    }

    /// The slower half of a foreign player's life: the language coming,
    /// somebody he can speak to walking through the door, and — at the
    /// other end of a career — the first look at the coaching badges.
    ///
    /// Both are conditions rather than days, so both read the longer
    /// window and are `Standing`: settling in is not something that
    /// happened on a Tuesday.
    fn file_career_life(out: &mut Vec<NewsStory>, player: &Player, date: NaiveDate) {
        let feed = RecentEvents::fortnight(player);

        if feed
            .any_of(&[
                HappinessEventType::CompatriotJoined,
                HappinessEventType::LanguageProgress,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::SettlingIn, date)
                    .about(player.id)
                    .weighted(PlayerStanding::importance(player) / 4),
            );
        }

        if feed.happened(HappinessEventType::CoachingCareerInterest) {
            out.push(
                NewsStory::new(NewsStoryKind::CoachingAmbition, date)
                    .about(player.id)
                    .with_numbers(player.age(date) as i32, 0)
                    .weighted(PlayerStanding::importance(player) / 4),
            );
        }
    }

    /// The foreign player's life away from the pitch.
    ///
    /// This is the half of a transfer that no fee explains and no form
    /// table shows. The simulation has always run it — a signing who
    /// cannot settle, one who never learns the language, one openly
    /// hoping for a move back toward home, one the dressing room will
    /// not have because of the shirt he used to wear — and the paper
    /// printed none of it, so a foreign player's decline read as
    /// unexplained bad form.
    ///
    /// All conditions rather than days, so all read the longer window
    /// and all are `Standing`: nobody becomes homesick on a Tuesday. One
    /// line per player, sourest first — a man who wants to go home is
    /// not also a man settling in.
    fn file_adaptation(out: &mut Vec<NewsStory>, player: &Player, date: NaiveDate) {
        let feed = RecentEvents::fortnight(player);
        let importance = PlayerStanding::importance(player);

        // He wants to go home. The one thing on this page nobody can
        // report for him — form can be observed, homesickness only
        // exists once he says it — which is why it is set as a quote.
        if feed.happened(HappinessEventType::WantsReturnHome) {
            out.push(
                NewsStory::new(NewsStoryKind::HomesickAbroad, date)
                    .about(player.id)
                    .with_numbers(player.age(date) as i32, 0)
                    .weighted(importance),
            );
            return;
        }

        // …and the same wish with a destination attached: a club in his
        // own country has actually come in. Named where the feed knew
        // who it was, because "a club back home" and "his first club
        // back home" are different stories.
        if feed.happened(HappinessEventType::HomeReturnOpportunity) {
            out.push(
                NewsStory::new(NewsStoryKind::HomeCalling, date)
                    .about(player.id)
                    .against(feed.interested_club(HappinessEventType::HomeReturnOpportunity))
                    .weighted(importance),
            );
            return;
        }

        // Signed from the neighbours, and the room has not forgotten.
        if feed.happened(HappinessEventType::ColdShoulderOverRivalPast) {
            out.push(
                NewsStory::new(NewsStoryKind::ColdShoulder, date)
                    .about(player.id)
                    .weighted(importance / 2),
            );
            return;
        }

        // A year in and still on his own. The explanation nobody reaches
        // for when a signing is not working.
        if feed.happened(HappinessEventType::FeelingIsolated) {
            out.push(
                NewsStory::new(NewsStoryKind::StrugglingToSettle, date)
                    .about(player.id)
                    .weighted(importance / 2),
            );
            return;
        }

        // He has stopped looking like a man in the wrong country.
        if feed.happened(HappinessEventType::SettledIntoSquad) {
            out.push(
                NewsStory::new(NewsStoryKind::SettledAtLast, date)
                    .about(player.id)
                    .weighted(importance / 3),
            );
        }
    }

    /// Where he stands in the building: how he is training, how much he
    /// is playing, what he makes of his wages, and whether the division
    /// is still big enough for him.
    ///
    /// Every one of these is a state rather than an event, so all are
    /// `Standing` and read the fortnight. The pecking order is the one a
    /// back page uses: a grievance outranks contentment, and something
    /// that stops him playing at all outranks both.
    fn file_standing_in_the_building(out: &mut Vec<NewsStory>, player: &Player, date: NaiveDate) {
        let feed = RecentEvents::fortnight(player);
        let importance = PlayerStanding::importance(player);

        // Not injured, not dropped — ineligible. Worse than either,
        // because it takes a window to undo.
        if feed.happened(HappinessEventType::SquadRegistrationOmitted) {
            // "Left out of the squad list" is an administrative
            // sentence. Which rule did it is a story about how the club
            // assembled itself, and a local readership takes the
            // homegrown version of it personally.
            use crate::RegulationSlotKind as Slot;
            let kind = match feed.regulation_slot(HappinessEventType::SquadRegistrationOmitted) {
                Some(Slot::HomegrownQuota) => NewsStoryKind::HomegrownQuotaOmission,
                Some(Slot::NonEuQuota | Slot::InternationalRegistration) => {
                    NewsStoryKind::ForeignQuotaOmission
                }
                _ => NewsStoryKind::LeftOutOfSquadList,
            };
            out.push(
                NewsStory::new(kind, date)
                    .about(player.id)
                    .weighted(importance),
            );
            return;
        }

        // He has outgrown the division and everybody can see it. A story
        // about the club as much as about him.
        if feed.happened(HappinessEventType::TooGoodForLevel) {
            out.push(
                NewsStory::new(NewsStoryKind::OutgrownDivision, date)
                    .about(player.id)
                    .with_numbers(player.age(date) as i32, 0)
                    .weighted(importance),
            );
            return;
        }

        // Sick of the bench, and no longer hiding it.
        if feed
            .any_of(&[
                HappinessEventType::LackOfPlayingTime,
                HappinessEventType::WantsFirstTeamFootball,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::BenchFrustration, date)
                    .about(player.id)
                    .weighted(importance),
            );
            return;
        }

        // He has seen what the man next to him earns. Read through the
        // event rather than off the wage bill on purpose: the dressing
        // room's sense of what is fair is already modelled, and a desk
        // comparing two salaries itself would be inventing a grievance
        // nobody in the squad actually has.
        if feed.happened(HappinessEventType::SalaryGapNoticed) {
            out.push(
                NewsStory::new(NewsStoryKind::WageEnvy, date)
                    .about(player.id)
                    .weighted(importance / 2),
            );
            return;
        }

        if feed.happened(HappinessEventType::RelegationFear) {
            out.push(
                NewsStory::new(NewsStoryKind::RelegationNerves, date)
                    .about(player.id)
                    .weighted(importance / 2),
            );
            return;
        }

        // The training ground. The quietest column in the paper and the
        // one that most often runs the week before somebody's run in the
        // side starts or ends.
        if feed
            .any_of(&[
                HappinessEventType::PoorTraining,
                HappinessEventType::TrainingStandardFrustration,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::TrainingConcerns, date)
                    .about(player.id)
                    .weighted(importance / 3),
            );
            return;
        }

        if feed
            .any_of(&[
                HappinessEventType::GoodTraining,
                HappinessEventType::EliteTrainingLift,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::TrainingGroundBuzz, date)
                    .about(player.id)
                    .weighted(importance / 4),
            );
        }
    }

    /// The things that happen to a footballer on a date and have nothing
    /// to do with a stat line: trouble away from the ground, a row with
    /// a teammate, a senior pro quietly taking him on, a shirt number
    /// changing hands, an international career ending by omission, a
    /// contract torn up, and the honours a season hands out.
    ///
    /// All read the seven-day feed on a seven-day tick, so all are
    /// `Event`. One line per player: the loudest wins.
    fn file_off_field(
        out: &mut Vec<NewsStory>,
        player: &Player,
        feed: &RecentEvents<'_>,
        date: NaiveDate,
    ) {
        let importance = PlayerStanding::importance(player);

        // The season's individual verdicts, from the player of the year
        // down to a place in the team of the season. Every one of them
        // is a front page somewhere and none had anywhere to appear.
        if feed
            .any_of(&[
                HappinessEventType::WorldPlayerOfYear,
                HappinessEventType::ContinentalPlayerOfYear,
                HappinessEventType::PlayerOfTheSeason,
                HappinessEventType::TeamOfTheYearSelection,
                HappinessEventType::TeamOfTheSeasonSelection,
                HappinessEventType::WorldPlayerOfYearNomination,
                HappinessEventType::ContinentalPlayerOfYearNomination,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::SeasonAward, date)
                    .about(player.id)
                    .weighted(importance),
            );
            return;
        }

        // Off-field trouble: every paper's favourite story and every
        // manager's least.
        if feed.happened(HappinessEventType::ControversyIncident) {
            out.push(
                NewsStory::new(NewsStoryKind::OffFieldControversy, date)
                    .about(player.id)
                    .weighted(importance),
            );
            return;
        }

        if feed.happened(HappinessEventType::ContractTerminated) {
            out.push(
                NewsStory::new(NewsStoryKind::ContractTornUp, date)
                    .about(player.id)
                    .weighted(importance),
            );
            return;
        }

        // Two of them went at it. What it was about is recorded, and
        // the three rows a dressing room genuinely has are worth naming
        // — a disagreement over training standards, two men after the
        // same shirt, and a senior pro pulling rank are different
        // stories with different consequences. Anything else falls
        // through to the flat piece, which is the honest answer when
        // the reason is "they do not like each other".
        if feed.happened(HappinessEventType::ConflictWithTeammate) {
            use crate::TeammateConflictReason as Row;

            let kind = match feed.conflict_reason() {
                Some(Row::TrainingStandards) => NewsStoryKind::TrainingStandardsRow,
                Some(Row::PositionalRivalry) => NewsStoryKind::PositionRivalryFeud,
                Some(Row::LeadershipChallenge) => NewsStoryKind::LeadershipPowerStruggle,
                // A row about money is the wage-envy piece, which the
                // desk already tells better and from the right angle.
                _ => NewsStoryKind::TeammateConflict,
            };

            out.push(
                NewsStory::new(kind, date)
                    .about(player.id)
                    .weighted(importance),
            );
            return;
        }

        // His country has stopped picking him. The quiet end of an
        // international career, and the paper has only ever printed the
        // call-up that began it.
        if feed.happened(HappinessEventType::NationalTeamDropped) {
            out.push(
                NewsStory::new(NewsStoryKind::DroppedByCountry, date)
                    .about(player.id)
                    .with_numbers(player.player_attributes.international_apps as i32, 0)
                    .weighted(importance),
            );
            return;
        }

        // Somebody stood up in the dressing room and said it out loud.
        if feed.happened(HappinessEventType::DressingRoomSpeech) {
            out.push(
                NewsStory::new(NewsStoryKind::DressingRoomSpeech, date)
                    .about(player.id)
                    .weighted(importance / 2),
            );
            return;
        }

        // The number nine changes hands. A small thing everywhere except
        // in the town it happens in.
        if feed.happened(HappinessEventType::ShirtNumberPromotion) {
            out.push(
                NewsStory::new(NewsStoryKind::ShirtNumberHandover, date)
                    .about(player.id)
                    .weighted(importance / 3),
            );
            return;
        }

        // The senior pro who has taken the new boy on. The quiet half of
        // a dressing room, and the half that decides whether a signing
        // works at all.
        if feed
            .any_of(&[
                HappinessEventType::SeniorMentorSupport,
                HappinessEventType::LearningFromStarTeammate,
                HappinessEventType::TakesReplacementUnderWing,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::TakenUnderWing, date)
                    .about(player.id)
                    .weighted(importance / 4),
            );
        }
    }

    /// Who is after whose shirt. Competition for places is the thing a
    /// dressing room is actually made of, and until now the paper only
    /// ever saw its outcome on a team sheet — never the argument.
    ///
    /// Two versions, and the sourer one wins: the club's own kid kept
    /// out by a borrowed player of the same level (the grievance a local
    /// readership takes personally, and the reason a homegrown core
    /// mutters about the message it sends), or the incumbent who can
    /// feel somebody coming for his place.
    fn file_competition_for_places(
        out: &mut Vec<NewsStory>,
        player: &Player,
        feed: &RecentEvents<'_>,
        date: NaiveDate,
    ) {
        // Why the manager left him out. The selector records a football
        // reason for every omission and the page could only say he was
        // not picked — which reads the same for a man being rested, a
        // man whose profile did not suit the opponent, a man dropped on
        // form and a man being punished. Four different stories, and
        // only one of them is a grievance.
        if feed.happened(HappinessEventType::MatchDropped) {
            use crate::SelectionOmissionReason as Why;

            let kind = match feed.omission_reason() {
                Some(
                    Why::FatigueManagement
                    | Why::FitnessProtection
                    | Why::MedicalRecurrenceRisk
                    | Why::ReturningFromInjury
                    | Why::CupRotation
                    | Why::LowMatchImportanceRotation
                    | Why::YouthDevelopmentRotation,
                ) => Some(NewsStoryKind::RotationRested),
                Some(
                    Why::TacticalMismatch
                    | Why::PositionFitIssue
                    | Why::NoNaturalRoleInFormation
                    | Why::OpponentMatchupMismatch
                    | Why::LineupBalanceCall
                    | Why::BenchScenarioCoverage
                    | Why::BenchBalance
                    | Why::TeammatePreferredForTacticalBalance,
                ) => Some(NewsStoryKind::TacticalOmission),
                Some(
                    Why::PoorRecentForm
                    | Why::LowerMatchReadiness
                    | Why::TeammatePreferredOnForm
                    | Why::TeammatePreferredOnAbility
                    | Why::TeammatePreferredOnFitness,
                ) => Some(NewsStoryKind::DroppedOnForm),
                Some(Why::DisciplinarySelection) => Some(NewsStoryKind::DisciplinaryOmission),
                // Trust, squad status and integration windows are the
                // dugout desk's territory: they are about a
                // relationship rather than about one team sheet.
                _ => None,
            };

            if let Some(kind) = kind {
                out.push(
                    NewsStory::new(kind, date)
                        .about(player.id)
                        .weighted(PlayerStanding::importance(player) / 2),
                );
                return;
            }
        }

        // Two different grievances that shared a headline for too long.
        // A kid behind a loan signing is a complaint about the club's
        // patience; one of our own behind an import is a complaint
        // about where the two of them are from, and a local readership
        // takes the second one personally in a way it does not take the
        // first.
        if feed.happened(HappinessEventType::UnhappyAboutBlockedHomegrown) {
            out.push(
                NewsStory::new(NewsStoryKind::HomegrownBlocked, date)
                    .about(player.id)
                    .with_numbers(player.age(date) as i32, 0)
                    .weighted(PlayerStanding::importance(player) / 2),
            );
            return;
        }

        if feed.happened(HappinessEventType::PathwayBlockedByLoanSigning) {
            out.push(
                NewsStory::new(NewsStoryKind::PathwayBlocked, date)
                    .about(player.id)
                    .with_numbers(player.age(date) as i32, 0)
                    .weighted(PlayerStanding::importance(player) / 2),
            );
            return;
        }

        if feed
            .any_of(&[
                HappinessEventType::ThreatenedByReturningLoanee,
                HappinessEventType::ThreatenedByNewSigning,
                HappinessEventType::LostStartingPlace,
            ])
            .is_some()
        {
            out.push(
                NewsStory::new(NewsStoryKind::ShirtUnderThreat, date)
                    .about(player.id)
                    .weighted(PlayerStanding::importance(player) / 2),
            );
            return;
        }

        // The winning half of the same argument. The paper has always
        // run "he has lost his place" and never "he has taken one",
        // which reads as though shirts are only ever surrendered.
        if feed.happened(HappinessEventType::WonStartingPlace) {
            out.push(
                NewsStory::new(NewsStoryKind::WonStartingPlace, date)
                    .about(player.id)
                    .with_numbers(player.age(date) as i32, 0)
                    .weighted(PlayerStanding::importance(player) / 3),
            );
        }
    }

    /// The contract column: a deal signed, and a deal running out.
    fn file_contract(
        out: &mut Vec<NewsStory>,
        player: &Player,
        feed: &RecentEvents<'_>,
        date: NaiveDate,
    ) {
        let Some(contract) = player.contract.as_ref() else {
            return;
        };
        if player.is_on_loan() {
            return;
        }

        let importance = PlayerStanding::importance(player);

        if feed.happened(HappinessEventType::ContractRenewal) {
            let years = ((contract.expiration - date).num_days() as f32 / 365.0)
                .round()
                .max(1.0) as i32;
            out.push(
                NewsStory::new(NewsStoryKind::ContractRenewed, date)
                    .about(player.id)
                    .with_numbers(years, 0)
                    .with_money(contract.salary as i64)
                    .weighted(importance),
            );
            return;
        }

        let days_left = (contract.expiration - date).num_days();
        if !(0..=210).contains(&days_left) {
            return;
        }

        // Only a player the club would miss makes this a story.
        if importance < 30 {
            return;
        }

        let months_left = ((days_left as f32) / 30.0).round().max(1.0) as i32;
        out.push(
            NewsStory::new(NewsStoryKind::ContractStandoff, date)
                .about(player.id)
                .with_numbers(months_left, 0)
                .weighted(importance - (months_left * 6)),
        );
    }
}
