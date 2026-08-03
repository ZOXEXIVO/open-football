use chrono::NaiveDate;
use std::collections::VecDeque;

/// Which desk filed a story. Rendered by the web layer as the standing
/// kicker above a headline — the same section furniture a real sports
/// page carries, and the only grouping a reader needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NewsDesk {
    /// Match reports and anything decided on the pitch.
    Match,
    /// Squad news: form, fitness, discipline, milestones, contracts.
    Squad,
    /// Marks out of ten. What one player did in one afternoon — the
    /// masterclass, the man of the match, the missed sitter, the error
    /// that cost it.
    ///
    /// Its own section rather than more squad news, for the reason a
    /// real paper separates them: a ratings column is written against
    /// the ninety minutes just played, while squad news is written
    /// against a career. Sharing an allowance would mean a striker's
    /// stinker competing with a testimonial, and one of them losing on
    /// priority every week.
    Verdicts,
    /// The transfer market — arrivals, departures, speculation.
    Market,
    /// The loan column: how the club's players away on loan are doing,
    /// and what they say about coming back.
    Loan,
    /// The terraces and the press box — what the people watching make
    /// of it all. A local paper has always carried this column, and
    /// without it a club's week reads as a set of results rather than
    /// as something happening to a town.
    Fans,
    /// Boardroom and balance sheet.
    Boardroom,
    /// The scoring charts. A club paper has no use for it — a division's
    /// leading marksmen are not one club's news — so nothing files here
    /// except the league's own monthly, where the charts are half the
    /// paper.
    Charts,
}

impl NewsDesk {
    /// Every desk that files copy. Walked by the locale tests.
    pub const ALL: [NewsDesk; 8] = [
        NewsDesk::Match,
        NewsDesk::Squad,
        NewsDesk::Verdicts,
        NewsDesk::Market,
        NewsDesk::Loan,
        NewsDesk::Fans,
        NewsDesk::Boardroom,
        NewsDesk::Charts,
    ];

    /// i18n key for the kicker label.
    pub fn i18n_key(self) -> &'static str {
        match self {
            NewsDesk::Match => "news_desk_match",
            NewsDesk::Squad => "news_desk_squad",
            NewsDesk::Verdicts => "news_desk_verdicts",
            NewsDesk::Market => "news_desk_market",
            NewsDesk::Loan => "news_desk_loan",
            NewsDesk::Fans => "news_desk_fans",
            NewsDesk::Boardroom => "news_desk_board",
            NewsDesk::Charts => "news_desk_charts",
        }
    }
}

/// Whether a story is a one-off event, a number that keeps moving, or a
/// condition that simply persists. Drives how long the editor waits
/// before letting the same theme back onto the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewsRecurrence {
    Event,
    Progress,
    Standing,
}

/// Everything the press can print about a club. Each variant is one
/// real recurring football story — the kind a local paper actually
/// leads on — and maps to exactly one headline / body pair in the
/// translation bundles (`news_h_<stem>` / `news_b_<stem>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewsStoryKind {
    // ── Match desk ────────────────────────────────────────────────
    LeagueWin,
    LeagueDraw,
    GoallessDraw,
    LeagueDefeat,
    Rout,
    HeavyDefeat,
    DerbyWin,
    DerbyDefeat,
    CupWin,
    CupExit,

    // ── Match desk: how it happened ───────────────────────────────
    //
    // The scoreline is only half a match report. A 2-1 won in the
    // eighty-ninth minute and a 2-1 won by half past three are the same
    // line in a table and completely different afternoons, and the
    // difference is the one thing every supporter in the ground
    // remembers. The engine has always stamped each goal with the
    // minute it was scored; these are the pieces that read them.
    //
    // At most one runs per match — the desk takes the biggest angle —
    // so a report and its sidebar never turn into four pieces about
    // ninety minutes.
    /// Won it with a goal in the last five minutes.
    LateWinner,
    /// …and won it after that, in time nobody had left on the clock.
    StoppageTimeDrama,
    /// Two behind and won anyway. The afternoon a season turns on.
    ComebackWin,
    /// The other side of the same coin, and the one a manager gets
    /// asked about: two ahead, and it finished level or worse.
    LeadThrownAway,
    /// Conceded, and had it back inside five minutes. The reply that
    /// tells a town the side has not gone under.
    InstantReply,
    /// Two goals inside the opening twenty minutes: the tie was over
    /// before the ground had settled.
    EarlyBlitz,
    /// Six goals or more between the two of them. Never mind who won.
    GoalFest,
    /// Won it with ten men.
    TenManWin,

    // ── Match desk: run-of-form and standings ─────────────────────
    WinningRun,
    UnbeatenRun,
    WinlessRun,
    TitleCharge,
    RelegationFight,
    /// Weeks without scoring. The drought a whole town can feel, as
    /// distinct from one striker's.
    GoalsDriedUp,
    /// Shipping goals every week. The other half of a bad run, and the
    /// half a paper blames somebody for.
    DefensiveCrisis,
    /// Nobody has won here in months.
    FortressHome,
    /// Winning on the road, which is the form that persuades a town its
    /// side is actually any good.
    AwayDayForm,

    // ── Match desk: European nights ───────────────────────────────
    //
    // Continental results have always reached club papers — they go
    // into the same global store the weekly gather walks, which is
    // why a European hat-trick was reported correctly. What was
    // missing was the label: every one of those midweeks was filed
    // as an ordinary league game, so a club could beat the
    // continent's best on a Wednesday and read about it as though
    // it had gone to a mid-table Saturday.
    /// A win under the lights against somebody from another
    /// country. The evenings a club measures its decade in.
    ContinentalNightWin,
    /// …and the other half of the same evening, which is usually a
    /// lesson about the gap rather than about the ninety minutes.
    ContinentalDefeat,
    /// Taking somebody apart in Europe. The night that gets
    /// replayed on a loop for a generation.
    ContinentalRout,
    /// …and being taken apart in it, in front of everybody, by a
    /// side that plays this football every year.
    ContinentalHiding,

    // ── Match desk: the playoffs ──────────────────────────────────
    //
    // A playoff runs through an inner league the weekly gather
    // already walks, so the football was always reported — as a
    // routine league Saturday, because nothing carried the stakes.
    // Read from the bracket's own series rather than from a cup
    // tie: a best-of-three sitting at one win each has had two
    // games this week and decided nothing.
    /// A playoff game won, with the series still open. Worth more
    /// than the same scoreline in April and for an obvious reason.
    PlayoffGameWin,
    /// …and one lost, with the series still alive. The week between
    /// two playoff games is the longest week in football.
    PlayoffGameDefeat,
    /// Through. The series is settled and the club is still in it.
    PlayoffTieWon,
    /// Out. A season that ran to May and ended in a fortnight.
    PlayoffTieLost,
    /// Through to the final. One match from everything the season
    /// was for, and the largest morning a club can have without
    /// having won anything at all.
    PlayoffFinalReached,

    // ── Squad desk ────────────────────────────────────────────────
    HatTrick,
    StarForm,
    RisingStar,

    // ── Squad desk: whether he is actually getting better ─────────
    //
    // Nothing in the game stored a historical ability, so nothing
    // outside the development engine could tell a breakthrough from
    // a plateau from a decline — the press could see what a player
    // is worth today and never whether that number had moved. All
    // four read a CURRENT-ability mark laid down a quarter of a
    // season ago. None of them reads potential, which stays hidden
    // from everything that is not the engine.
    /// A young player who has genuinely improved — not a good run of
    /// form, an actual step up that has held for months.
    BreakthroughSeason,
    /// The same thing later in a career, where it is rarer and
    /// therefore stranger: a player nobody expected to improve has.
    TrainingTransformation,
    /// A young player who has stopped getting better. The quietest
    /// bad news in football and the hardest to say out loud.
    StalledProspect,
    /// A player going backwards. It happens to all of them and it is
    /// still the hardest paragraph a local paper has to write.
    PowersFading,
    KeeperWall,
    /// The afternoon a goalkeeper kept his side in it on his own — the
    /// one position whose best day never shows up in a scoreline.
    KeeperMasterclass,
    /// He kept out a spot-kick, and the shoot-out turned on it.
    KeeperPenaltySave,
    /// Beaten repeatedly. Sometimes it is him and sometimes it is the
    /// ten in front of him, which is exactly the argument a town has.
    KeeperOverrun,
    /// The mistake that ended in his own net. A keeper's error is the
    /// only one on the pitch with no defender behind it.
    KeeperBlunder,
    /// A shut-out milestone: the number a goalkeeper's career is
    /// actually counted in.
    KeeperShutoutMilestone,
    /// The division's best goalkeeper over a season, by the one award
    /// that is only ever his.
    KeeperGoldenGlove,
    InjuryBlow,
    InjuryReturn,
    InjurySetback,
    RedCard,
    Suspension,
    YouthDebut,
    MilestoneApps,
    MilestoneGoals,
    PlayerOfMonth,
    TeamOfTheWeek,
    CaptainNamed,
    NationalCallUp,

    /// One of the club's players has won a tournament with his
    /// country. The one thing that happens to a footballer which his
    /// club had no hand in and every one of its supporters claims a
    /// share of anyway.
    TournamentTriumph,
    /// …and the other end of the same summer: a month of football
    /// and a final lost, and a man due back at training in a fortnight.
    TournamentHeartbreak,
    GoalDrought,
    DroughtEnded,
    FormerClubGoal,
    ClubServant,
    RetirementAnnounced,
    TrainingBustUp,
    ContractRenewed,
    /// Off the mark for his new club. The one goal a signing is asked
    /// about at every press conference until he scores it.
    FirstClubGoal,
    /// He has taken the shirt off somebody. The quiet half of every
    /// selection story the paper has only ever printed the loud half of.
    WonStartingPlace,
    /// A voice in the dressing room where there was not one before.
    LeaderEmerging,
    /// The foreigner is settling: the language is coming, and somebody
    /// he can speak to has walked through the door.
    SettlingIn,
    /// A friend or a mentor has gone, and the man left behind feels it.
    /// Squad news, not transfer news — the deal was somebody else's.
    TeammateFarewell,
    /// He has stood down from the international side while carrying on
    /// at his club. A career decision with a date on it.
    InternationalRetirement,
    /// The veteran has started looking at the other side of the white
    /// line. Half the coaches in the game gave this interview once.
    CoachingAmbition,
    /// The armband taken off him — the other half of a story the paper
    /// has always printed only the happy end of.
    CaptaincyLost,
    /// Somebody has come for his place: a signing, or the kid back from
    /// a loan he owned.
    ShirtUnderThreat,
    /// The academy product who cannot get past a borrowed stopgap of
    /// his own level. The story a local paper exists to run.
    PathwayBlocked,
    /// A career visibly closing — the veteran weighing up whether this
    /// is the last one.
    CareerTwilight,

    // ── Squad desk: the foreigner's life ──────────────────────────
    /// He wants to go home. The one thing a foreign signing's form
    /// never explains and every dressing room knows about first.
    HomesickAbroad,
    /// A year in and still on his own: the language has not come, the
    /// city has not opened up, and it is showing on Saturdays.
    StrugglingToSettle,
    /// A club back in his own country has come in for him, and this
    /// time it is not a rumour he can dismiss.
    HomeCalling,
    /// He has stopped looking like a man in the wrong country.
    SettledAtLast,
    /// Signed from the neighbours, and the dressing room has not
    /// forgotten which shirt he used to wear.
    ColdShoulder,

    // ── Squad desk: the room itself ───────────────────────────────
    /// Two of them went at it, and it was not about football.
    TeammateConflict,

    // ── Squad desk: the same beats, told specifically ─────────────
    //
    // The dressing room has always recorded WHY. The page printed
    // one flat sentence for each of them — "two of them went at
    // it", "he was left out", "he has retired", "he is back from
    // loan", "he is not on the list", "he is injured" — which is
    // true of every version and useful about none. These read the
    // reason back off the event's own context.
    /// A row about how seriously the other man trains. The one
    /// argument in a dressing room that is genuinely about football.
    TrainingStandardsRow,
    /// Two men after the same shirt, and it has stopped being
    /// professional.
    PositionRivalryFeud,
    /// A senior figure pulled rank and it did not land. The row
    /// that decides who the dressing room actually belongs to.
    LeadershipPowerStruggle,
    /// Left out to be looked after — rotation, a cup night, a leg
    /// that needed the week. Reported quietly, because it is not a
    /// grievance and the page should not invent one.
    RotationRested,
    /// Left out because the shape wanted somebody else's profile.
    /// No reflection on him and no comfort in it either.
    TacticalOmission,
    /// Left out because somebody else has been better. The only
    /// version of an omission a player cannot argue with and the one
    /// he takes worst.
    DroppedOnForm,
    /// Left out as a punishment rather than as a selection. Nobody
    /// at the club will use the word, and everybody will understand
    /// it.
    DisciplinaryOmission,
    /// A career ended by an injury rather than by a decision. It is
    /// the same squad-list line as a planned farewell and nothing
    /// alike on a page.
    ForcedToRetire,
    /// Home from a loan that did exactly what it was sent to do.
    LoanReturnTriumph,
    /// …and home from one that did not: a season of somebody
    /// else's bench, or a real run of games he was visibly not up to.
    LoanReturnWasted,
    /// Not registered because the homegrown slots were spent. An
    /// administrative sentence that is really a story about how the
    /// club has been run.
    HomegrownQuotaOmission,
    /// …and the same page from the other end: a foreign player left
    /// off because the registration slots ran out.
    ForeignQuotaOmission,
    /// A muscle. The injury that keeps recurring, keeps being rushed
    /// back from, and quietly costs more seasons than the dramatic
    /// ones.
    HamstringBlow,
    /// A knee. The words a physio says slowly and a supporter
    /// recognises immediately.
    KneeLigamentBlow,
    /// A break. Brutal, unambiguous, and usually less career-shaped
    /// than the ligament everybody fears more.
    BrokenBoneBlow,
    /// The senior pro who has taken the new boy on — the quiet half of
    /// a dressing room, and the half that decides whether a signing
    /// works.
    TakenUnderWing,
    /// Somebody stood up in the dressing room and said it out loud.
    DressingRoomSpeech,

    // ── Squad desk: the room itself, and what the coach thinks ────
    //
    // Two things the simulation has always known and the press
    // could not see: how well a dressing room has actually knitted
    // together, and what the man who picks the team privately
    // believes about each player. Both are refreshed weekly, so
    // both are Standing — a condition, never a day.
    /// The dressing room has come together — the thing every manager
    /// claims to be building and almost nobody can point at.
    SquadKnitsTogether,
    /// Too many new faces at once. A squad rebuilt in one window
    /// pays for it in a currency nobody budgets for.
    TurnoverToll,
    /// The room has split into groups that do not much like each
    /// other. The quiet version of a dressing-room problem, and the
    /// one that lasts longest.
    CliqueConcerns,
    /// The manager has decided he can be relied on when it matters.
    /// Never stated publicly and visible in every team sheet.
    BigMatchTrust,
    /// …and the other half: a mistake the manager has not finished
    /// forgetting, whatever he says in a press conference.
    ManagerDoubtsLinger,

    // ── Squad desk: the training ground and the bench ─────────────
    /// The reports out of the training ground are good — the week's
    /// quietest story and the one that most often precedes a run in
    /// the side.
    TrainingGroundBuzz,

    /// The manager has torn up the shape and kept the new one.
    /// Nothing is ever announced; the team sheet simply stops looking
    /// like it did, and everybody notices in the same week.
    FormationRevolution,
    /// …and the version nobody wants written about them.
    TrainingConcerns,
    /// He is sick of the bench and has stopped hiding it.
    BenchFrustration,
    /// His country has stopped picking him.
    DroppedByCountry,
    /// Left off the registered list altogether: not injured, not
    /// dropped — ineligible, which is worse and takes longer to fix.
    LeftOutOfSquadList,
    /// Off-field trouble. Every paper's favourite story and every
    /// manager's least.
    OffFieldControversy,
    /// He has seen what the man next to him earns.
    WageEnvy,
    /// The number nine shirt changes hands. A small thing everywhere
    /// except in the town it happens in.
    ShirtNumberHandover,
    /// He has outgrown the division and everybody can see it.
    OutgrownDivision,
    /// The dressing room has started doing the arithmetic.
    RelegationNerves,
    /// The deal torn up before its date — by agreement, or otherwise.
    ContractTornUp,
    /// The season's individual verdict: player of the year, a place in
    /// the team of the season, a nomination among the best in the
    /// world.
    SeasonAward,

    // ── Squad desk: what a move, an award and a new manager
    //    actually do to a player ────────────────────────────────
    //
    // Every one of these is a beat the dressing room has always
    // recorded and the press had no way to print. The paper knew a
    // player had signed and never what the move meant to him; it
    // reported a manager being sacked and never what the squad made
    // of it; it printed one line for every award in football.
    /// The move he had always wanted, and the club he had always
    /// wanted it to be.
    DreamMoveComplete,
    /// He arrived as somebody and discovered he is somebody else
    /// here. The quiet violence of moving up a level.
    StatusShock,
    /// The contract changed his life, and everybody in the dressing
    /// room worked out roughly by how much.
    PayWindfall,
    /// …and the other direction: he moved and took less, which is a
    /// decision a supporter reads as either loyalty or desperation.
    WageRealityCheck,
    /// The division's player of the week.
    PlayerOfWeek,
    /// …and its best young one.
    YoungPlayerOfWeek,
    /// Named in the division's team of the month — the honour that
    /// says he was not just good on one afternoon.
    TeamOfMonthNod,
    /// The division's best young player over a month. Its own piece
    /// rather than a line in the senior award's: a nineteen-year-old
    /// beating other nineteen-year-olds is a different story from a
    /// twenty-eight-year-old beating everybody.
    YoungPlayerOfMonthAward,
    /// …and over a whole season, which is the one that gets
    /// mentioned for the rest of his career.
    YoungPlayerOfSeasonAward,
    /// A new man in the dugout and everybody suddenly training like
    /// they mean it. The oldest phenomenon in the sport.
    NewManagerBounce,
    /// The other half of a dugout change, which the paper has always
    /// reported without: what it did to the players who were signed by
    /// the man who has gone.
    ManagerExitUnsettles,
    /// The club said it would and it did. Half of a story the paper
    /// has only ever printed the unhappy end of.
    PromiseKept,
    /// The suspension is over and he is available again — the news
    /// a manager actually cares about, and the half of a ban that
    /// never got printed.
    BanServed,
    /// Playing for his future in the most literal sense: a deal
    /// running down and every performance a negotiation.
    PlayingForContract,
    /// The coaches have put him on a programme of his own. Filler on
    /// a busy week and genuinely interesting on a quiet one.
    PersonalTrainingPlan,
    /// The manager is teaching him a different job. Most careers
    /// have one of these in them and it is rarely reported until it
    /// has already worked.
    RoleRetraining,
    /// One of the club's own, kept out by somebody brought in. Its
    /// own piece rather than a line in the blocked-pathway story: the
    /// grievance here is about where the two of them are from.
    HomegrownBlocked,
    /// He thinks the manager has favourites and that he is not one.
    /// A different complaint from being left out of one big match,
    /// and a more corrosive one.
    FavouritismGrumbles,

    // ── Squad desk: the life around the football ──────────────────
    //
    // A footballer is a man with a family, a language he may not
    // speak and a life that does not stop for the fixture list. The
    // simulation has always modelled that; the press could only ever
    // see that *something* had happened away from the pitch, because
    // the whole of it arrives on one event type. Reading the kind
    // back is what turns a shrug into a sentence.
    /// The family has not settled: a new country, a language nobody
    /// at home speaks, and a house that does not feel like one.
    FamilyUnsettled,
    /// A birth in the family. The one week of a footballer's year
    /// that has nothing at all to do with football.
    FamilyCelebration,
    /// A bereavement. Handled the way a local paper handles one:
    /// early, briefly, and without a single cheerful word in it.
    CompassionateLeave,
    /// He has asked for help with the language, which is the least
    /// glamorous and most decisive thing a foreign signing ever does.
    LanguageLessons,
    /// A veteran wants one last season at the club he grew up at.
    /// Sentimental, entirely real, and the beginning of an exit
    /// nobody has announced yet.
    VeteranHomecomingWish,
    /// A long-serving player turning down the chance to go. The
    /// loyalty story a town tells about itself.
    LegendWontLeave,
    /// He turned down a move to a rival that would have been a step
    /// up in every measurable way, and took the shirt he already had.
    RefusesRivalMove,
    /// He wants out of the noise — a smaller club, fewer cameras,
    /// and a crowd that does not turn on people. The ambition story
    /// running the other way.
    SeeksQuieterStage,

    // ── Verdicts desk: one player, ninety minutes ─────────────────
    /// The afternoon's outstanding player, as the match itself
    /// recorded him.
    ManOfTheMatch,
    /// A performance the paper marks in the eights — the game a
    /// supporter brings up for a decade.
    MatchMasterclass,
    /// Three or more made for other people in one game. The rarest
    /// line on a ratings page, and never once printed before.
    AssistShow,
    /// He ran the game without scoring in it: the passes that opened
    /// it up, whoever eventually applied the finish.
    CreatorInChief,
    /// A defender's masterclass — headers, blocks, tackles, and
    /// nothing behind him. The performance that never shows up in a
    /// scoreline.
    DefensiveRock,
    /// Nobody could get near him.
    DribblingDisplay,
    /// Off the bench and decisive. Half an hour that changed the
    /// afternoon.
    SuperSub,
    /// The man who won a derby. A different story from winning a
    /// match, and the one a town keeps.
    DerbyHero,
    /// A performance the paper marks down. The other half of a ratings
    /// column, and the half that sells it.
    MatchStinker,
    /// The chances were there and he put them everywhere but in. The
    /// single most-argued-about afternoon in football.
    WastefulFinishing,
    /// His mistake, their goal. An outfield error has ten men and a
    /// goalkeeper behind it and still ended up in the net.
    CostlyError,
    /// Into his own goal. Nobody's fault and entirely his, which is
    /// exactly why it prints.
    OwnGoalShame,
    /// He missed from twelve yards when it was his to settle.
    PenaltyMissed,
    /// Taken off before the hour with the game still there to be
    /// played. A manager's verdict delivered in public.
    HookedEarly,
    /// He spent the afternoon fouling people, and the referee ran out
    /// of patience.
    FoulTrouble,

    // ── Verdicts desk: the afternoons the column had no line for ──
    //
    // The ratings page could mark a man out of ten and say he was
    // outstanding, poor, wasteful or at fault. It could not say he
    // scored twice, that he was seventeen, that it was his first
    // afternoon in the shirt, or that a centre-half had gone up for
    // a corner and won the match — which are the afternoons a
    // supporter actually retells.
    /// Two goals in one afternoon. The hat-trick's quieter and far
    /// more common sibling, and the column had no line for it.
    BraceHero,
    /// A goal and a hand in another. Involvement in two of them is
    /// a different afternoon from scoring one.
    GoalAndAssistShow,
    /// A teenager who did not look like one. The same mark means
    /// something different at nineteen, and a paper that prints the
    /// same sentence for both has not looked at the team sheet.
    TeenageStarTurn,
    /// The other end of the same idea: a veteran with an afternoon
    /// nobody expected he still had in him.
    RolledBackYears,
    /// A debut that went the way he will have dreamt it. There is
    /// exactly one of these per career.
    DreamDebut,
    /// …and the version nobody dreams. Reported because a paper
    /// that only prints the good debuts is not reporting debuts.
    DebutNightmare,
    /// A defender on the scoresheet. Rare enough that the whole
    /// ground remembers who took the corner.
    GoalFromDefence,
    /// He did both jobs: won it back and then made something of it.
    /// The shift that decides matches and never shows up in a
    /// scoreline or a goal feed.
    MidfieldEngine,

    // ── Squad desk: the manager and his players ───────────────────
    ManagerBacksPlayer,
    ManagerCallsOutPlayer,
    DroppedForBigMatch,
    PromiseBroken,
    PlayerFined,
    ClearTheAir,
    /// Played out of position, or hooked once too often.
    RoleFrustration,

    // ── Market desk ───────────────────────────────────────────────
    NewSigning,
    RecordSigning,
    FreeSigning,
    LoanArrival,
    /// A player the club once owned walks back through the door. The
    /// story a town prints above any ordinary signing of the same size.
    HomecomingSigning,
    /// A loanee the club decided to keep: the option exercised, the
    /// temporary shirt made his own.
    LoanMadePermanent,
    /// A teenager signed for the years ahead rather than for Saturday.
    ProspectSigned,
    /// Thirty-something through the door — experience, and a dressing
    /// room that just got older and calmer at once.
    VeteranArrives,

    // ── Market desk: why the club actually did it ─────────────────
    //
    // Every completed deal has always recorded its motive, and the
    // market desk had never opened one — so a succession plan, a
    // panic buy, a scouting department's own find and a raid on the
    // neighbours all reached the page as "the club have signed a
    // player". These are the same business reported the way a paper
    // with a source inside the club would report it.
    /// Taken from a rival. The signing a town enjoys twice.
    RivalRaid,
    /// The scouting department pushed for him and got him.
    ScoutingCoup,
    /// Signed to succeed somebody who is still here — the most
    /// awkward and most necessary kind of business a club does.
    SuccessionSigning,
    /// Cover. Not glamorous, and the reason seasons do not fall apart
    /// in February.
    DepthSigning,
    /// A hole in the shape, filled.
    GapPlugged,
    /// Straightforwardly better than what was there, and paid for.
    MarqueeUpgrade,
    /// Cheap, and the cheapness is the story.
    BargainBuy,
    /// One of the club's own, promoted out of the academy.
    AcademyGraduate,
    /// Somebody wanted him badly enough to borrow him and play him,
    /// which from the parent club's paper is a lad sent out to grow.
    LoanedOutToGrow,

    PlayerSold,
    StarSold,
    FreeExit,
    LoanExit,
    LoanReturn,
    TransferSpeculation,
    TransferListed,
    ContractStandoff,
    /// A deal is agreed and everybody knows it: the fee settled, the
    /// medical booked, the shirt number already argued about.
    TransferAgreed,
    /// The link that quietly died — the suitor moved on without a bid,
    /// and the paper closes the story it spent a month feeding.
    RumourCools,
    /// The player answers the speculation himself: he is going nowhere,
    /// and he says so in his own words.
    CommitsToClub,
    /// Final year, no offer, no talks — just silence from upstairs, and
    /// a supporter who can read a calendar.
    ContractRunningDown,
    // Rumour mill: the same saga a supporter follows all summer.
    RumourInterest,
    RumourRival,
    ScoutsWatching,
    AgentTouting,
    HomecomingLink,
    BidRejected,
    TalksExpected,
    TransferRequestFiled,
    ToldNotInPlans,
    ContractTalksStalled,
    /// The move he had set his heart on fell through.
    MoveCollapsed,
    /// He wants more than the club can offer — a stronger squad, a title
    /// race, a bigger league, or out of the division it just dropped to.
    AmbitionWarning,
    /// The window shut with him still on the list and still on the books.
    UnsoldStillHere,
    // The verdict on business already done.
    SigningNotWorking,
    SigningComesGood,

    // ── Market desk: leverage and ambition ────────────────────────
    /// He used somebody else's interest to get a better contract out
    /// of this club, and everybody involved knows it.
    LeverageUsed,
    /// He wants continental football, and has stopped being coy
    /// about whether this club can give it to him.
    ContinentalAmbition,

    // ── Loan column ───────────────────────────────────────────────
    LoanWatchStarter,
    LoanWatchGoals,
    LoanWatchBenched,
    LoanWantsReturn,
    LoanWantsPermanent,
    /// He has had enough of being lent out and wants a pre-season at
    /// the club that owns him.
    LoanFedUp,
    LoanRecallTalk,
    LoanSpellEnds,
    LoanFlop,
    LoanStepTooBig,

    // ── The terraces and the press box ────────────────────────────
    FansChant,
    FansTurnOnTeam,
    FansGetBehind,
    FansAngryAtRumour,
    MediaPressure,
    MediaDarling,

    // ── Fans desk: the town, not the dressing room ────────────────
    //
    // The terraces column could only see what individual players
    // felt about the crowd, which is the wrong way round. A
    // supporter's week is made of the table, the transfer business
    // and how the cup tie went, and none of it reached the one desk
    // whose subject is the people watching.
    /// The anger has stopped being a mood and started being
    /// something happening outside the ground.
    ProtestBrewing,
    /// The table says top two and the ground has started to believe
    /// it, which are two different things and only interesting
    /// together.
    PromotionFever,
    /// …and the version nobody wants: the arithmetic is bad and the
    /// ground has done it.
    RelegationDread,
    /// The club has sold somebody the terraces had adopted, and the
    /// terraces have views about it.
    FansFurySale,
    /// …and the morning it goes the other way, when the club spends
    /// money on somebody the town actually wanted.
    FansDreamSigning,
    /// Not a cup exit — a hiding in one. Nobody rings a phone-in
    /// about losing a tie narrowly.
    CupHumiliationFallout,
    /// The people who paid twice — for the ticket and for the
    /// journey — got something back for it.
    TravellingSupportRewarded,
    /// The crowd has taken to one of its own. A different affection
    /// from the one a signing earns, and the only kind a local paper
    /// genuinely owns.
    AcademyDarling,

    // ── Boardroom ─────────────────────────────────────────────────
    ManagerPressure,
    BoardBacking,
    ManagerSacked,
    NewManagerArrives,
    /// A bigger club came for him and paid to take him. Not a sacking,
    /// and a town reads the two very differently.
    ManagerPoached,
    /// Somebody has to pick the team on Saturday. The interim spell the
    /// paper used to report as a permanent appointment.
    CaretakerTakesCharge,
    /// The stand-in kept the job. The club looked outside, found
    /// nothing better, and gave it to the man who was already there.
    CaretakerConfirmed,
    /// The vacancy itself: the seat is empty, the names are doing the
    /// rounds, and nobody has signed anything.
    ManagerHunt,
    /// The club has moved for somebody else's manager.
    ManagerTargetLinked,
    /// Somebody has moved for ours.
    ManagerWanted,
    /// The board went public with the final warning. The rung of the
    /// ladder a supporter actually gets to watch.
    ManagerUltimatum,
    /// A new deal for the man in the dugout.
    ManagerContractExtended,
    /// Somebody wants to buy the club.
    TakeoverRumour,
    /// Somebody did. The single loudest thing that can happen to a club
    /// without a ball being kicked.
    TakeoverCompleted,
    /// The sale fell over, and everything it was going to pay for went
    /// with it.
    TakeoverCollapsed,
    /// The ground itself gets bigger.
    StadiumExpansion,
    /// Money spent where the supporters cannot see it — training
    /// pitches, the academy building, the scouting network.
    FacilityUpgrade,
    /// Money on the table for the manager to spend.
    WarChest,
    /// Money taken back off it.
    BudgetCut,
    /// The board said there would be something, and there was not.
    BoardPromiseBroken,
    DressingRoomInquest,
    SquadRallies,
    BoardInvests,
    TrophyWon,
    /// Promotion confirmed — the biggest thing that can happen to a
    /// club this size, and the edition supporters keep.
    PromotionWon,
    /// The drop confirmed. The other edition supporters keep.
    RelegationConfirmed,
    /// A cup final reached and lost — the season that nearly was.
    CupFinalHeartbreak,
    /// Continental football secured for next season.
    EuropeSecured,
    AcademyPraise,

    /// Nobody left to promote. A club whose coaching staff has
    /// emptied out entirely and has had to put a stranger in charge of
    /// training, which is a story about the institution rather than
    /// about a dugout.
    BackroomEmpty,

    // ── Boardroom: the academy's own calendar ─────────────────────
    //
    // The pathway had exactly one story — a standing piece about
    // the academy's reputation — and none about the two mornings a
    // year it actually has: the day boys come in, and the day boys
    // are handed up. Both leave nothing behind but a longer squad
    // list, which is why they were never printed.
    /// The academy's intake: a year group signed on one morning,
    /// none of whom anybody will see for three years.
    IntakeDay,
    /// An intake the recruitment department privately thinks is
    /// special. Their verdict, not a reading of anybody's ceiling —
    /// and the phrase every club regrets using within five years.
    GoldenGeneration,
    /// The other end of the pipeline: a year group handed up into
    /// the senior squads on the same morning.
    GraduationDay,
    MoneyWorries,

    // ── Boardroom: the balance sheet in trouble ───────────────────
    /// The club has gone into administration: a points deduction, an
    /// embargo, and the debt written down to something it can service.
    /// The loudest thing a set of accounts can do to a football club.
    AdministrationEntered,
    /// Out the other side of it, a year later.
    AdministrationExited,
    /// The owner covered a shortfall the club could not. Money that
    /// keeps the lights on rather than money that buys a centre-half.
    OwnerBailout,
    /// Commercial news: a shirt or partner deal signed, with the annual
    /// value on it.
    SponsorSigned,
    /// …and the other half of the same column, which papers print far
    /// less often: deals expired and nobody replaced them.
    SponsorshipLost,
    /// A clause written into an old sale has paid out: the club
    /// banks money for a player it no longer owns.
    SellOnWindfall,
    /// …and the version of it that arrives because somebody else got
    /// promoted, which is a supporter's favourite kind of money.
    PromotionBonusDue,
    /// A Bosman agreed months in advance: a signing that will cost
    /// nothing, told from the paper of the club that is getting him.
    PreContractAgreed,
    /// …and the same morning told from the other end, where a player
    /// the side still picks every week has agreed to leave for free.
    BosmanDepartureLooms,
    /// Borrowing past the agreed facility. The number a supporter
    /// quotes at every phone-in for the next five years.
    DebtMountain,
    /// The club may not spend a fee on a player at all.
    TransferEmbargo,
    /// Wages have outrun what comes through the door.
    WageBillCrisis,
    /// Nobody arrives until somebody leaves. The market consequence of
    /// a balance sheet, and the version of it the rumour mill prints.
    MustSellBeforeBuying,

    // ── Boardroom: the decisions that used to be discarded ────────
    //
    // The board really takes these — a crisis meeting, an order to
    // sell, a veto on a signing the manager wanted, a training-ground
    // plan refused — and every one of them was matched and thrown
    // away at the decision site, so none of them reached a page.
    // They are the reason a manager's week goes the way it does, and
    // a supporter always hears about them from somebody.
    /// The board called everybody in. Not a decision — the sound a
    /// boardroom makes immediately before one.
    CrisisTalks,
    /// Somebody has to go, and the instruction came from upstairs
    /// rather than from the dugout.
    BoardDemandsSale,
    /// A signing the manager wanted, vetoed above his head. The
    /// half of a transfer window nobody announces.
    BoardBlocksDeal,
    /// The money for the training ground was asked for and refused.
    /// The quietest way a club tells you what its ambitions are.
    FacilityPlanRejected,

    // ── Charts desk: the division's month ─────────────────────────
    /// The month's leading marksman in the division. A club paper can
    /// only ever say a player scored; the league's own paper is the one
    /// that can say nobody scored more.
    LeagueTopScorer,
    /// The men behind him on the same chart. Repeats within one edition
    /// — a scoring chart is a list, and a list of one is a result.
    LeagueScoringChase,

    // ── Charts desk: the rest of the month ────────────────────────
    //
    // The monthly awards snapshot has always carried a player of
    // the month, a best young player, an assists chart, a ratings
    // chart and a team of the month, frozen and ready. The league's
    // own paper read exactly one field of it — the scorers — and
    // printed the same two stories every month for the life of a
    // save.
    /// The division's player of the month, told from the division's
    /// own paper — where it is a verdict rather than a club's news.
    LeaguePlayerOfMonth,
    /// …and its best young player, which is the award the rest of
    /// the division reads as a transfer rumour.
    LeagueYoungStar,
    /// The month's leading provider. The chart that decides who the
    /// scorers should be thanking.
    LeagueAssistKing,
    /// The men behind him on it. Repeats within one edition for the
    /// same reason the scoring chase does: a chart is a list.
    LeagueAssistChase,
    /// Best marked in the division over a month — the chart that
    /// catches the players a goal tally never will.
    LeagueRatingsLeader,
    /// Named in the division's team of the month. Repeats: the
    /// column is a list of names and printing one of them is not a
    /// team of the month.
    LeagueTeamOfMonth,
}

impl NewsStoryKind {
    /// Every kind the presses can set. The web layer walks this to prove
    /// each one has a headline and a body in every translation bundle,
    /// so adding a variant without its copy fails a test rather than
    /// printing a raw key on the front page.
    pub const ALL: [NewsStoryKind; 308] = [
        NewsStoryKind::LeagueWin,
        NewsStoryKind::LeagueDraw,
        NewsStoryKind::GoallessDraw,
        NewsStoryKind::LeagueDefeat,
        NewsStoryKind::Rout,
        NewsStoryKind::HeavyDefeat,
        NewsStoryKind::DerbyWin,
        NewsStoryKind::DerbyDefeat,
        NewsStoryKind::CupWin,
        NewsStoryKind::CupExit,
        NewsStoryKind::LateWinner,
        NewsStoryKind::StoppageTimeDrama,
        NewsStoryKind::ComebackWin,
        NewsStoryKind::LeadThrownAway,
        NewsStoryKind::InstantReply,
        NewsStoryKind::EarlyBlitz,
        NewsStoryKind::GoalFest,
        NewsStoryKind::TenManWin,
        NewsStoryKind::WinningRun,
        NewsStoryKind::UnbeatenRun,
        NewsStoryKind::WinlessRun,
        NewsStoryKind::TitleCharge,
        NewsStoryKind::RelegationFight,
        NewsStoryKind::GoalsDriedUp,
        NewsStoryKind::DefensiveCrisis,
        NewsStoryKind::FortressHome,
        NewsStoryKind::AwayDayForm,
        NewsStoryKind::ContinentalNightWin,
        NewsStoryKind::ContinentalDefeat,
        NewsStoryKind::ContinentalRout,
        NewsStoryKind::ContinentalHiding,
        NewsStoryKind::PlayoffGameWin,
        NewsStoryKind::PlayoffGameDefeat,
        NewsStoryKind::PlayoffTieWon,
        NewsStoryKind::PlayoffTieLost,
        NewsStoryKind::PlayoffFinalReached,
        NewsStoryKind::HatTrick,
        NewsStoryKind::StarForm,
        NewsStoryKind::RisingStar,
        NewsStoryKind::BreakthroughSeason,
        NewsStoryKind::TrainingTransformation,
        NewsStoryKind::StalledProspect,
        NewsStoryKind::PowersFading,
        NewsStoryKind::KeeperWall,
        NewsStoryKind::KeeperMasterclass,
        NewsStoryKind::KeeperPenaltySave,
        NewsStoryKind::KeeperOverrun,
        NewsStoryKind::KeeperBlunder,
        NewsStoryKind::KeeperShutoutMilestone,
        NewsStoryKind::KeeperGoldenGlove,
        NewsStoryKind::InjuryBlow,
        NewsStoryKind::InjuryReturn,
        NewsStoryKind::InjurySetback,
        NewsStoryKind::RedCard,
        NewsStoryKind::Suspension,
        NewsStoryKind::YouthDebut,
        NewsStoryKind::MilestoneApps,
        NewsStoryKind::MilestoneGoals,
        NewsStoryKind::PlayerOfMonth,
        NewsStoryKind::TeamOfTheWeek,
        NewsStoryKind::CaptainNamed,
        NewsStoryKind::NationalCallUp,
        NewsStoryKind::TournamentTriumph,
        NewsStoryKind::TournamentHeartbreak,
        NewsStoryKind::GoalDrought,
        NewsStoryKind::DroughtEnded,
        NewsStoryKind::FormerClubGoal,
        NewsStoryKind::ClubServant,
        NewsStoryKind::RetirementAnnounced,
        NewsStoryKind::TrainingBustUp,
        NewsStoryKind::ContractRenewed,
        NewsStoryKind::FirstClubGoal,
        NewsStoryKind::WonStartingPlace,
        NewsStoryKind::LeaderEmerging,
        NewsStoryKind::SettlingIn,
        NewsStoryKind::TeammateFarewell,
        NewsStoryKind::InternationalRetirement,
        NewsStoryKind::CoachingAmbition,
        NewsStoryKind::CaptaincyLost,
        NewsStoryKind::ShirtUnderThreat,
        NewsStoryKind::PathwayBlocked,
        NewsStoryKind::CareerTwilight,
        NewsStoryKind::HomesickAbroad,
        NewsStoryKind::StrugglingToSettle,
        NewsStoryKind::HomeCalling,
        NewsStoryKind::SettledAtLast,
        NewsStoryKind::ColdShoulder,
        NewsStoryKind::TeammateConflict,
        NewsStoryKind::TrainingStandardsRow,
        NewsStoryKind::PositionRivalryFeud,
        NewsStoryKind::LeadershipPowerStruggle,
        NewsStoryKind::RotationRested,
        NewsStoryKind::TacticalOmission,
        NewsStoryKind::DroppedOnForm,
        NewsStoryKind::DisciplinaryOmission,
        NewsStoryKind::ForcedToRetire,
        NewsStoryKind::LoanReturnTriumph,
        NewsStoryKind::LoanReturnWasted,
        NewsStoryKind::HomegrownQuotaOmission,
        NewsStoryKind::ForeignQuotaOmission,
        NewsStoryKind::HamstringBlow,
        NewsStoryKind::KneeLigamentBlow,
        NewsStoryKind::BrokenBoneBlow,
        NewsStoryKind::TakenUnderWing,
        NewsStoryKind::DressingRoomSpeech,
        NewsStoryKind::SquadKnitsTogether,
        NewsStoryKind::TurnoverToll,
        NewsStoryKind::CliqueConcerns,
        NewsStoryKind::BigMatchTrust,
        NewsStoryKind::ManagerDoubtsLinger,
        NewsStoryKind::TrainingGroundBuzz,
        NewsStoryKind::FormationRevolution,
        NewsStoryKind::TrainingConcerns,
        NewsStoryKind::BenchFrustration,
        NewsStoryKind::DroppedByCountry,
        NewsStoryKind::LeftOutOfSquadList,
        NewsStoryKind::OffFieldControversy,
        NewsStoryKind::WageEnvy,
        NewsStoryKind::ShirtNumberHandover,
        NewsStoryKind::OutgrownDivision,
        NewsStoryKind::RelegationNerves,
        NewsStoryKind::ContractTornUp,
        NewsStoryKind::SeasonAward,
        NewsStoryKind::DreamMoveComplete,
        NewsStoryKind::StatusShock,
        NewsStoryKind::PayWindfall,
        NewsStoryKind::WageRealityCheck,
        NewsStoryKind::PlayerOfWeek,
        NewsStoryKind::YoungPlayerOfWeek,
        NewsStoryKind::TeamOfMonthNod,
        NewsStoryKind::YoungPlayerOfMonthAward,
        NewsStoryKind::YoungPlayerOfSeasonAward,
        NewsStoryKind::NewManagerBounce,
        NewsStoryKind::ManagerExitUnsettles,
        NewsStoryKind::PromiseKept,
        NewsStoryKind::BanServed,
        NewsStoryKind::PlayingForContract,
        NewsStoryKind::PersonalTrainingPlan,
        NewsStoryKind::RoleRetraining,
        NewsStoryKind::HomegrownBlocked,
        NewsStoryKind::FavouritismGrumbles,
        NewsStoryKind::FamilyUnsettled,
        NewsStoryKind::FamilyCelebration,
        NewsStoryKind::CompassionateLeave,
        NewsStoryKind::LanguageLessons,
        NewsStoryKind::VeteranHomecomingWish,
        NewsStoryKind::LegendWontLeave,
        NewsStoryKind::RefusesRivalMove,
        NewsStoryKind::SeeksQuieterStage,
        NewsStoryKind::ManOfTheMatch,
        NewsStoryKind::MatchMasterclass,
        NewsStoryKind::AssistShow,
        NewsStoryKind::CreatorInChief,
        NewsStoryKind::DefensiveRock,
        NewsStoryKind::DribblingDisplay,
        NewsStoryKind::SuperSub,
        NewsStoryKind::DerbyHero,
        NewsStoryKind::MatchStinker,
        NewsStoryKind::WastefulFinishing,
        NewsStoryKind::CostlyError,
        NewsStoryKind::OwnGoalShame,
        NewsStoryKind::PenaltyMissed,
        NewsStoryKind::HookedEarly,
        NewsStoryKind::FoulTrouble,
        NewsStoryKind::BraceHero,
        NewsStoryKind::GoalAndAssistShow,
        NewsStoryKind::TeenageStarTurn,
        NewsStoryKind::RolledBackYears,
        NewsStoryKind::DreamDebut,
        NewsStoryKind::DebutNightmare,
        NewsStoryKind::GoalFromDefence,
        NewsStoryKind::MidfieldEngine,
        NewsStoryKind::ManagerBacksPlayer,
        NewsStoryKind::ManagerCallsOutPlayer,
        NewsStoryKind::DroppedForBigMatch,
        NewsStoryKind::PromiseBroken,
        NewsStoryKind::PlayerFined,
        NewsStoryKind::ClearTheAir,
        NewsStoryKind::RoleFrustration,
        NewsStoryKind::NewSigning,
        NewsStoryKind::RecordSigning,
        NewsStoryKind::FreeSigning,
        NewsStoryKind::LoanArrival,
        NewsStoryKind::HomecomingSigning,
        NewsStoryKind::LoanMadePermanent,
        NewsStoryKind::ProspectSigned,
        NewsStoryKind::VeteranArrives,
        NewsStoryKind::RivalRaid,
        NewsStoryKind::ScoutingCoup,
        NewsStoryKind::SuccessionSigning,
        NewsStoryKind::DepthSigning,
        NewsStoryKind::GapPlugged,
        NewsStoryKind::MarqueeUpgrade,
        NewsStoryKind::BargainBuy,
        NewsStoryKind::AcademyGraduate,
        NewsStoryKind::LoanedOutToGrow,
        NewsStoryKind::PlayerSold,
        NewsStoryKind::StarSold,
        NewsStoryKind::FreeExit,
        NewsStoryKind::LoanExit,
        NewsStoryKind::LoanReturn,
        NewsStoryKind::TransferSpeculation,
        NewsStoryKind::TransferListed,
        NewsStoryKind::ContractStandoff,
        NewsStoryKind::TransferAgreed,
        NewsStoryKind::RumourCools,
        NewsStoryKind::CommitsToClub,
        NewsStoryKind::ContractRunningDown,
        NewsStoryKind::RumourInterest,
        NewsStoryKind::RumourRival,
        NewsStoryKind::ScoutsWatching,
        NewsStoryKind::AgentTouting,
        NewsStoryKind::HomecomingLink,
        NewsStoryKind::BidRejected,
        NewsStoryKind::TalksExpected,
        NewsStoryKind::TransferRequestFiled,
        NewsStoryKind::ToldNotInPlans,
        NewsStoryKind::ContractTalksStalled,
        NewsStoryKind::MoveCollapsed,
        NewsStoryKind::AmbitionWarning,
        NewsStoryKind::UnsoldStillHere,
        NewsStoryKind::SigningNotWorking,
        NewsStoryKind::SigningComesGood,
        NewsStoryKind::LeverageUsed,
        NewsStoryKind::ContinentalAmbition,
        NewsStoryKind::LoanWatchStarter,
        NewsStoryKind::LoanWatchGoals,
        NewsStoryKind::LoanWatchBenched,
        NewsStoryKind::LoanWantsReturn,
        NewsStoryKind::LoanWantsPermanent,
        NewsStoryKind::LoanFedUp,
        NewsStoryKind::LoanRecallTalk,
        NewsStoryKind::LoanSpellEnds,
        NewsStoryKind::LoanFlop,
        NewsStoryKind::LoanStepTooBig,
        NewsStoryKind::FansChant,
        NewsStoryKind::FansTurnOnTeam,
        NewsStoryKind::FansGetBehind,
        NewsStoryKind::FansAngryAtRumour,
        NewsStoryKind::MediaPressure,
        NewsStoryKind::MediaDarling,
        NewsStoryKind::ProtestBrewing,
        NewsStoryKind::PromotionFever,
        NewsStoryKind::RelegationDread,
        NewsStoryKind::FansFurySale,
        NewsStoryKind::FansDreamSigning,
        NewsStoryKind::CupHumiliationFallout,
        NewsStoryKind::TravellingSupportRewarded,
        NewsStoryKind::AcademyDarling,
        NewsStoryKind::ManagerPressure,
        NewsStoryKind::BoardBacking,
        NewsStoryKind::ManagerSacked,
        NewsStoryKind::NewManagerArrives,
        NewsStoryKind::ManagerPoached,
        NewsStoryKind::CaretakerTakesCharge,
        NewsStoryKind::CaretakerConfirmed,
        NewsStoryKind::ManagerHunt,
        NewsStoryKind::ManagerTargetLinked,
        NewsStoryKind::ManagerWanted,
        NewsStoryKind::ManagerUltimatum,
        NewsStoryKind::ManagerContractExtended,
        NewsStoryKind::TakeoverRumour,
        NewsStoryKind::TakeoverCompleted,
        NewsStoryKind::TakeoverCollapsed,
        NewsStoryKind::StadiumExpansion,
        NewsStoryKind::FacilityUpgrade,
        NewsStoryKind::WarChest,
        NewsStoryKind::BudgetCut,
        NewsStoryKind::BoardPromiseBroken,
        NewsStoryKind::DressingRoomInquest,
        NewsStoryKind::SquadRallies,
        NewsStoryKind::BoardInvests,
        NewsStoryKind::TrophyWon,
        NewsStoryKind::PromotionWon,
        NewsStoryKind::RelegationConfirmed,
        NewsStoryKind::CupFinalHeartbreak,
        NewsStoryKind::EuropeSecured,
        NewsStoryKind::AcademyPraise,
        NewsStoryKind::BackroomEmpty,
        NewsStoryKind::IntakeDay,
        NewsStoryKind::GoldenGeneration,
        NewsStoryKind::GraduationDay,
        NewsStoryKind::MoneyWorries,
        NewsStoryKind::AdministrationEntered,
        NewsStoryKind::AdministrationExited,
        NewsStoryKind::OwnerBailout,
        NewsStoryKind::SponsorSigned,
        NewsStoryKind::SponsorshipLost,
        NewsStoryKind::SellOnWindfall,
        NewsStoryKind::PromotionBonusDue,
        NewsStoryKind::PreContractAgreed,
        NewsStoryKind::BosmanDepartureLooms,
        NewsStoryKind::DebtMountain,
        NewsStoryKind::TransferEmbargo,
        NewsStoryKind::WageBillCrisis,
        NewsStoryKind::MustSellBeforeBuying,
        NewsStoryKind::CrisisTalks,
        NewsStoryKind::BoardDemandsSale,
        NewsStoryKind::BoardBlocksDeal,
        NewsStoryKind::FacilityPlanRejected,
        NewsStoryKind::LeagueTopScorer,
        NewsStoryKind::LeagueScoringChase,
        NewsStoryKind::LeaguePlayerOfMonth,
        NewsStoryKind::LeagueYoungStar,
        NewsStoryKind::LeagueAssistKing,
        NewsStoryKind::LeagueAssistChase,
        NewsStoryKind::LeagueRatingsLeader,
        NewsStoryKind::LeagueTeamOfMonth,
    ];

    pub fn desk(self) -> NewsDesk {
        match self {
            NewsStoryKind::LeagueWin
            | NewsStoryKind::LeagueDraw
            | NewsStoryKind::GoallessDraw
            | NewsStoryKind::LeagueDefeat
            | NewsStoryKind::Rout
            | NewsStoryKind::HeavyDefeat
            | NewsStoryKind::DerbyWin
            | NewsStoryKind::DerbyDefeat
            | NewsStoryKind::CupWin
            | NewsStoryKind::CupExit
            | NewsStoryKind::LateWinner
            | NewsStoryKind::StoppageTimeDrama
            | NewsStoryKind::ComebackWin
            | NewsStoryKind::LeadThrownAway
            | NewsStoryKind::InstantReply
            | NewsStoryKind::EarlyBlitz
            | NewsStoryKind::GoalFest
            | NewsStoryKind::TenManWin
            | NewsStoryKind::WinningRun
            | NewsStoryKind::UnbeatenRun
            | NewsStoryKind::WinlessRun
            | NewsStoryKind::TitleCharge
            | NewsStoryKind::RelegationFight
            | NewsStoryKind::GoalsDriedUp
            | NewsStoryKind::DefensiveCrisis
            | NewsStoryKind::FortressHome
            | NewsStoryKind::ContinentalNightWin
            | NewsStoryKind::ContinentalDefeat
            | NewsStoryKind::ContinentalRout
            | NewsStoryKind::ContinentalHiding
            | NewsStoryKind::PlayoffGameWin
            | NewsStoryKind::PlayoffGameDefeat
            | NewsStoryKind::PlayoffTieWon
            | NewsStoryKind::PlayoffTieLost
            | NewsStoryKind::PlayoffFinalReached
            | NewsStoryKind::AwayDayForm => NewsDesk::Match,

            NewsStoryKind::HatTrick
            | NewsStoryKind::StarForm
            | NewsStoryKind::RisingStar
            | NewsStoryKind::KeeperWall
            | NewsStoryKind::KeeperMasterclass
            | NewsStoryKind::KeeperPenaltySave
            | NewsStoryKind::KeeperOverrun
            | NewsStoryKind::KeeperBlunder
            | NewsStoryKind::KeeperShutoutMilestone
            | NewsStoryKind::KeeperGoldenGlove
            | NewsStoryKind::InjuryBlow
            | NewsStoryKind::InjuryReturn
            | NewsStoryKind::InjurySetback
            | NewsStoryKind::RedCard
            | NewsStoryKind::Suspension
            | NewsStoryKind::YouthDebut
            | NewsStoryKind::MilestoneApps
            | NewsStoryKind::MilestoneGoals
            | NewsStoryKind::PlayerOfMonth
            | NewsStoryKind::TeamOfTheWeek
            | NewsStoryKind::CaptainNamed
            | NewsStoryKind::NationalCallUp
            | NewsStoryKind::GoalDrought
            | NewsStoryKind::DroughtEnded
            | NewsStoryKind::FormerClubGoal
            | NewsStoryKind::ClubServant
            | NewsStoryKind::RetirementAnnounced
            | NewsStoryKind::TrainingBustUp
            | NewsStoryKind::ContractRenewed
            | NewsStoryKind::FirstClubGoal
            | NewsStoryKind::WonStartingPlace
            | NewsStoryKind::LeaderEmerging
            | NewsStoryKind::SettlingIn
            | NewsStoryKind::TeammateFarewell
            | NewsStoryKind::InternationalRetirement
            | NewsStoryKind::CoachingAmbition
            | NewsStoryKind::CaptaincyLost
            | NewsStoryKind::ShirtUnderThreat
            | NewsStoryKind::PathwayBlocked
            | NewsStoryKind::CareerTwilight
            | NewsStoryKind::HomesickAbroad
            | NewsStoryKind::StrugglingToSettle
            | NewsStoryKind::HomeCalling
            | NewsStoryKind::SettledAtLast
            | NewsStoryKind::ColdShoulder
            | NewsStoryKind::TeammateConflict
            | NewsStoryKind::TakenUnderWing
            | NewsStoryKind::DressingRoomSpeech
            | NewsStoryKind::TrainingGroundBuzz
            | NewsStoryKind::TrainingConcerns
            | NewsStoryKind::BenchFrustration
            | NewsStoryKind::DroppedByCountry
            | NewsStoryKind::LeftOutOfSquadList
            | NewsStoryKind::OffFieldControversy
            | NewsStoryKind::WageEnvy
            | NewsStoryKind::ShirtNumberHandover
            | NewsStoryKind::OutgrownDivision
            | NewsStoryKind::RelegationNerves
            | NewsStoryKind::ContractTornUp
            | NewsStoryKind::SeasonAward
            | NewsStoryKind::ManagerBacksPlayer
            | NewsStoryKind::ManagerCallsOutPlayer
            | NewsStoryKind::DroppedForBigMatch
            | NewsStoryKind::PromiseBroken
            | NewsStoryKind::PlayerFined
            | NewsStoryKind::ClearTheAir
            | NewsStoryKind::DreamMoveComplete
            | NewsStoryKind::StatusShock
            | NewsStoryKind::PayWindfall
            | NewsStoryKind::WageRealityCheck
            | NewsStoryKind::PlayerOfWeek
            | NewsStoryKind::YoungPlayerOfWeek
            | NewsStoryKind::TeamOfMonthNod
            | NewsStoryKind::YoungPlayerOfMonthAward
            | NewsStoryKind::YoungPlayerOfSeasonAward
            | NewsStoryKind::NewManagerBounce
            | NewsStoryKind::ManagerExitUnsettles
            | NewsStoryKind::PromiseKept
            | NewsStoryKind::BanServed
            | NewsStoryKind::PlayingForContract
            | NewsStoryKind::PersonalTrainingPlan
            | NewsStoryKind::RoleRetraining
            | NewsStoryKind::HomegrownBlocked
            | NewsStoryKind::FavouritismGrumbles
            | NewsStoryKind::FamilyUnsettled
            | NewsStoryKind::FamilyCelebration
            | NewsStoryKind::CompassionateLeave
            | NewsStoryKind::LanguageLessons
            | NewsStoryKind::VeteranHomecomingWish
            | NewsStoryKind::LegendWontLeave
            | NewsStoryKind::RefusesRivalMove
            | NewsStoryKind::SeeksQuieterStage
            | NewsStoryKind::TrainingStandardsRow
            | NewsStoryKind::PositionRivalryFeud
            | NewsStoryKind::LeadershipPowerStruggle
            | NewsStoryKind::RotationRested
            | NewsStoryKind::TacticalOmission
            | NewsStoryKind::DroppedOnForm
            | NewsStoryKind::DisciplinaryOmission
            | NewsStoryKind::ForcedToRetire
            | NewsStoryKind::HomegrownQuotaOmission
            | NewsStoryKind::ForeignQuotaOmission
            | NewsStoryKind::HamstringBlow
            | NewsStoryKind::KneeLigamentBlow
            | NewsStoryKind::BrokenBoneBlow
            | NewsStoryKind::SquadKnitsTogether
            | NewsStoryKind::TurnoverToll
            | NewsStoryKind::CliqueConcerns
            | NewsStoryKind::BigMatchTrust
            | NewsStoryKind::ManagerDoubtsLinger
            | NewsStoryKind::FormationRevolution
            | NewsStoryKind::BreakthroughSeason
            | NewsStoryKind::TrainingTransformation
            | NewsStoryKind::StalledProspect
            | NewsStoryKind::PowersFading
            | NewsStoryKind::TournamentTriumph
            | NewsStoryKind::TournamentHeartbreak
            | NewsStoryKind::RoleFrustration => NewsDesk::Squad,

            NewsStoryKind::ManOfTheMatch
            | NewsStoryKind::MatchMasterclass
            | NewsStoryKind::AssistShow
            | NewsStoryKind::CreatorInChief
            | NewsStoryKind::DefensiveRock
            | NewsStoryKind::DribblingDisplay
            | NewsStoryKind::SuperSub
            | NewsStoryKind::DerbyHero
            | NewsStoryKind::MatchStinker
            | NewsStoryKind::WastefulFinishing
            | NewsStoryKind::CostlyError
            | NewsStoryKind::OwnGoalShame
            | NewsStoryKind::PenaltyMissed
            | NewsStoryKind::HookedEarly
            | NewsStoryKind::BraceHero
            | NewsStoryKind::GoalAndAssistShow
            | NewsStoryKind::TeenageStarTurn
            | NewsStoryKind::RolledBackYears
            | NewsStoryKind::DreamDebut
            | NewsStoryKind::DebutNightmare
            | NewsStoryKind::GoalFromDefence
            | NewsStoryKind::MidfieldEngine
            | NewsStoryKind::FoulTrouble => NewsDesk::Verdicts,

            NewsStoryKind::NewSigning
            | NewsStoryKind::RecordSigning
            | NewsStoryKind::FreeSigning
            | NewsStoryKind::LoanArrival
            | NewsStoryKind::HomecomingSigning
            | NewsStoryKind::LoanMadePermanent
            | NewsStoryKind::ProspectSigned
            | NewsStoryKind::VeteranArrives
            | NewsStoryKind::RivalRaid
            | NewsStoryKind::ScoutingCoup
            | NewsStoryKind::SuccessionSigning
            | NewsStoryKind::DepthSigning
            | NewsStoryKind::GapPlugged
            | NewsStoryKind::MarqueeUpgrade
            | NewsStoryKind::BargainBuy
            | NewsStoryKind::AcademyGraduate
            | NewsStoryKind::LoanedOutToGrow
            | NewsStoryKind::TransferAgreed
            | NewsStoryKind::RumourCools
            | NewsStoryKind::CommitsToClub
            | NewsStoryKind::ContractRunningDown
            | NewsStoryKind::PlayerSold
            | NewsStoryKind::StarSold
            | NewsStoryKind::FreeExit
            | NewsStoryKind::LoanExit
            | NewsStoryKind::LoanReturn
            | NewsStoryKind::TransferSpeculation
            | NewsStoryKind::TransferListed
            | NewsStoryKind::ContractStandoff
            | NewsStoryKind::RumourInterest
            | NewsStoryKind::RumourRival
            | NewsStoryKind::ScoutsWatching
            | NewsStoryKind::AgentTouting
            | NewsStoryKind::HomecomingLink
            | NewsStoryKind::BidRejected
            | NewsStoryKind::TalksExpected
            | NewsStoryKind::TransferRequestFiled
            | NewsStoryKind::ToldNotInPlans
            | NewsStoryKind::ContractTalksStalled
            | NewsStoryKind::MoveCollapsed
            | NewsStoryKind::AmbitionWarning
            | NewsStoryKind::UnsoldStillHere
            | NewsStoryKind::SigningNotWorking
            | NewsStoryKind::LeverageUsed
            | NewsStoryKind::ContinentalAmbition
            | NewsStoryKind::LoanReturnTriumph
            | NewsStoryKind::LoanReturnWasted
            | NewsStoryKind::SigningComesGood => NewsDesk::Market,

            NewsStoryKind::LoanWatchStarter
            | NewsStoryKind::LoanWatchGoals
            | NewsStoryKind::LoanWatchBenched
            | NewsStoryKind::LoanWantsReturn
            | NewsStoryKind::LoanWantsPermanent
            | NewsStoryKind::LoanFedUp
            | NewsStoryKind::LoanRecallTalk
            | NewsStoryKind::LoanSpellEnds
            | NewsStoryKind::LoanFlop
            | NewsStoryKind::LoanStepTooBig => NewsDesk::Loan,

            NewsStoryKind::FansChant
            | NewsStoryKind::FansTurnOnTeam
            | NewsStoryKind::FansGetBehind
            | NewsStoryKind::FansAngryAtRumour
            | NewsStoryKind::MediaPressure
            | NewsStoryKind::ProtestBrewing
            | NewsStoryKind::PromotionFever
            | NewsStoryKind::RelegationDread
            | NewsStoryKind::FansFurySale
            | NewsStoryKind::FansDreamSigning
            | NewsStoryKind::CupHumiliationFallout
            | NewsStoryKind::TravellingSupportRewarded
            | NewsStoryKind::AcademyDarling
            | NewsStoryKind::MediaDarling => NewsDesk::Fans,

            NewsStoryKind::ManagerPressure
            | NewsStoryKind::BoardBacking
            | NewsStoryKind::ManagerSacked
            | NewsStoryKind::NewManagerArrives
            | NewsStoryKind::ManagerPoached
            | NewsStoryKind::CaretakerTakesCharge
            | NewsStoryKind::CaretakerConfirmed
            | NewsStoryKind::ManagerHunt
            | NewsStoryKind::ManagerTargetLinked
            | NewsStoryKind::ManagerWanted
            | NewsStoryKind::ManagerUltimatum
            | NewsStoryKind::ManagerContractExtended
            | NewsStoryKind::TakeoverRumour
            | NewsStoryKind::TakeoverCompleted
            | NewsStoryKind::TakeoverCollapsed
            | NewsStoryKind::StadiumExpansion
            | NewsStoryKind::FacilityUpgrade
            | NewsStoryKind::WarChest
            | NewsStoryKind::BudgetCut
            | NewsStoryKind::BoardPromiseBroken
            | NewsStoryKind::DressingRoomInquest
            | NewsStoryKind::SquadRallies
            | NewsStoryKind::BoardInvests
            | NewsStoryKind::TrophyWon
            | NewsStoryKind::PromotionWon
            | NewsStoryKind::RelegationConfirmed
            | NewsStoryKind::CupFinalHeartbreak
            | NewsStoryKind::EuropeSecured
            | NewsStoryKind::AcademyPraise
            | NewsStoryKind::MoneyWorries
            | NewsStoryKind::AdministrationEntered
            | NewsStoryKind::AdministrationExited
            | NewsStoryKind::OwnerBailout
            | NewsStoryKind::SponsorSigned
            | NewsStoryKind::SponsorshipLost
            | NewsStoryKind::SellOnWindfall
            | NewsStoryKind::PromotionBonusDue
            | NewsStoryKind::PreContractAgreed
            | NewsStoryKind::BosmanDepartureLooms
            | NewsStoryKind::DebtMountain
            | NewsStoryKind::TransferEmbargo
            | NewsStoryKind::WageBillCrisis
            | NewsStoryKind::CrisisTalks
            | NewsStoryKind::BoardDemandsSale
            | NewsStoryKind::BoardBlocksDeal
            | NewsStoryKind::FacilityPlanRejected
            | NewsStoryKind::IntakeDay
            | NewsStoryKind::GoldenGeneration
            | NewsStoryKind::GraduationDay
            | NewsStoryKind::BackroomEmpty
            | NewsStoryKind::MustSellBeforeBuying => NewsDesk::Boardroom,

            NewsStoryKind::LeagueTopScorer
            | NewsStoryKind::LeagueScoringChase
            | NewsStoryKind::LeaguePlayerOfMonth
            | NewsStoryKind::LeagueYoungStar
            | NewsStoryKind::LeagueAssistKing
            | NewsStoryKind::LeagueAssistChase
            | NewsStoryKind::LeagueRatingsLeader
            | NewsStoryKind::LeagueTeamOfMonth => NewsDesk::Charts,
        }
    }

    /// Stable identifier the web layer expands into the headline and
    /// body translation keys. Never reuse a stem across kinds.
    pub fn key_stem(self) -> &'static str {
        match self {
            NewsStoryKind::LeagueWin => "league_win",
            NewsStoryKind::LeagueDraw => "league_draw",
            NewsStoryKind::GoallessDraw => "goalless_draw",
            NewsStoryKind::LeagueDefeat => "league_defeat",
            NewsStoryKind::Rout => "rout",
            NewsStoryKind::HeavyDefeat => "heavy_defeat",
            NewsStoryKind::DerbyWin => "derby_win",
            NewsStoryKind::DerbyDefeat => "derby_defeat",
            NewsStoryKind::CupWin => "cup_win",
            NewsStoryKind::CupExit => "cup_exit",
            NewsStoryKind::LateWinner => "late_winner",
            NewsStoryKind::StoppageTimeDrama => "stoppage_time_drama",
            NewsStoryKind::ComebackWin => "comeback_win",
            NewsStoryKind::LeadThrownAway => "lead_thrown_away",
            NewsStoryKind::InstantReply => "instant_reply",
            NewsStoryKind::EarlyBlitz => "early_blitz",
            NewsStoryKind::GoalFest => "goal_fest",
            NewsStoryKind::TenManWin => "ten_man_win",
            NewsStoryKind::WinningRun => "winning_run",
            NewsStoryKind::UnbeatenRun => "unbeaten_run",
            NewsStoryKind::WinlessRun => "winless_run",
            NewsStoryKind::TitleCharge => "title_charge",
            NewsStoryKind::RelegationFight => "relegation_fight",
            NewsStoryKind::GoalsDriedUp => "goals_dried_up",
            NewsStoryKind::DefensiveCrisis => "defensive_crisis",
            NewsStoryKind::FortressHome => "fortress_home",
            NewsStoryKind::AwayDayForm => "away_day_form",
            NewsStoryKind::ContinentalNightWin => "continental_night_win",
            NewsStoryKind::ContinentalDefeat => "continental_defeat",
            NewsStoryKind::ContinentalRout => "continental_rout",
            NewsStoryKind::ContinentalHiding => "continental_hiding",
            NewsStoryKind::PlayoffGameWin => "playoff_game_win",
            NewsStoryKind::PlayoffGameDefeat => "playoff_game_defeat",
            NewsStoryKind::PlayoffTieWon => "playoff_tie_won",
            NewsStoryKind::PlayoffTieLost => "playoff_tie_lost",
            NewsStoryKind::PlayoffFinalReached => "playoff_final_reached",
            NewsStoryKind::HatTrick => "hat_trick",
            NewsStoryKind::StarForm => "star_form",
            NewsStoryKind::RisingStar => "rising_star",
            NewsStoryKind::BreakthroughSeason => "breakthrough_season",
            NewsStoryKind::TrainingTransformation => "training_transformation",
            NewsStoryKind::StalledProspect => "stalled_prospect",
            NewsStoryKind::PowersFading => "powers_fading",
            NewsStoryKind::KeeperWall => "keeper_wall",
            NewsStoryKind::KeeperMasterclass => "keeper_masterclass",
            NewsStoryKind::KeeperPenaltySave => "keeper_penalty_save",
            NewsStoryKind::KeeperOverrun => "keeper_overrun",
            NewsStoryKind::KeeperBlunder => "keeper_blunder",
            NewsStoryKind::KeeperShutoutMilestone => "keeper_shutout_milestone",
            NewsStoryKind::KeeperGoldenGlove => "keeper_golden_glove",
            NewsStoryKind::InjuryBlow => "injury_blow",
            NewsStoryKind::InjuryReturn => "injury_return",
            NewsStoryKind::InjurySetback => "injury_setback",
            NewsStoryKind::RedCard => "red_card",
            NewsStoryKind::Suspension => "suspension",
            NewsStoryKind::YouthDebut => "youth_debut",
            NewsStoryKind::MilestoneApps => "milestone_apps",
            NewsStoryKind::MilestoneGoals => "milestone_goals",
            NewsStoryKind::PlayerOfMonth => "player_of_month",
            NewsStoryKind::TeamOfTheWeek => "team_of_the_week",
            NewsStoryKind::CaptainNamed => "captain_named",
            NewsStoryKind::NationalCallUp => "national_callup",
            NewsStoryKind::TournamentTriumph => "tournament_triumph",
            NewsStoryKind::TournamentHeartbreak => "tournament_heartbreak",
            NewsStoryKind::GoalDrought => "goal_drought",
            NewsStoryKind::DroughtEnded => "drought_ended",
            NewsStoryKind::FormerClubGoal => "former_club_goal",
            NewsStoryKind::ClubServant => "club_servant",
            NewsStoryKind::RetirementAnnounced => "retirement_announced",
            NewsStoryKind::TrainingBustUp => "training_bust_up",
            NewsStoryKind::ContractRenewed => "contract_renewed",
            NewsStoryKind::FirstClubGoal => "first_club_goal",
            NewsStoryKind::WonStartingPlace => "won_starting_place",
            NewsStoryKind::LeaderEmerging => "leader_emerging",
            NewsStoryKind::SettlingIn => "settling_in",
            NewsStoryKind::TeammateFarewell => "teammate_farewell",
            NewsStoryKind::InternationalRetirement => "international_retirement",
            NewsStoryKind::CoachingAmbition => "coaching_ambition",
            NewsStoryKind::CaptaincyLost => "captaincy_lost",
            NewsStoryKind::ShirtUnderThreat => "shirt_under_threat",
            NewsStoryKind::PathwayBlocked => "pathway_blocked",
            NewsStoryKind::CareerTwilight => "career_twilight",
            NewsStoryKind::HomesickAbroad => "homesick_abroad",
            NewsStoryKind::StrugglingToSettle => "struggling_to_settle",
            NewsStoryKind::HomeCalling => "home_calling",
            NewsStoryKind::SettledAtLast => "settled_at_last",
            NewsStoryKind::ColdShoulder => "cold_shoulder",
            NewsStoryKind::TeammateConflict => "teammate_conflict",
            NewsStoryKind::TrainingStandardsRow => "training_standards_row",
            NewsStoryKind::PositionRivalryFeud => "position_rivalry_feud",
            NewsStoryKind::LeadershipPowerStruggle => "leadership_power_struggle",
            NewsStoryKind::RotationRested => "rotation_rested",
            NewsStoryKind::TacticalOmission => "tactical_omission",
            NewsStoryKind::DroppedOnForm => "dropped_on_form",
            NewsStoryKind::DisciplinaryOmission => "disciplinary_omission",
            NewsStoryKind::ForcedToRetire => "forced_to_retire",
            NewsStoryKind::LoanReturnTriumph => "loan_return_triumph",
            NewsStoryKind::LoanReturnWasted => "loan_return_wasted",
            NewsStoryKind::HomegrownQuotaOmission => "homegrown_quota_omission",
            NewsStoryKind::ForeignQuotaOmission => "foreign_quota_omission",
            NewsStoryKind::HamstringBlow => "hamstring_blow",
            NewsStoryKind::KneeLigamentBlow => "knee_ligament_blow",
            NewsStoryKind::BrokenBoneBlow => "broken_bone_blow",
            NewsStoryKind::TakenUnderWing => "taken_under_wing",
            NewsStoryKind::DressingRoomSpeech => "dressing_room_speech",
            NewsStoryKind::SquadKnitsTogether => "squad_knits_together",
            NewsStoryKind::TurnoverToll => "turnover_toll",
            NewsStoryKind::CliqueConcerns => "clique_concerns",
            NewsStoryKind::BigMatchTrust => "big_match_trust",
            NewsStoryKind::ManagerDoubtsLinger => "manager_doubts_linger",
            NewsStoryKind::TrainingGroundBuzz => "training_ground_buzz",
            NewsStoryKind::FormationRevolution => "formation_revolution",
            NewsStoryKind::TrainingConcerns => "training_concerns",
            NewsStoryKind::BenchFrustration => "bench_frustration",
            NewsStoryKind::DroppedByCountry => "dropped_by_country",
            NewsStoryKind::LeftOutOfSquadList => "left_out_of_squad_list",
            NewsStoryKind::OffFieldControversy => "off_field_controversy",
            NewsStoryKind::WageEnvy => "wage_envy",
            NewsStoryKind::ShirtNumberHandover => "shirt_number_handover",
            NewsStoryKind::OutgrownDivision => "outgrown_division",
            NewsStoryKind::RelegationNerves => "relegation_nerves",
            NewsStoryKind::ContractTornUp => "contract_torn_up",
            NewsStoryKind::SeasonAward => "season_award",
            NewsStoryKind::DreamMoveComplete => "dream_move_complete",
            NewsStoryKind::StatusShock => "status_shock",
            NewsStoryKind::PayWindfall => "pay_windfall",
            NewsStoryKind::WageRealityCheck => "wage_reality_check",
            NewsStoryKind::PlayerOfWeek => "player_of_week",
            NewsStoryKind::YoungPlayerOfWeek => "young_player_of_week",
            NewsStoryKind::TeamOfMonthNod => "team_of_month_nod",
            NewsStoryKind::YoungPlayerOfMonthAward => "young_player_of_month",
            NewsStoryKind::YoungPlayerOfSeasonAward => "young_player_of_season",
            NewsStoryKind::NewManagerBounce => "new_manager_bounce",
            NewsStoryKind::ManagerExitUnsettles => "manager_exit_unsettles",
            NewsStoryKind::PromiseKept => "promise_kept",
            NewsStoryKind::BanServed => "ban_served",
            NewsStoryKind::PlayingForContract => "playing_for_contract",
            NewsStoryKind::PersonalTrainingPlan => "personal_training_plan",
            NewsStoryKind::RoleRetraining => "role_retraining",
            NewsStoryKind::HomegrownBlocked => "homegrown_blocked",
            NewsStoryKind::FavouritismGrumbles => "favouritism_grumbles",
            NewsStoryKind::FamilyUnsettled => "family_unsettled",
            NewsStoryKind::FamilyCelebration => "family_celebration",
            NewsStoryKind::CompassionateLeave => "compassionate_leave",
            NewsStoryKind::LanguageLessons => "language_lessons",
            NewsStoryKind::VeteranHomecomingWish => "veteran_homecoming_wish",
            NewsStoryKind::LegendWontLeave => "legend_wont_leave",
            NewsStoryKind::RefusesRivalMove => "refuses_rival_move",
            NewsStoryKind::SeeksQuieterStage => "seeks_quieter_stage",
            NewsStoryKind::ManOfTheMatch => "man_of_the_match",
            NewsStoryKind::MatchMasterclass => "match_masterclass",
            NewsStoryKind::AssistShow => "assist_show",
            NewsStoryKind::CreatorInChief => "creator_in_chief",
            NewsStoryKind::DefensiveRock => "defensive_rock",
            NewsStoryKind::DribblingDisplay => "dribbling_display",
            NewsStoryKind::SuperSub => "super_sub",
            NewsStoryKind::DerbyHero => "derby_hero",
            NewsStoryKind::MatchStinker => "match_stinker",
            NewsStoryKind::WastefulFinishing => "wasteful_finishing",
            NewsStoryKind::CostlyError => "costly_error",
            NewsStoryKind::OwnGoalShame => "own_goal_shame",
            NewsStoryKind::PenaltyMissed => "penalty_missed",
            NewsStoryKind::HookedEarly => "hooked_early",
            NewsStoryKind::FoulTrouble => "foul_trouble",
            NewsStoryKind::BraceHero => "brace_hero",
            NewsStoryKind::GoalAndAssistShow => "goal_and_assist_show",
            NewsStoryKind::TeenageStarTurn => "teenage_star_turn",
            NewsStoryKind::RolledBackYears => "rolled_back_years",
            NewsStoryKind::DreamDebut => "dream_debut",
            NewsStoryKind::DebutNightmare => "debut_nightmare",
            NewsStoryKind::GoalFromDefence => "goal_from_defence",
            NewsStoryKind::MidfieldEngine => "midfield_engine",
            NewsStoryKind::RoleFrustration => "role_frustration",
            NewsStoryKind::MoveCollapsed => "move_collapsed",
            NewsStoryKind::AmbitionWarning => "ambition_warning",
            NewsStoryKind::UnsoldStillHere => "unsold_still_here",
            NewsStoryKind::LoanFedUp => "loan_fed_up",
            NewsStoryKind::ManagerBacksPlayer => "manager_backs_player",
            NewsStoryKind::ManagerCallsOutPlayer => "manager_calls_out",
            NewsStoryKind::DroppedForBigMatch => "dropped_big_match",
            NewsStoryKind::PromiseBroken => "promise_broken",
            NewsStoryKind::PlayerFined => "player_fined",
            NewsStoryKind::ClearTheAir => "clear_the_air",
            NewsStoryKind::NewSigning => "new_signing",
            NewsStoryKind::RecordSigning => "record_signing",
            NewsStoryKind::FreeSigning => "free_signing",
            NewsStoryKind::LoanArrival => "loan_arrival",
            NewsStoryKind::HomecomingSigning => "homecoming_signing",
            NewsStoryKind::LoanMadePermanent => "loan_made_permanent",
            NewsStoryKind::ProspectSigned => "prospect_signed",
            NewsStoryKind::VeteranArrives => "veteran_arrives",
            NewsStoryKind::TransferAgreed => "transfer_agreed",
            NewsStoryKind::RumourCools => "rumour_cools",
            NewsStoryKind::CommitsToClub => "commits_to_club",
            NewsStoryKind::ContractRunningDown => "contract_running_down",
            NewsStoryKind::PlayerSold => "player_sold",
            NewsStoryKind::StarSold => "star_sold",
            NewsStoryKind::FreeExit => "free_exit",
            NewsStoryKind::RivalRaid => "rival_raid",
            NewsStoryKind::ScoutingCoup => "scouting_coup",
            NewsStoryKind::SuccessionSigning => "succession_signing",
            NewsStoryKind::DepthSigning => "depth_signing",
            NewsStoryKind::GapPlugged => "gap_plugged",
            NewsStoryKind::MarqueeUpgrade => "marquee_upgrade",
            NewsStoryKind::BargainBuy => "bargain_buy",
            NewsStoryKind::AcademyGraduate => "academy_graduate",
            NewsStoryKind::LoanedOutToGrow => "loaned_out_to_grow",
            NewsStoryKind::LoanExit => "loan_exit",
            NewsStoryKind::LoanReturn => "loan_return",
            NewsStoryKind::TransferSpeculation => "transfer_speculation",
            NewsStoryKind::TransferListed => "transfer_listed",
            NewsStoryKind::ContractStandoff => "contract_standoff",
            NewsStoryKind::RumourInterest => "rumour_interest",
            NewsStoryKind::RumourRival => "rumour_rival",
            NewsStoryKind::ScoutsWatching => "scouts_watching",
            NewsStoryKind::AgentTouting => "agent_touting",
            NewsStoryKind::HomecomingLink => "homecoming_link",
            NewsStoryKind::BidRejected => "bid_rejected",
            NewsStoryKind::TalksExpected => "talks_expected",
            NewsStoryKind::TransferRequestFiled => "transfer_request",
            NewsStoryKind::ToldNotInPlans => "told_not_in_plans",
            NewsStoryKind::ContractTalksStalled => "contract_talks_stalled",
            NewsStoryKind::SigningNotWorking => "signing_not_working",
            NewsStoryKind::SigningComesGood => "signing_comes_good",
            NewsStoryKind::LeverageUsed => "leverage_used",
            NewsStoryKind::ContinentalAmbition => "continental_ambition",
            NewsStoryKind::LoanWatchStarter => "loan_watch_starter",
            NewsStoryKind::LoanWatchGoals => "loan_watch_goals",
            NewsStoryKind::LoanWatchBenched => "loan_watch_benched",
            NewsStoryKind::LoanWantsReturn => "loan_wants_return",
            NewsStoryKind::LoanWantsPermanent => "loan_wants_permanent",
            NewsStoryKind::LoanRecallTalk => "loan_recall_talk",
            NewsStoryKind::LoanSpellEnds => "loan_spell_ends",
            NewsStoryKind::LoanFlop => "loan_flop",
            NewsStoryKind::LoanStepTooBig => "loan_step_too_big",
            NewsStoryKind::FansChant => "fans_chant",
            NewsStoryKind::FansTurnOnTeam => "fans_turn",
            NewsStoryKind::FansGetBehind => "fans_get_behind",
            NewsStoryKind::FansAngryAtRumour => "fans_angry_rumour",
            NewsStoryKind::MediaPressure => "media_pressure",
            NewsStoryKind::MediaDarling => "media_darling",
            NewsStoryKind::ProtestBrewing => "protest_brewing",
            NewsStoryKind::PromotionFever => "promotion_fever",
            NewsStoryKind::RelegationDread => "relegation_dread",
            NewsStoryKind::FansFurySale => "fans_fury_sale",
            NewsStoryKind::FansDreamSigning => "fans_dream_signing",
            NewsStoryKind::CupHumiliationFallout => "cup_humiliation_fallout",
            NewsStoryKind::TravellingSupportRewarded => "travelling_support_rewarded",
            NewsStoryKind::AcademyDarling => "academy_darling",
            NewsStoryKind::ManagerPressure => "manager_pressure",
            NewsStoryKind::BoardBacking => "board_backing",
            NewsStoryKind::ManagerSacked => "manager_sacked",
            NewsStoryKind::NewManagerArrives => "new_manager",
            NewsStoryKind::ManagerPoached => "manager_poached",
            NewsStoryKind::CaretakerTakesCharge => "caretaker_takes_charge",
            NewsStoryKind::CaretakerConfirmed => "caretaker_confirmed",
            NewsStoryKind::ManagerHunt => "manager_hunt",
            NewsStoryKind::ManagerTargetLinked => "manager_target_linked",
            NewsStoryKind::ManagerWanted => "manager_wanted",
            NewsStoryKind::ManagerUltimatum => "manager_ultimatum",
            NewsStoryKind::ManagerContractExtended => "manager_contract_extended",
            NewsStoryKind::TakeoverRumour => "takeover_rumour",
            NewsStoryKind::TakeoverCompleted => "takeover_completed",
            NewsStoryKind::TakeoverCollapsed => "takeover_collapsed",
            NewsStoryKind::StadiumExpansion => "stadium_expansion",
            NewsStoryKind::FacilityUpgrade => "facility_upgrade",
            NewsStoryKind::WarChest => "war_chest",
            NewsStoryKind::BudgetCut => "budget_cut",
            NewsStoryKind::BoardPromiseBroken => "board_promise_broken",
            NewsStoryKind::DressingRoomInquest => "dressing_room_inquest",
            NewsStoryKind::SquadRallies => "squad_rallies",
            NewsStoryKind::BoardInvests => "board_invests",
            NewsStoryKind::TrophyWon => "trophy_won",
            NewsStoryKind::PromotionWon => "promotion_won",
            NewsStoryKind::RelegationConfirmed => "relegation_confirmed",
            NewsStoryKind::CupFinalHeartbreak => "cup_final_heartbreak",
            NewsStoryKind::EuropeSecured => "europe_secured",
            NewsStoryKind::AcademyPraise => "academy_praise",
            NewsStoryKind::BackroomEmpty => "backroom_empty",
            NewsStoryKind::IntakeDay => "intake_day",
            NewsStoryKind::GoldenGeneration => "golden_generation",
            NewsStoryKind::GraduationDay => "graduation_day",
            NewsStoryKind::MoneyWorries => "money_worries",
            NewsStoryKind::AdministrationEntered => "administration_entered",
            NewsStoryKind::AdministrationExited => "administration_exited",
            NewsStoryKind::OwnerBailout => "owner_bailout",
            NewsStoryKind::SponsorSigned => "sponsor_signed",
            NewsStoryKind::SponsorshipLost => "sponsorship_lost",
            NewsStoryKind::SellOnWindfall => "sell_on_windfall",
            NewsStoryKind::PromotionBonusDue => "promotion_bonus_due",
            NewsStoryKind::PreContractAgreed => "pre_contract_agreed",
            NewsStoryKind::BosmanDepartureLooms => "bosman_departure_looms",
            NewsStoryKind::DebtMountain => "debt_mountain",
            NewsStoryKind::TransferEmbargo => "transfer_embargo",
            NewsStoryKind::WageBillCrisis => "wage_bill_crisis",
            NewsStoryKind::MustSellBeforeBuying => "must_sell_before_buying",
            NewsStoryKind::CrisisTalks => "crisis_talks",
            NewsStoryKind::BoardDemandsSale => "board_demands_sale",
            NewsStoryKind::BoardBlocksDeal => "board_blocks_deal",
            NewsStoryKind::FacilityPlanRejected => "facility_plan_rejected",
            NewsStoryKind::LeagueTopScorer => "league_top_scorer",
            NewsStoryKind::LeagueScoringChase => "league_scoring_chase",
            NewsStoryKind::LeaguePlayerOfMonth => "league_player_of_month",
            NewsStoryKind::LeagueYoungStar => "league_young_star",
            NewsStoryKind::LeagueAssistKing => "league_assist_king",
            NewsStoryKind::LeagueAssistChase => "league_assist_chase",
            NewsStoryKind::LeagueRatingsLeader => "league_ratings_leader",
            NewsStoryKind::LeagueTeamOfMonth => "league_team_of_month",
        }
    }

    /// Newsworthiness before per-story modifiers. Calibrated against
    /// how a local paper really ranks its page: silverware and a derby
    /// lead over a routine win, a routine win leads over a contract
    /// standoff, and the loan column is back-page furniture that only
    /// reaches the front when a loanee says something worth quoting.
    ///
    /// The bands, so a new kind can be placed without reading all of
    /// them:
    ///
    /// - **900+** — the edition a town keeps. Silverware, promotion,
    ///   relegation, administration.
    /// - **600–800** — the big week: a sacking, a takeover, a derby, a
    ///   record signing, the afternoon the season turned on.
    /// - **400–600** — the ordinary front of the paper. Match reports,
    ///   transfers done, runs of form, a manager under pressure.
    /// - **below 400** — furniture. Real news, but the stuff that fills
    ///   a page rather than leads it.
    ///
    /// Ties are safe — the editor breaks them deterministically on date,
    /// stem and ids — so a new kind only has to land in the right band.
    pub fn base_priority(self) -> u16 {
        match self {
            NewsStoryKind::TrophyWon => 900,
            // Going up and going down are the two editions a town keeps.
            NewsStoryKind::PromotionWon => 890,
            NewsStoryKind::RelegationConfirmed => 870,
            // Administration outranks a sacking and very nearly a
            // relegation: it takes points off a club that has not played
            // for them, and it is the one boardroom morning a supporter
            // still talks about twenty years later.
            NewsStoryKind::AdministrationEntered => 860,
            NewsStoryKind::ManagerSacked => 820,
            // Losing the manager to somebody bigger is very nearly the
            // same size of morning as sacking him, and it hurts more:
            // nobody at the club wanted this one.
            NewsStoryKind::ManagerPoached => 810,
            // New owners. The loudest thing that can happen to a club
            // without a ball being kicked.
            NewsStoryKind::TakeoverCompleted => 800,
            // Coming out the other side. Smaller than going in — the
            // damage is already done and the town has lived with it for
            // a year — but the day the sanctions lift is still page one.
            NewsStoryKind::AdministrationExited => 700,
            NewsStoryKind::DerbyWin | NewsStoryKind::DerbyDefeat => 720,
            NewsStoryKind::RecordSigning => 700,
            NewsStoryKind::NewManagerArrives => 690,
            // The stand-in gets it for keeps. Smaller than unveiling a
            // stranger, and a story the town has stronger feelings
            // about, because it already knows him.
            NewsStoryKind::CaretakerConfirmed => 660,
            // The public ultimatum. Louder than the standing "pressure
            // builds" piece because it happened on a day.
            NewsStoryKind::ManagerUltimatum => 620,
            // The final lost is the week the whole town went quiet.
            NewsStoryKind::CupFinalHeartbreak => 680,
            NewsStoryKind::StarSold => 660,
            // Continental football is a season-defining prize for
            // everybody outside the handful of clubs that expect it.
            NewsStoryKind::EuropeSecured => 650,
            NewsStoryKind::CupWin => 640,
            NewsStoryKind::CupExit => 620,
            NewsStoryKind::Rout | NewsStoryKind::HeavyDefeat => 600,
            // How it happened, when how it happened was the story. All
            // of these outrank the plain report of the same afternoon,
            // which is the right way round: no paper leads on "won 3-2"
            // when it can lead on "two down at half time".
            NewsStoryKind::ComebackWin => 615,
            NewsStoryKind::LeadThrownAway => 610,
            NewsStoryKind::StoppageTimeDrama => 605,
            NewsStoryKind::LateWinner => 590,
            // Winning a man short is a story about the ten who stayed
            // on, and it is told about them for years.
            NewsStoryKind::TenManWin => 585,
            NewsStoryKind::HatTrick => 580,
            // The month's leading scorer leads the division's own paper
            // the way a trophy leads a club's. It is the one thing that
            // page exists to say, and it outranks every live transfer
            // link on it — a chart is settled, a rumour is not.
            NewsStoryKind::LeagueTopScorer => 760,
            // The rest of the chart. Deliberately below a rejected bid
            // and a transfer request: after the man at the top, the
            // biggest thing on a back page is somebody trying to leave.
            NewsStoryKind::LeagueScoringChase => 470,
            // The division's verdict on a month. Just below the scoring
            // chart, which is the one thing that page exists to settle.
            NewsStoryKind::LeaguePlayerOfMonth => 740,
            NewsStoryKind::LeagueYoungStar => 620,
            // The other chart. Below the goals, because it always is,
            // and above everything else on the page.
            NewsStoryKind::LeagueAssistKing => 560,
            NewsStoryKind::LeagueAssistChase => 440,
            NewsStoryKind::LeagueRatingsLeader => 520,
            NewsStoryKind::LeagueTeamOfMonth => 458,
            // A shoot-out kept out is the save a town retells for
            // twenty years, and it decided the tie it happened in.
            NewsStoryKind::KeeperPenaltySave => 575,
            // The keeper who won the match on his own. Rare enough
            // that when it happens it leads.
            NewsStoryKind::KeeperMasterclass => 545,
            NewsStoryKind::RetirementAnnounced => 560,
            // Somebody has to pick the team on Saturday, and the paper
            // has to tell its readers who.
            NewsStoryKind::CaretakerTakesCharge => 558,
            // Rumours of a sale. It changes nothing yet and everybody
            // talks about nothing else.
            NewsStoryKind::TakeoverRumour => 556,
            // Another club has come asking about ours. A supporter's
            // week is ruined by the asking, never mind the answer.
            NewsStoryKind::ManagerWanted => 545,
            NewsStoryKind::TitleCharge | NewsStoryKind::RelegationFight => 540,
            NewsStoryKind::ManagerPressure => 520,
            // Tying the man down. Quieter than appointing one, and the
            // clearest statement of intent a board ever makes.
            NewsStoryKind::ManagerContractExtended => 500,
            NewsStoryKind::TransferRequestFiled => 510,
            NewsStoryKind::PromiseBroken => 505,
            NewsStoryKind::WinningRun | NewsStoryKind::WinlessRun => 500,
            // Six goals in one match. Nobody reads the table for this
            // one, they read it because it was a good afternoon out.
            NewsStoryKind::GoalFest => 480,
            // The two halves of a bad run that a phone-in argues about
            // separately: we cannot score, and we cannot defend.
            NewsStoryKind::GoalsDriedUp => 462,
            NewsStoryKind::DefensiveCrisis => 458,
            // Form with an address on it. Away wins persuade a town its
            // side is real; a long unbeaten home run is the thing
            // visiting managers get asked about.
            NewsStoryKind::AwayDayForm => 450,
            // A European night is a bigger evening than any domestic
            // fixture that is not a derby, and a town treats it that way.
            NewsStoryKind::ContinentalNightWin => 650,
            NewsStoryKind::ContinentalDefeat => 596,
            NewsStoryKind::ContinentalRout => 690,
            NewsStoryKind::ContinentalHiding => 664,
            // A playoff game is worth more than the identical scoreline
            // in April, and every supporter knows exactly why.
            NewsStoryKind::PlayoffGameWin => 618,
            NewsStoryKind::PlayoffGameDefeat => 606,
            // A series settled. The whole point of a playoff, and a
            // bigger morning than any single game inside it.
            NewsStoryKind::PlayoffTieWon => 700,
            NewsStoryKind::PlayoffTieLost => 686,
            // One game from everything. The largest thing that can
            // happen to a club without anything actually being won.
            NewsStoryKind::PlayoffFinalReached => 780,
            NewsStoryKind::FortressHome => 446,
            // Over inside twenty minutes, and the reply that said it
            // was not. Both are shapes of an afternoon rather than
            // results, so they sit below the run-of-form pieces.
            NewsStoryKind::EarlyBlitz => 444,
            NewsStoryKind::InstantReply => 432,
            NewsStoryKind::FansTurnOnTeam => 495,
            NewsStoryKind::BidRejected => 490,
            // A deal agreed is louder than a deal rumoured and quieter
            // than a deal done — the exact register of "medical booked".
            NewsStoryKind::TransferAgreed => 486,
            // The prodigal outranks a routine signing of the same size:
            // the town already knows the name.
            NewsStoryKind::HomecomingSigning => 484,
            NewsStoryKind::NewSigning => 480,
            // Keeping a loanee is quieter news than unveiling a
            // stranger — everybody has already seen him play.
            NewsStoryKind::LoanMadePermanent => 468,
            // The summer everybody remembers is the one where the move
            // fell through, not the one where it went smoothly.
            NewsStoryKind::MoveCollapsed => 478,
            // A local kid coming through is the story a town wants
            // most, and it outranks the same form from a senior pro.
            NewsStoryKind::RisingStar => 475,
            // A young player who has visibly got better. The story a
            // club's own supporters most want to be true.
            NewsStoryKind::BreakthroughSeason => 462,
            NewsStoryKind::TrainingTransformation => 418,
            NewsStoryKind::StalledProspect => 386,
            // A veteran going backwards. Everybody sees it before
            // anybody says it, which is what makes it printable.
            NewsStoryKind::PowersFading => 424,
            NewsStoryKind::UnbeatenRun => 470,
            // The club has moved for somebody else's manager, and the
            // sale that was going to change everything fell over. Both
            // are stories about a thing that has not happened.
            NewsStoryKind::ManagerTargetLinked => 472,
            NewsStoryKind::TakeoverCollapsed => 468,
            NewsStoryKind::PlayerSold => 460,
            // The vacancy itself, week after week: no manager, no
            // signature, and a list of names in the papers.
            NewsStoryKind::ManagerHunt => 452,
            NewsStoryKind::ClubServant => 450,
            NewsStoryKind::LeagueWin | NewsStoryKind::LeagueDefeat => 440,
            NewsStoryKind::RumourInterest | NewsStoryKind::RumourRival => 430,
            NewsStoryKind::MilestoneGoals | NewsStoryKind::MilestoneApps => 420,
            // The board's word, and what it bought. A promise not kept
            // outranks the money itself: the money can come back.
            NewsStoryKind::BoardPromiseBroken => 425,
            NewsStoryKind::StadiumExpansion => 430,
            // The owner keeping the lights on. Big news, and news a
            // supporter reads with relief rather than pleasure — money
            // that covers a hole buys nobody.
            NewsStoryKind::OwnerBailout => 462,
            // Barred from signing anybody at all. It decides the club's
            // next two windows and every rumour printed in between.
            NewsStoryKind::TransferEmbargo => 445,
            // The debt figure itself: the number a phone-in quotes for
            // the next five years.
            NewsStoryKind::DebtMountain => 428,
            // Wages against income. Duller than the debt and the reason
            // for it.
            NewsStoryKind::WageBillCrisis => 396,
            // "Nobody comes in until somebody goes out" — the market's
            // version of a balance sheet, and the line the rumour mill
            // runs on all summer.
            NewsStoryKind::MustSellBeforeBuying => 388,
            // The board summoning everybody is one step from a sacking
            // and every supporter in the town knows it.
            NewsStoryKind::CrisisTalks => 588,
            NewsStoryKind::BoardDemandsSale => 512,
            NewsStoryKind::BoardBlocksDeal => 496,
            NewsStoryKind::FacilityPlanRejected => 384,
            NewsStoryKind::WarChest => 410,
            // Commercial news. It pays for everything and nobody sings
            // about it, so it sits below the football and above the
            // small change.
            NewsStoryKind::SponsorSigned => 358,
            NewsStoryKind::SponsorshipLost => 366,
            // Money for a player the club no longer owns. A promotion
            // bonus is the sweeter of the two — somebody else went up
            // and it paid us — so it leads the pair.
            NewsStoryKind::PromotionBonusDue => 392,
            NewsStoryKind::SellOnWindfall => 384,
            // Losing a man for nothing is the louder half of a Bosman
            // by a distance: one club has done good business, the
            // other has to explain itself for the rest of the season.
            NewsStoryKind::BosmanDepartureLooms => 498,
            NewsStoryKind::PreContractAgreed => 462,
            NewsStoryKind::PlayerOfMonth => 410,
            // A player telling his own club he has been away long
            // enough is the loan column's loudest week — it outranks
            // the ordinary "get me out of here" line, which is about
            // the borrowing club rather than about home.
            NewsStoryKind::LoanFedUp => 408,
            NewsStoryKind::LoanWantsReturn => 405,
            // A dressing-room name saying the club is not good enough
            // for him is a back page in itself.
            NewsStoryKind::AmbitionWarning => 402,
            NewsStoryKind::DressingRoomInquest => 415,
            NewsStoryKind::InjuryBlow => 400,
            NewsStoryKind::ContractRenewed => 395,
            NewsStoryKind::TrainingBustUp => 390,
            NewsStoryKind::SigningNotWorking => 385,
            NewsStoryKind::LeagueDraw => 380,
            NewsStoryKind::BudgetCut => 360,
            // The last cap. A player can be finished with his country
            // and nowhere near finished with his club.
            NewsStoryKind::InternationalRetirement => 348,
            // Off the mark. Every signing is asked about this goal at
            // every press conference until he scores it.
            NewsStoryKind::FirstClubGoal => 342,
            NewsStoryKind::TalksExpected => 375,
            NewsStoryKind::RedCard => 370,
            // The season's best goalkeeper, by the one award nobody
            // else can win.
            NewsStoryKind::KeeperGoldenGlove => 425,
            // A keeper's own mistake in his own net: the shortest
            // route from anonymity to the back page.
            NewsStoryKind::KeeperBlunder => 405,
            // Beaten four times. Whether it was him or the ten in
            // front is the argument, and the argument is the story.
            NewsStoryKind::KeeperOverrun => 372,
            NewsStoryKind::GoallessDraw => 365,
            NewsStoryKind::YouthDebut => 360,
            NewsStoryKind::ToldNotInPlans => 355,
            NewsStoryKind::ManagerCallsOutPlayer => 352,
            NewsStoryKind::StarForm => 350,
            // A raid on the neighbours is enjoyed twice: once for the
            // player and once for where he came from. It outranks any
            // signing of the same size that came from nobody in
            // particular.
            NewsStoryKind::RivalRaid => 520,
            // The upgrade the club paid for, and the bargain it did
            // not. Both are the ordinary signing report with a reason
            // attached, so both sit just above it.
            NewsStoryKind::MarqueeUpgrade => 486,
            // One of our own, promoted. A local readership takes this
            // more personally than a signing of any size.
            NewsStoryKind::AcademyGraduate => 448,
            NewsStoryKind::BargainBuy => 442,
            // Signing a man to replace somebody who is still in the
            // building is the transfer with a subject nobody names.
            NewsStoryKind::SuccessionSigning => 436,
            NewsStoryKind::GapPlugged => 412,
            // The scouting department's own find, with the confidence
            // it filed the report at.
            NewsStoryKind::ScoutingCoup => 406,
            // One for the future: real news, but the paper cannot rave
            // about a kid nobody has seen play yet.
            NewsStoryKind::ProspectSigned => 398,
            NewsStoryKind::VeteranArrives => 368,
            // Cover is the least glamorous business a club does and
            // the reason a season survives February.
            NewsStoryKind::DepthSigning => 352,
            // A lad sent out to play, told from the parent's page.
            NewsStoryKind::LoanedOutToGrow => 268,
            NewsStoryKind::NationalCallUp => 345,
            // A player of this club has won an international
            // tournament. The largest thing that can appear on a club
            // page that the club had nothing to do with.
            NewsStoryKind::TournamentTriumph => 820,
            NewsStoryKind::TournamentHeartbreak => 640,
            NewsStoryKind::MoneyWorries => 340,
            // Losing the armband is the sharper end of the captaincy
            // story, so it runs a shade above the naming of one.
            NewsStoryKind::CaptaincyLost => 338,
            NewsStoryKind::CaptainNamed => 335,
            // The club's own kid kept out by a borrowed player is the
            // grievance a local readership takes personally.
            NewsStoryKind::PathwayBlocked => 332,
            NewsStoryKind::Suspension => 330,
            NewsStoryKind::DroppedForBigMatch => 325,
            // Whether the old warrior has one more season in him is a
            // story the town starts telling long before he answers it.
            NewsStoryKind::CareerTwilight => 318,
            NewsStoryKind::FreeSigning | NewsStoryKind::LoanArrival => 320,
            NewsStoryKind::ContractTalksStalled => 315,
            // Silence from upstairs with the clock running: quieter
            // than talks breaking down, sharper than plain speculation.
            NewsStoryKind::ContractRunningDown => 312,
            // "I am going nowhere" is a story precisely because the
            // paper spent a month printing the opposite.
            NewsStoryKind::CommitsToClub => 296,
            NewsStoryKind::LoanWantsPermanent => 310,
            NewsStoryKind::DroughtEnded => 305,
            NewsStoryKind::KeeperWall => 300,
            // He has taken somebody's shirt. The paper has always run
            // the losing half of this story and never the winning one.
            NewsStoryKind::WonStartingPlace => 302,
            // A voice in the dressing room where there was not one.
            NewsStoryKind::LeaderEmerging => 292,
            // A friend gone. The transfer was somebody else's story;
            // this is what it did to the man left behind.
            NewsStoryKind::TeammateFarewell => 266,
            // Looking at the other side of the white line. Half the
            // coaches in the game gave this interview once.
            NewsStoryKind::CoachingAmbition => 252,
            // Settling in: the language coming, a compatriot arriving.
            NewsStoryKind::SettlingIn => 238,
            // Money spent where a supporter cannot see it.
            NewsStoryKind::FacilityUpgrade => 232,
            // The number a goalkeeper's career is really counted in.
            NewsStoryKind::KeeperShutoutMilestone => 415,
            NewsStoryKind::FreeExit => 295,
            NewsStoryKind::LoanFlop => 292,
            NewsStoryKind::TransferListed => 290,
            NewsStoryKind::ClearTheAir => 288,
            // Somebody has come for his shirt — squad news, not a
            // headline, until he loses it.
            NewsStoryKind::ShirtUnderThreat => 286,
            NewsStoryKind::RoleFrustration => 284,
            // Still here, still unwanted: a standing embarrassment
            // rather than an event, and the paper runs it quietly.
            NewsStoryKind::UnsoldStillHere => 276,
            NewsStoryKind::LoanWatchGoals => 285,
            NewsStoryKind::MediaPressure => 282,
            NewsStoryKind::ContractStandoff => 280,
            NewsStoryKind::SquadRallies => 278,
            NewsStoryKind::HomecomingLink => 275,
            NewsStoryKind::SigningComesGood => 272,
            // Interest from elsewhere, converted into a better deal here.
            // The most cynical thing in football and the most ordinary.
            NewsStoryKind::LeverageUsed => 380,
            NewsStoryKind::ContinentalAmbition => 402,
            NewsStoryKind::FormerClubGoal => 270,
            NewsStoryKind::PlayerFined => 268,
            NewsStoryKind::AgentTouting => 265,
            NewsStoryKind::FansGetBehind => 262,
            NewsStoryKind::TransferSpeculation => 260,
            NewsStoryKind::LoanStepTooBig => 258,
            NewsStoryKind::TeamOfTheWeek => 255,
            NewsStoryKind::BoardInvests => 252,
            NewsStoryKind::LoanRecallTalk => 250,
            NewsStoryKind::InjurySetback => 245,
            NewsStoryKind::FansAngryAtRumour => 242,
            NewsStoryKind::LoanExit => 240,
            // A rumour ending is a paragraph, not a lead — but a paper
            // that never closes its own stories reads as noise.
            NewsStoryKind::RumourCools => 236,
            NewsStoryKind::GoalDrought => 235,
            NewsStoryKind::AcademyPraise => 230,
            // A club with no coaching staff at all. Not an interim
            // appointment — an institution with nobody left in it.
            NewsStoryKind::BackroomEmpty => 604,
            // One morning a year, and the only day the academy is
            // visible from outside the building.
            NewsStoryKind::IntakeDay => 396,
            NewsStoryKind::GoldenGeneration => 528,
            NewsStoryKind::GraduationDay => 470,
            NewsStoryKind::LoanWatchStarter => 225,
            NewsStoryKind::BoardBacking => 220,
            NewsStoryKind::MediaDarling => 218,
            // Unrest that has organised itself. Above every other mood
            // piece, because it is no longer a mood.
            NewsStoryKind::ProtestBrewing => 610,
            NewsStoryKind::PromotionFever => 500,
            NewsStoryKind::RelegationDread => 516,
            // Selling somebody the ground had decided was theirs. The
            // loudest thing a board can do to a supporter without
            // touching the manager.
            NewsStoryKind::FansFurySale => 544,
            NewsStoryKind::FansDreamSigning => 468,
            NewsStoryKind::CupHumiliationFallout => 556,
            NewsStoryKind::TravellingSupportRewarded => 412,
            NewsStoryKind::AcademyDarling => 452,
            NewsStoryKind::ScoutsWatching => 215,
            NewsStoryKind::LoanReturn => 210,
            NewsStoryKind::ManagerBacksPlayer => 205,
            NewsStoryKind::InjuryReturn => 200,
            NewsStoryKind::LoanWatchBenched => 195,
            NewsStoryKind::LoanSpellEnds => 190,
            NewsStoryKind::FansChant => 180,

            // ── The ratings page ──────────────────────────────────
            // What one man did in one afternoon. These rank high on
            // purpose: individual performance is most of what a real
            // football paper is, and the desk that files them was
            // added because the page had almost none of it.
            //
            // Winning a derby on your own is the loudest thing a
            // player can do in a shirt.
            NewsStoryKind::DerbyHero => 468,
            // An eight-out-of-ten afternoon: the game a supporter
            // still brings up a decade later.
            NewsStoryKind::MatchMasterclass => 432,
            // Twelve yards, his to settle, and he missed. Louder than
            // any good thing he did the same afternoon.
            NewsStoryKind::PenaltyMissed => 414,
            // Into his own net. Nobody's fault and entirely his.
            NewsStoryKind::OwnGoalShame => 401,
            // His mistake, their goal — and unlike a keeper's, this
            // one had a keeper behind it.
            NewsStoryKind::CostlyError => 397,
            // Three made for other people in one game.
            NewsStoryKind::AssistShow => 394,
            NewsStoryKind::ManOfTheMatch => 391,
            // The chances were there. This is the afternoon a town
            // argues about all week, which is exactly why it prints
            // above the ordinary bad display.
            NewsStoryKind::WastefulFinishing => 357,
            // Marked down. The half of a ratings column that sells it.
            NewsStoryKind::MatchStinker => 345,
            NewsStoryKind::SuperSub => 337,
            NewsStoryKind::DefensiveRock => 307,
            // A manager's verdict delivered in public, at 55 minutes.
            NewsStoryKind::HookedEarly => 293,
            NewsStoryKind::CreatorInChief => 289,
            NewsStoryKind::DribblingDisplay => 281,
            NewsStoryKind::FoulTrouble => 273,
            // Two goals. Below a hat-trick and above every other verdict
            // the column can reach for, because it decided the match.
            NewsStoryKind::BraceHero => 552,
            NewsStoryKind::GoalAndAssistShow => 486,
            // A kid doing it against grown men. Above the plain
            // masterclass, because the age is the story.
            NewsStoryKind::TeenageStarTurn => 540,
            NewsStoryKind::RolledBackYears => 528,
            // First afternoon in the shirt, and it went perfectly. A
            // player only ever gets one of these.
            NewsStoryKind::DreamDebut => 572,
            NewsStoryKind::DebutNightmare => 508,
            NewsStoryKind::GoalFromDefence => 494,
            NewsStoryKind::MidfieldEngine => 424,

            // ── A foreign player's life ───────────────────────────
            // Wanting out of the country is a different story from
            // wanting out of the club, and a bigger one: form can be
            // coached, homesickness cannot.
            NewsStoryKind::HomesickAbroad => 393,
            // …and the version with a destination attached.
            NewsStoryKind::HomeCalling => 373,
            // Signed from the neighbours. A dressing room forgets
            // nothing and a local paper forgets less.
            NewsStoryKind::ColdShoulder => 317,
            // Still on his own after a year. The explanation nobody
            // reaches for when a signing is not working.
            NewsStoryKind::StrugglingToSettle => 309,
            NewsStoryKind::SettledAtLast => 244,

            // ── The room ──────────────────────────────────────────
            NewsStoryKind::TeammateConflict => 387,
            // The three rows a dressing room actually has. All a shade
            // above the flat conflict piece they were folded into,
            // because a named grievance is a bigger story than a spat.
            NewsStoryKind::TrainingStandardsRow => 366,
            NewsStoryKind::PositionRivalryFeud => 372,
            NewsStoryKind::LeadershipPowerStruggle => 380,
            // Being rested is the one omission that is not a story, and
            // printing it as one is how a paper loses a reader. Filler,
            // deliberately.
            NewsStoryKind::RotationRested => 232,
            NewsStoryKind::TacticalOmission => 318,
            NewsStoryKind::DroppedOnForm => 348,
            // Left out as a punishment. Not a selection call at all, and
            // the only omission a dressing room reads as news.
            NewsStoryKind::DisciplinaryOmission => 402,
            // A career ended by a body rather than by a decision. Above
            // the planned farewell, because nobody got to say goodbye.
            NewsStoryKind::ForcedToRetire => 640,
            // The loan finally says how it went. A spell that worked is a
            // squad option the manager did not have in August.
            NewsStoryKind::LoanReturnTriumph => 404,
            NewsStoryKind::LoanReturnWasted => 386,
            // Left off a list because of how the squad was assembled
            // rather than because of him. A story about the club.
            NewsStoryKind::HomegrownQuotaOmission => 356,
            NewsStoryKind::ForeignQuotaOmission => 350,
            // The three injuries a supporter can picture. All above the
            // generic blow they were folded into: naming the injury is
            // the difference between a squad note and news.
            NewsStoryKind::HamstringBlow => 404,
            // The one every supporter fears by name.
            NewsStoryKind::KneeLigamentBlow => 620,
            NewsStoryKind::BrokenBoneBlow => 560,
            NewsStoryKind::DressingRoomSpeech => 263,
            // The room, read as a whole. Above most individual squad
            // beats because it is about all of them at once.
            NewsStoryKind::SquadKnitsTogether => 372,
            NewsStoryKind::TurnoverToll => 388,
            NewsStoryKind::CliqueConcerns => 396,
            // What the manager privately thinks, inferred from the press
            // box rather than said out loud by anybody.
            NewsStoryKind::BigMatchTrust => 342,
            NewsStoryKind::ManagerDoubtsLinger => 356,
            NewsStoryKind::TakenUnderWing => 233,

            // ── Everything else a real back page carries ──────────
            // The season's individual honours. Above every weekly
            // beat: a player of the year is a front page anywhere.
            NewsStoryKind::SeasonAward => 442,
            // The move he would have taken for nothing. Bigger than the
            // transfer report beside it, which only knows the fee.
            NewsStoryKind::DreamMoveComplete => 470,
            NewsStoryKind::StatusShock => 340,
            NewsStoryKind::PayWindfall => 330,
            NewsStoryKind::WageRealityCheck => 336,
            // The week's award, and the young one. Smaller than the
            // month, and the only honour most players ever collect.
            NewsStoryKind::PlayerOfWeek => 372,
            NewsStoryKind::YoungPlayerOfWeek => 368,
            NewsStoryKind::TeamOfMonthNod => 396,
            NewsStoryKind::YoungPlayerOfMonthAward => 430,
            // A season's verdict on a kid. One of the few individual
            // honours a town remembers the year of.
            NewsStoryKind::YoungPlayerOfSeasonAward => 560,
            NewsStoryKind::NewManagerBounce => 348,
            NewsStoryKind::ManagerExitUnsettles => 342,
            // Somebody at the club said a thing and then did it. Quieter
            // than the broken promise the page has always printed, and
            // the reason a dressing room believes the next one.
            NewsStoryKind::PromiseKept => 358,
            NewsStoryKind::BanServed => 286,
            NewsStoryKind::PlayingForContract => 322,
            NewsStoryKind::PersonalTrainingPlan => 250,
            NewsStoryKind::RoleRetraining => 262,
            // A local kid behind an import, and a squad list that says so
            // out loud. The grievance a local readership owns.
            NewsStoryKind::HomegrownBlocked => 334,
            NewsStoryKind::FavouritismGrumbles => 344,
            // A footballer's family is the half of a transfer nobody
            // negotiates and the half that decides most of them.
            NewsStoryKind::FamilyUnsettled => 316,
            NewsStoryKind::FamilyCelebration => 306,
            // Reported plainly and high, the way a paper reports a
            // bereavement: above the week's ordinary squad news and
            // written without an exclamation mark anywhere near it.
            NewsStoryKind::CompassionateLeave => 372,
            NewsStoryKind::LanguageLessons => 244,
            NewsStoryKind::VeteranHomecomingWish => 388,
            NewsStoryKind::LegendWontLeave => 396,
            // He said no to the neighbours, and to a better contract at
            // the same time. A supporter will forgive him a great deal
            // for this one.
            NewsStoryKind::RefusesRivalMove => 428,
            NewsStoryKind::SeeksQuieterStage => 352,
            // Off-field trouble. Every paper's favourite story.
            NewsStoryKind::OffFieldControversy => 427,
            NewsStoryKind::ContractTornUp => 403,
            // His country has stopped picking him — the quiet end of
            // an international career, and it runs a shade under the
            // call-up that started it.
            NewsStoryKind::DroppedByCountry => 329,
            NewsStoryKind::OutgrownDivision => 322,
            // Ineligible rather than dropped: worse, and it takes a
            // window to undo.
            NewsStoryKind::LeftOutOfSquadList => 313,
            NewsStoryKind::BenchFrustration => 303,
            NewsStoryKind::WageEnvy => 297,
            NewsStoryKind::RelegationNerves => 287,
            NewsStoryKind::TrainingConcerns => 247,
            NewsStoryKind::ShirtNumberHandover => 228,
            // The week's quietest story, and the one that most often
            // comes before a run in the side.
            NewsStoryKind::TrainingGroundBuzz => 213,
            // The manager has changed the shape and stuck with it. Read
            // off the match log rather than announced anywhere, which is
            // also how a supporter works it out.
            NewsStoryKind::FormationRevolution => 388,
        }
    }

    /// How the editor decides whether a story has already been printed.
    pub fn recurrence(self) -> NewsRecurrence {
        match self {
            // Something that happened on a date. It can only be
            // detected once, so the back catalogue is never consulted.
            NewsStoryKind::LeagueWin
            | NewsStoryKind::LeagueDraw
            | NewsStoryKind::GoallessDraw
            | NewsStoryKind::LeagueDefeat
            | NewsStoryKind::Rout
            | NewsStoryKind::HeavyDefeat
            | NewsStoryKind::DerbyWin
            | NewsStoryKind::DerbyDefeat
            | NewsStoryKind::CupWin
            | NewsStoryKind::CupExit
            // How one afternoon went. Read once off that week's goal
            // feed, and never true of the same match twice.
            | NewsStoryKind::LateWinner
            | NewsStoryKind::StoppageTimeDrama
            | NewsStoryKind::ComebackWin
            | NewsStoryKind::LeadThrownAway
            | NewsStoryKind::InstantReply
            | NewsStoryKind::EarlyBlitz
            | NewsStoryKind::GoalFest
            | NewsStoryKind::TenManWin
            | NewsStoryKind::HatTrick
            // A goalkeeper's afternoon, read once off that week's stat
            // lines — and the award, which is handed out once a season.
            | NewsStoryKind::KeeperMasterclass
            | NewsStoryKind::KeeperPenaltySave
            | NewsStoryKind::KeeperOverrun
            | NewsStoryKind::KeeperBlunder
            | NewsStoryKind::KeeperGoldenGlove
            | NewsStoryKind::RedCard
            | NewsStoryKind::PlayerOfMonth
            | NewsStoryKind::TeamOfTheWeek
            | NewsStoryKind::CaptainNamed
            | NewsStoryKind::NationalCallUp
            | NewsStoryKind::DroughtEnded
            | NewsStoryKind::FormerClubGoal
            | NewsStoryKind::ClubServant
            | NewsStoryKind::RetirementAnnounced
            | NewsStoryKind::TrainingBustUp
            | NewsStoryKind::ContractRenewed
            | NewsStoryKind::FansChant
            | NewsStoryKind::PromiseBroken
            | NewsStoryKind::PlayerFined
            | NewsStoryKind::ClearTheAir
            | NewsStoryKind::CaptaincyLost
            // Read from the seven-day feed on a seven-day tick, and
            // emitted once per arrival / audit pass — the same shape as
            // the bust-up and fine beats above.
            | NewsStoryKind::ShirtUnderThreat
            | NewsStoryKind::PathwayBlocked
            | NewsStoryKind::RoleFrustration
            | NewsStoryKind::DroppedForBigMatch
            | NewsStoryKind::DressingRoomInquest
            | NewsStoryKind::SquadRallies
            | NewsStoryKind::BoardInvests
            | NewsStoryKind::NewSigning
            | NewsStoryKind::RecordSigning
            | NewsStoryKind::FreeSigning
            | NewsStoryKind::LoanArrival
            // Completed business, read from the week's transfer ledger
            // exactly once — same shape as every other arrival.
            | NewsStoryKind::HomecomingSigning
            | NewsStoryKind::LoanMadePermanent
            | NewsStoryKind::ProspectSigned
            | NewsStoryKind::VeteranArrives
            // The same ledger row read for its motive rather than its
            // fee. Still one row, still one day, still one edition.
            | NewsStoryKind::RivalRaid
            | NewsStoryKind::ScoutingCoup
            | NewsStoryKind::SuccessionSigning
            | NewsStoryKind::DepthSigning
            | NewsStoryKind::GapPlugged
            | NewsStoryKind::MarqueeUpgrade
            | NewsStoryKind::BargainBuy
            | NewsStoryKind::AcademyGraduate
            | NewsStoryKind::LoanedOutToGrow
            | NewsStoryKind::PlayerSold
            | NewsStoryKind::StarSold
            | NewsStoryKind::FreeExit
            | NewsStoryKind::LoanExit
            | NewsStoryKind::LoanReturn
            // The dugout and the boardroom, read from the club's own
            // dated diary rather than from the state they leave behind.
            // A log entry belongs to one day, so it belongs to exactly
            // one edition — which is what makes these safe as `Event`
            // where the old state-scraped versions were not.
            | NewsStoryKind::ManagerSacked
            | NewsStoryKind::NewManagerArrives
            | NewsStoryKind::ManagerPoached
            | NewsStoryKind::CaretakerTakesCharge
            | NewsStoryKind::CaretakerConfirmed
            | NewsStoryKind::ManagerUltimatum
            | NewsStoryKind::ManagerContractExtended
            | NewsStoryKind::TakeoverRumour
            | NewsStoryKind::TakeoverCompleted
            | NewsStoryKind::TakeoverCollapsed
            | NewsStoryKind::StadiumExpansion
            | NewsStoryKind::FacilityUpgrade
            | NewsStoryKind::WarChest
            | NewsStoryKind::BudgetCut
            | NewsStoryKind::BoardPromiseBroken
            // The balance sheet's dated moments, from the same diary:
            // the day the club went in, the day it came out, the day the
            // owner wired the money, the day a sponsor signed or walked.
            // The CONDITIONS those leave behind (the debt, the embargo,
            // the wage bill) are `Standing` and live further down.
            | NewsStoryKind::AdministrationEntered
            | NewsStoryKind::AdministrationExited
            | NewsStoryKind::OwnerBailout
            | NewsStoryKind::SponsorSigned
            | NewsStoryKind::SponsorshipLost
            // A clause fires on the day it fires and is retired by the
            // settler, so it can only ever be read once.
            | NewsStoryKind::SellOnWindfall
            | NewsStoryKind::PromotionBonusDue
            | NewsStoryKind::PreContractAgreed
            | NewsStoryKind::BosmanDepartureLooms
            // Squad life read from the seven-day feed on a seven-day
            // tick — the same shape as the fine and bust-up beats.
            | NewsStoryKind::FirstClubGoal
            | NewsStoryKind::WonStartingPlace
            | NewsStoryKind::LeaderEmerging
            | NewsStoryKind::TeammateFarewell
            | NewsStoryKind::InternationalRetirement
            // Season verdicts fire once from the players' event feeds,
            // the same Monday the whole squad carries them.
            | NewsStoryKind::PromotionWon
            | NewsStoryKind::RelegationConfirmed
            | NewsStoryKind::CupFinalHeartbreak
            | NewsStoryKind::EuropeSecured
            | NewsStoryKind::TrophyWon
            // The scoring charts are read off a frozen monthly snapshot,
            // which exists exactly once per calendar month. A chart is
            // not a status that lingers and it is not a tally that ticks
            // — it is a table somebody closed on the last day of the
            // month, so it belongs to that month's edition and no other.
            | NewsStoryKind::LeagueTopScorer
            | NewsStoryKind::LeagueScoringChase
            // The whole ratings page. Every one of these is read off ONE
            // afternoon's stat line out of the week just played — the
            // same channel, and the same guarantee, as the goalkeeper
            // beats above: the match happened on a date, its numbers
            // never change again, and next Monday's facts are built from
            // next week's fixtures. A verdict on Saturday is never
            // re-detected, so the back catalogue is irrelevant to it.
            | NewsStoryKind::ManOfTheMatch
            | NewsStoryKind::MatchMasterclass
            | NewsStoryKind::AssistShow
            | NewsStoryKind::CreatorInChief
            | NewsStoryKind::DefensiveRock
            | NewsStoryKind::DribblingDisplay
            | NewsStoryKind::SuperSub
            | NewsStoryKind::DerbyHero
            | NewsStoryKind::MatchStinker
            | NewsStoryKind::WastefulFinishing
            | NewsStoryKind::CostlyError
            | NewsStoryKind::OwnGoalShame
            | NewsStoryKind::PenaltyMissed
            | NewsStoryKind::HookedEarly
            | NewsStoryKind::FoulTrouble
            // Squad life that happens on a day and is read from the
            // seven-day feed on a seven-day tick — the same shape as the
            // fine and bust-up beats above.
            | NewsStoryKind::TeammateConflict
            | NewsStoryKind::DressingRoomSpeech
            | NewsStoryKind::OffFieldControversy
            | NewsStoryKind::ShirtNumberHandover
            | NewsStoryKind::ContractTornUp
            | NewsStoryKind::DroppedByCountry
            | NewsStoryKind::DreamMoveComplete
            | NewsStoryKind::StatusShock
            | NewsStoryKind::PayWindfall
            | NewsStoryKind::WageRealityCheck
            | NewsStoryKind::PlayerOfWeek
            | NewsStoryKind::YoungPlayerOfWeek
            | NewsStoryKind::TeamOfMonthNod
            | NewsStoryKind::YoungPlayerOfMonthAward
            | NewsStoryKind::YoungPlayerOfSeasonAward
            | NewsStoryKind::NewManagerBounce
            | NewsStoryKind::ManagerExitUnsettles
            | NewsStoryKind::PromiseKept
            | NewsStoryKind::BanServed
            | NewsStoryKind::PersonalTrainingPlan
            | NewsStoryKind::RoleRetraining
            | NewsStoryKind::HomegrownBlocked
            | NewsStoryKind::FavouritismGrumbles
            | NewsStoryKind::LeverageUsed
            | NewsStoryKind::FamilyCelebration
            | NewsStoryKind::CompassionateLeave
            | NewsStoryKind::BraceHero
            | NewsStoryKind::GoalAndAssistShow
            | NewsStoryKind::TeenageStarTurn
            | NewsStoryKind::RolledBackYears
            | NewsStoryKind::DreamDebut
            | NewsStoryKind::DebutNightmare
            | NewsStoryKind::GoalFromDefence
            | NewsStoryKind::MidfieldEngine
            | NewsStoryKind::CrisisTalks
            | NewsStoryKind::BoardDemandsSale
            | NewsStoryKind::BoardBlocksDeal
            | NewsStoryKind::FacilityPlanRejected
            | NewsStoryKind::LeaguePlayerOfMonth
            | NewsStoryKind::LeagueYoungStar
            | NewsStoryKind::LeagueAssistKing
            | NewsStoryKind::LeagueAssistChase
            | NewsStoryKind::LeagueRatingsLeader
            | NewsStoryKind::LeagueTeamOfMonth
            | NewsStoryKind::FansFurySale
            | NewsStoryKind::FansDreamSigning
            | NewsStoryKind::CupHumiliationFallout
            | NewsStoryKind::TravellingSupportRewarded
            | NewsStoryKind::AcademyDarling
            | NewsStoryKind::TrainingStandardsRow
            | NewsStoryKind::PositionRivalryFeud
            | NewsStoryKind::LeadershipPowerStruggle
            | NewsStoryKind::RotationRested
            | NewsStoryKind::TacticalOmission
            | NewsStoryKind::DroppedOnForm
            | NewsStoryKind::DisciplinaryOmission
            | NewsStoryKind::ForcedToRetire
            | NewsStoryKind::LoanReturnTriumph
            | NewsStoryKind::LoanReturnWasted
            | NewsStoryKind::HamstringBlow
            | NewsStoryKind::KneeLigamentBlow
            | NewsStoryKind::BrokenBoneBlow
            | NewsStoryKind::IntakeDay
            | NewsStoryKind::GoldenGeneration
            | NewsStoryKind::GraduationDay
            | NewsStoryKind::ContinentalNightWin
            | NewsStoryKind::ContinentalDefeat
            | NewsStoryKind::ContinentalRout
            | NewsStoryKind::ContinentalHiding
            | NewsStoryKind::PlayoffGameWin
            | NewsStoryKind::PlayoffGameDefeat
            | NewsStoryKind::PlayoffTieWon
            | NewsStoryKind::PlayoffTieLost
            | NewsStoryKind::PlayoffFinalReached
            | NewsStoryKind::BackroomEmpty
            | NewsStoryKind::TournamentTriumph
            | NewsStoryKind::TournamentHeartbreak
            | NewsStoryKind::SeasonAward => NewsRecurrence::Event,

            // A number that moves. The paper runs it again as soon as
            // the number does — "make it five in a row".
            NewsStoryKind::WinningRun
            | NewsStoryKind::UnbeatenRun
            | NewsStoryKind::WinlessRun
            // The same shape of number, counted off the match log:
            // weeks without scoring, weeks shipping them, and the two
            // runs that have an address on them.
            | NewsStoryKind::GoalsDriedUp
            | NewsStoryKind::DefensiveCrisis
            | NewsStoryKind::FortressHome
            | NewsStoryKind::AwayDayForm
            | NewsStoryKind::StarForm
            | NewsStoryKind::KeeperWall
            | NewsStoryKind::MilestoneApps
            | NewsStoryKind::MilestoneGoals
            | NewsStoryKind::GoalDrought
            // Shut-outs arrive occasionally, never weekly — the shape
            // `Progress` is for.
            | NewsStoryKind::KeeperShutoutMilestone
            // `Progress` is right only when the figure moves
            // OCCASIONALLY — a goal, another win in a run. A figure that
            // moves EVERY week (appearances) makes a fresh key every
            // week, which is `Event` behaviour wearing a different name:
            // the story reruns each Monday with the number nudged by
            // one. Anything counting appearances belongs in `Standing`.
            | NewsStoryKind::LoanWatchGoals => NewsRecurrence::Progress,

            // A condition that persists. Printing it every week would
            // read like a stuck record, so it waits its turn.
            NewsStoryKind::TitleCharge
            | NewsStoryKind::RelegationFight
            | NewsStoryKind::InjuryBlow
            | NewsStoryKind::InjuryReturn
            | NewsStoryKind::InjurySetback
            | NewsStoryKind::Suspension
            | NewsStoryKind::YouthDebut
            | NewsStoryKind::RisingStar
            | NewsStoryKind::TransferSpeculation
            | NewsStoryKind::TransferListed
            | NewsStoryKind::ContractStandoff
            | NewsStoryKind::RumourInterest
            | NewsStoryKind::RumourRival
            | NewsStoryKind::ScoutsWatching
            | NewsStoryKind::AgentTouting
            | NewsStoryKind::HomecomingLink
            | NewsStoryKind::TalksExpected
            | NewsStoryKind::ToldNotInPlans
            | NewsStoryKind::ContractTalksStalled
            // Read from a status that persists for months, or from the
            // 16-day event window the rumour and verdict desks use on a
            // 7-day tick. Either way the same fact is re-detected next
            // Monday, so the back catalogue has to be consulted.
            | NewsStoryKind::TransferRequestFiled
            | NewsStoryKind::BidRejected
            | NewsStoryKind::MoveCollapsed
            // `Trn` sits on the player until the deal completes, the
            // cooled-interest and loyalty-pledge beats come off the
            // 16-day feed, and a run-down contract stays run down —
            // all re-detected every Monday.
            | NewsStoryKind::TransferAgreed
            | NewsStoryKind::RumourCools
            | NewsStoryKind::CommitsToClub
            | NewsStoryKind::ContractRunningDown
            | NewsStoryKind::AmbitionWarning
            | NewsStoryKind::UnsoldStillHere
            | NewsStoryKind::CareerTwilight
            | NewsStoryKind::LoanFedUp
            | NewsStoryKind::SigningNotWorking
            | NewsStoryKind::SigningComesGood
            | NewsStoryKind::LoanFlop
            | NewsStoryKind::LoanWatchStarter
            | NewsStoryKind::LoanWatchBenched
            | NewsStoryKind::LoanWantsReturn
            | NewsStoryKind::LoanWantsPermanent
            | NewsStoryKind::LoanRecallTalk
            | NewsStoryKind::LoanSpellEnds
            | NewsStoryKind::LoanStepTooBig
            | NewsStoryKind::ManagerBacksPlayer
            | NewsStoryKind::ManagerCallsOutPlayer
            | NewsStoryKind::FansTurnOnTeam
            | NewsStoryKind::FansGetBehind
            | NewsStoryKind::FansAngryAtRumour
            | NewsStoryKind::MediaPressure
            | NewsStoryKind::MediaDarling
            | NewsStoryKind::ManagerPressure
            | NewsStoryKind::BoardBacking
            // An open vacancy and an in-flight approach both persist:
            // the search runs for weeks and an approach lives for days
            // across a weekly tick, so both are re-detected on the next
            // Monday and must consult the back catalogue.
            | NewsStoryKind::ManagerHunt
            | NewsStoryKind::ManagerTargetLinked
            | NewsStoryKind::ManagerWanted
            // Settling in and eyeing the coaching badges are conditions
            // rather than days, and both are read from the fortnight
            // window on a seven-day tick.
            | NewsStoryKind::SettlingIn
            | NewsStoryKind::CoachingAmbition
            | NewsStoryKind::AcademyPraise
            | NewsStoryKind::MoneyWorries
            // What a balance sheet leaves behind rather than what it
            // did on a day. All four are true for months at a time, and
            // a paper that ran the debt figure every Monday would read
            // like a stuck record about the one subject nobody enjoys.
            | NewsStoryKind::DebtMountain
            | NewsStoryKind::TransferEmbargo
            | NewsStoryKind::WageBillCrisis
            | NewsStoryKind::MustSellBeforeBuying
            // A foreign player's life. Not one of these is a Tuesday:
            // homesickness, isolation, a dressing room that has not
            // forgiven where he came from, a senior pro quietly looking
            // after him — all conditions, all read from the fortnight
            // window on a seven-day tick, all re-detected next Monday.
            | NewsStoryKind::HomesickAbroad
            | NewsStoryKind::StrugglingToSettle
            | NewsStoryKind::HomeCalling
            | NewsStoryKind::SettledAtLast
            | NewsStoryKind::ColdShoulder
            | NewsStoryKind::TakenUnderWing
            // …and the same for how he is training, how much he is
            // playing, what he thinks of his wages, and whether the
            // division is still big enough for him. Every one is a state
            // he is in rather than something that happened, so a paper
            // that ran them weekly would print the same man's grievance
            // every Monday until it was resolved.
            | NewsStoryKind::TrainingGroundBuzz
            | NewsStoryKind::TrainingConcerns
            | NewsStoryKind::BenchFrustration
            | NewsStoryKind::WageEnvy
            | NewsStoryKind::OutgrownDivision
            | NewsStoryKind::RelegationNerves
            // Ineligibility lasts as long as the registered list does.
            | NewsStoryKind::PlayingForContract
            | NewsStoryKind::ContinentalAmbition
            | NewsStoryKind::FamilyUnsettled
            | NewsStoryKind::LanguageLessons
            | NewsStoryKind::VeteranHomecomingWish
            | NewsStoryKind::LegendWontLeave
            | NewsStoryKind::RefusesRivalMove
            | NewsStoryKind::SeeksQuieterStage
            | NewsStoryKind::ProtestBrewing
            | NewsStoryKind::PromotionFever
            | NewsStoryKind::RelegationDread
            | NewsStoryKind::HomegrownQuotaOmission
            | NewsStoryKind::ForeignQuotaOmission
            | NewsStoryKind::SquadKnitsTogether
            | NewsStoryKind::TurnoverToll
            | NewsStoryKind::CliqueConcerns
            | NewsStoryKind::BigMatchTrust
            | NewsStoryKind::ManagerDoubtsLinger
            | NewsStoryKind::FormationRevolution
            | NewsStoryKind::BreakthroughSeason
            | NewsStoryKind::TrainingTransformation
            | NewsStoryKind::StalledProspect
            | NewsStoryKind::PowersFading
            | NewsStoryKind::LeftOutOfSquadList => NewsRecurrence::Standing,
        }
    }

    /// True when several stories of this kind can share one edition.
    /// Match reports can (a club plays twice in a week); a paper never
    /// runs two "board backs the manager" pieces side by side.
    pub fn allows_repeat(self) -> bool {
        matches!(
            self,
            NewsStoryKind::PlayoffGameWin
                |             NewsStoryKind::PlayoffGameDefeat
                |             NewsStoryKind::ContinentalNightWin
                |             NewsStoryKind::ContinentalDefeat
                |             NewsStoryKind::ContinentalRout
                |             NewsStoryKind::ContinentalHiding
                |             NewsStoryKind::LeagueAssistChase
                |             NewsStoryKind::LeagueTeamOfMonth
                |             NewsStoryKind::BraceHero
                |             NewsStoryKind::GoalAndAssistShow
                |             NewsStoryKind::GoalFromDefence
                |             NewsStoryKind::MidfieldEngine
                |             NewsStoryKind::LeagueWin
                | NewsStoryKind::LeagueDraw
                | NewsStoryKind::GoallessDraw
                | NewsStoryKind::LeagueDefeat
                | NewsStoryKind::Rout
                | NewsStoryKind::HeavyDefeat
                | NewsStoryKind::DerbyWin
                | NewsStoryKind::DerbyDefeat
                | NewsStoryKind::CupWin
                // A club that plays twice in a week can have two
                // afternoons worth telling this way, and the desk
                // already holds each of them to a single angle.
                | NewsStoryKind::LateWinner
                | NewsStoryKind::StoppageTimeDrama
                | NewsStoryKind::ComebackWin
                | NewsStoryKind::LeadThrownAway
                | NewsStoryKind::InstantReply
                | NewsStoryKind::EarlyBlitz
                | NewsStoryKind::GoalFest
                | NewsStoryKind::TenManWin
                | NewsStoryKind::HatTrick
                | NewsStoryKind::InjuryBlow
                | NewsStoryKind::NewSigning
                | NewsStoryKind::PlayerSold
                | NewsStoryKind::LoanArrival
                | NewsStoryKind::LoanExit
                | NewsStoryKind::FreeExit
                | NewsStoryKind::LoanReturn
                | NewsStoryKind::FreeSigning
                | NewsStoryKind::HomecomingSigning
                | NewsStoryKind::LoanMadePermanent
                | NewsStoryKind::ProspectSigned
                | NewsStoryKind::VeteranArrives
                // A window week brings several of each of these, and a
                // paper lists them rather than picking one.
                | NewsStoryKind::RivalRaid
                | NewsStoryKind::ScoutingCoup
                | NewsStoryKind::SuccessionSigning
                | NewsStoryKind::DepthSigning
                | NewsStoryKind::GapPlugged
                | NewsStoryKind::MarqueeUpgrade
                | NewsStoryKind::BargainBuy
                | NewsStoryKind::AcademyGraduate
                | NewsStoryKind::LoanedOutToGrow
                | NewsStoryKind::NationalCallUp
                // Two players can both be off the mark in the same
                // week, and each of them has waited for it.
                | NewsStoryKind::FirstClubGoal
                | NewsStoryKind::LoanWatchGoals
                | NewsStoryKind::LoanWatchStarter
                | NewsStoryKind::SigningNotWorking
                // A scoring chart is a list. Printing one entry of it
                // and calling that the charts is a result, not a table.
                | NewsStoryKind::LeagueScoringChase
                // A ratings column names several men, which is the whole
                // point of one: after a 4-0 a paper marks down the back
                // four together, and after a 4-0 the other way it raves
                // about three of them. The desk allowance still caps the
                // section at four lines, so letting these repeat widens
                // the column without ever widening the page.
                | NewsStoryKind::ManOfTheMatch
                | NewsStoryKind::MatchMasterclass
                | NewsStoryKind::MatchStinker
                | NewsStoryKind::CostlyError
                | NewsStoryKind::WastefulFinishing
                | NewsStoryKind::DefensiveRock
        )
    }

    /// True when this kind's copy quotes a season rating (`{rating}`,
    /// read from `NewsStory::b`).
    ///
    /// A rating of zero is not a bad rating, it is *no* rating — a
    /// player who has never been on the pitch reads 0.00 — and a
    /// headline built on one is a sentence the paper cannot stand
    /// behind. The editor refuses these outright rather than trusting
    /// every desk to remember. Kept in lockstep with the translation
    /// bundles by `a_kind_that_quotes_a_figure_declares_it` (web).
    pub fn quotes_a_rating(self) -> bool {
        // `KeeperWall` deliberately absent: it carries a rating in `b`
        // but its copy is written on shut-outs, which is how a
        // goalkeeper's season is actually measured.
        matches!(
            self,
            NewsStoryKind::LeaguePlayerOfMonth
                |             NewsStoryKind::LeagueYoungStar
                |             NewsStoryKind::LeagueAssistKing
                |             NewsStoryKind::LeagueAssistChase
                |             NewsStoryKind::LeagueRatingsLeader
                |             NewsStoryKind::LeagueTeamOfMonth
                |             NewsStoryKind::BraceHero
                |             NewsStoryKind::GoalAndAssistShow
                |             NewsStoryKind::TeenageStarTurn
                |             NewsStoryKind::RolledBackYears
                |             NewsStoryKind::DreamDebut
                |             NewsStoryKind::DebutNightmare
                |             NewsStoryKind::GoalFromDefence
                |             NewsStoryKind::MidfieldEngine
                |             NewsStoryKind::StarForm
                | NewsStoryKind::RisingStar
                | NewsStoryKind::TeamOfTheWeek
                | NewsStoryKind::LoanWatchStarter
                | NewsStoryKind::LoanRecallTalk
                | NewsStoryKind::LoanFlop
                | NewsStoryKind::SigningNotWorking
                | NewsStoryKind::SigningComesGood
                // The ratings page quotes a MATCH rating rather than a
                // season average — the number out of ten beside a name,
                // which is the one figure a ratings column cannot be
                // written without. Same slot (`b`), same guarantee: a
                // player who was not on the pitch has no mark, and the
                // editor refuses a verdict that would print 0.00.
                | NewsStoryKind::ManOfTheMatch
                | NewsStoryKind::MatchMasterclass
                | NewsStoryKind::MatchStinker
                | NewsStoryKind::HookedEarly
        )
    }

    /// True when this kind's copy quotes a transfer fee (`{fee}`, read
    /// from `NewsStory::money`). Same contract as
    /// [`Self::quotes_a_rating`]: "sold for $0.00" is the line that
    /// tells a reader the page was generated rather than written.
    pub fn quotes_a_fee(self) -> bool {
        matches!(
            self,
            NewsStoryKind::FansFurySale
                |             NewsStoryKind::FansDreamSigning
                |             NewsStoryKind::NewSigning
                | NewsStoryKind::RecordSigning
                // Both of these are about the money: what the club
                // decided he was worth, and what it decided it had got
                // away with. Neither can print without a real fee.
                | NewsStoryKind::MarqueeUpgrade
                | NewsStoryKind::BargainBuy
                | NewsStoryKind::PlayerSold
                | NewsStoryKind::StarSold
                // The boardroom's money. "The board have released
                // $0.00" is the same sentence nobody can stand behind.
                | NewsStoryKind::WarChest
                | NewsStoryKind::BudgetCut
                // …and the same contract on the other side of the
                // ledger: a bailout of nothing, a sponsor worth nothing
                // and a debt of nothing are each a sentence the paper
                // cannot stand behind, so the editor refuses them.
                | NewsStoryKind::OwnerBailout
                | NewsStoryKind::SponsorSigned
                | NewsStoryKind::DebtMountain
                // A windfall of nothing is not a windfall.
                | NewsStoryKind::SellOnWindfall
                | NewsStoryKind::PromotionBonusDue
        )
    }

    /// True when this kind's copy names somebody from the dugout
    /// (`{manager}`, read from [`NewsStory::staff_id`]).
    ///
    /// Same lockstep contract as [`Self::quotes_a_fee`], for a different
    /// failure: a kind that carries a staff id nobody wrote into the
    /// copy prints an anonymous "the manager" for a man the paper knows
    /// the name of, and a kind whose copy names a manager it was never
    /// given prints the fallback for every club in the world. Pinned to
    /// the bundles by `a_kind_that_names_a_manager_declares_it` (web).
    pub fn names_a_manager(self) -> bool {
        matches!(
            self,
            NewsStoryKind::ManagerSacked
                | NewsStoryKind::NewManagerArrives
                | NewsStoryKind::ManagerPoached
                | NewsStoryKind::CaretakerTakesCharge
                | NewsStoryKind::CaretakerConfirmed
                | NewsStoryKind::ManagerTargetLinked
                | NewsStoryKind::ManagerWanted
                | NewsStoryKind::ManagerUltimatum
                | NewsStoryKind::ManagerContractExtended
        )
    }

    /// True when the body copy is the player talking rather than the
    /// correspondent writing. The page sets these as a pull-quote —
    /// the one piece of typographic furniture that tells a reader at a
    /// glance that somebody actually said this.
    pub fn is_quote(self) -> bool {
        matches!(
            self,
            NewsStoryKind::LoanWantsReturn
                | NewsStoryKind::LoanWantsPermanent
                | NewsStoryKind::LoanFedUp
                | NewsStoryKind::CommitsToClub
                | NewsStoryKind::AmbitionWarning
                | NewsStoryKind::CareerTwilight
                | NewsStoryKind::TransferRequestFiled
                | NewsStoryKind::ToldNotInPlans
                | NewsStoryKind::ContractTalksStalled
                | NewsStoryKind::RetirementAnnounced
                | NewsStoryKind::AgentTouting
                | NewsStoryKind::ContractRenewed
                | NewsStoryKind::NewManagerArrives
                // The interim's first words, the veteran talking about
                // his badges, and the international retirement — all
                // three are statements somebody actually made.
                | NewsStoryKind::CaretakerTakesCharge
                | NewsStoryKind::CoachingAmbition
                | NewsStoryKind::InternationalRetirement
                | NewsStoryKind::PromiseBroken
                | NewsStoryKind::ClearTheAir
                | NewsStoryKind::ManagerCallsOutPlayer
                | NewsStoryKind::FansTurnOnTeam
                // The three a foreign player, a substitute and a captain
                // say out loud. Wanting to go home is the one thing on
                // this page nobody else can say for him — a paper can
                // report bad form, but homesickness only exists once he
                // admits it.
                | NewsStoryKind::HomesickAbroad
                | NewsStoryKind::BenchFrustration
                | NewsStoryKind::DressingRoomSpeech
        )
    }
}

/// One printed item. Deliberately allocation-free: every club in the
/// world keeps five editions on hand, so a story carries identifiers
/// and numbers only and the web layer resolves names, money formats
/// and translated prose at render time.
#[derive(Debug, Clone, Copy)]
pub struct NewsStory {
    pub kind: NewsStoryKind,
    pub date: NaiveDate,
    pub priority: u16,
    /// Player the story is about. `0` when the story is about the club.
    pub player_id: u32,
    /// Opponent team (match reports) or the other club (everything the
    /// market, rumour and loan desks file). `0` when the story has no
    /// second party.
    pub other_id: u32,
    /// The staff member the story names — a manager, a caretaker, a
    /// target. `0` when nobody from the dugout is in it.
    ///
    /// Its own slot rather than a reuse of `player_id`: the two live in
    /// separate id spaces and separate lookups, and a dugout story that
    /// silently resolved a coach id against the player index would name
    /// the wrong man rather than fail.
    pub staff_id: u32,
    /// The side this story was credited to on the day it was filed.
    ///
    /// A club's own paper is its own credit and never needs this: the
    /// nameplate is the club, whoever has since been sold. A division's
    /// paper names a different side in every story, and it used to look
    /// that side up when the page was served — so a back issue quietly
    /// re-credited itself to wherever the subject had moved in the
    /// meantime, and printed "asks to leave" against the club he had
    /// just joined. Frozen at press time instead, exactly as the awards
    /// shelf freezes the club card on a month's scoring chart.
    ///
    /// `0` when no side was recorded — every club paper, and any
    /// subject a press run could not place.
    pub credited_team_id: u32,
    /// Primary figure: goals scored, days out, league position, …
    pub a: i32,
    /// Secondary figure: goals conceded, points, rating × 100, …
    pub b: i32,
    /// Transfer fee or other money amount. `0` when not a money story.
    pub money: i64,
    /// Match reports only: the side this paper covers played at home.
    /// Together with `date` and `other_id` it rebuilds the match
    /// record's id, which is how a scoreline in the copy links to the
    /// match page. Meaningless — and left `false` — on every other desk.
    pub home: bool,
}

impl NewsStory {
    pub fn new(kind: NewsStoryKind, date: NaiveDate) -> Self {
        NewsStory {
            kind,
            date,
            priority: kind.base_priority(),
            player_id: 0,
            other_id: 0,
            staff_id: 0,
            credited_team_id: 0,
            a: 0,
            b: 0,
            money: 0,
            home: false,
        }
    }

    pub fn about(mut self, player_id: u32) -> Self {
        self.player_id = player_id;
        self
    }

    /// Name the man from the dugout this story is about.
    pub fn by_staff(mut self, staff_id: u32) -> Self {
        self.staff_id = staff_id;
        self
    }

    pub fn against(mut self, other_id: u32) -> Self {
        self.other_id = other_id;
        self
    }

    /// Freeze the side this story belongs to, for a paper that carries
    /// more than one. See [`NewsStory::credited_team_id`].
    pub fn credited_to(mut self, team_id: u32) -> Self {
        self.credited_team_id = team_id;
        self
    }

    pub fn with_numbers(mut self, a: i32, b: i32) -> Self {
        self.a = a;
        self.b = b;
        self
    }

    pub fn with_money(mut self, money: i64) -> Self {
        self.money = money;
        self
    }

    /// Match reports only: which end of the fixture this paper's side
    /// was, so the scoreline can point at the match record.
    pub fn at_home(mut self, home: bool) -> Self {
        self.home = home;
        self
    }

    /// Nudge newsworthiness away from the kind's baseline. Saturating
    /// on both ends so a stack of modifiers can never wrap.
    pub fn weighted(mut self, delta: i32) -> Self {
        let next = self.priority as i32 + delta;
        self.priority = next.clamp(0, u16::MAX as i32) as u16;
        self
    }

    pub fn desk(&self) -> NewsDesk {
        self.kind.desk()
    }
}

/// Which competition a result was played in. The ruled results column
/// marks the cup ties, because "lost 0-1" reads very differently when
/// it is the round the club went out in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultCompetition {
    League,
    Cup,
    /// A European or South American night. Its own label rather than a
    /// cup tie: the group stage is a league inside a knockout, and the
    /// results rail is the one place a supporter looks to check which
    /// of the three competitions a midweek scoreline belonged to.
    Continental,
    /// A playoff game. Its own label because the stakes are the whole
    /// point: the same 1-0 that is a routine Saturday in April decides
    /// a season in May, and the ruled column is where a reader checks
    /// which of the two he is looking at.
    Playoff,
}

impl ResultCompetition {
    /// Suffix the results column uses as a competition mark.
    pub fn slug(self) -> &'static str {
        match self {
            ResultCompetition::League => "league",
            ResultCompetition::Cup => "cup",
            ResultCompetition::Continental => "continental",
            ResultCompetition::Playoff => "playoff",
        }
    }
}

/// One line in the ruled results panel — the fixtures column every
/// football paper prints regardless of what else happened that week.
#[derive(Debug, Clone, Copy)]
pub struct IssueResult {
    pub date: NaiveDate,
    pub opponent_team_id: u32,
    pub goals_for: u8,
    pub goals_against: u8,
    pub competition: ResultCompetition,
    /// Which end of the fixture this side was. With the date and the
    /// two team ids this is enough to rebuild the match record's id
    /// (`{date}_{home}_{away}`), so the scoreline in the column can
    /// open the match itself.
    pub is_home: bool,
}

impl IssueResult {
    pub fn is_win(&self) -> bool {
        self.goals_for > self.goals_against
    }

    pub fn is_draw(&self) -> bool {
        self.goals_for == self.goals_against
    }

    pub fn is_defeat(&self) -> bool {
        self.goals_for < self.goals_against
    }

    pub fn is_cup(&self) -> bool {
        self.competition == ResultCompetition::Cup
    }

    /// A European or South American night, which the ruled results
    /// column marks differently from a domestic knockout: a supporter
    /// scanning the week wants to know which midweek was which.
    pub fn is_continental(&self) -> bool {
        self.competition == ResultCompetition::Continental
    }

    /// A playoff game — a knockout by another name, and the results
    /// column marks it as one.
    pub fn is_playoff(&self) -> bool {
        self.competition == ResultCompetition::Playoff
    }
}

/// The tone the paper takes this week. Local reporting swings hard with
/// results, and the swing is itself part of the reading experience.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressMood {
    Triumph,
    Upbeat,
    Steady,
    Uneasy,
    Crisis,
}

impl PressMood {
    /// Read the room from the week just gone and the standing pressure
    /// on the club. `form` is the club's recent win ratio (0..1) and
    /// `pressure` is 0..1 with 1 meaning the board is close to acting.
    pub fn read(week: (u8, u8, u8), form: f32, pressure: f32) -> Self {
        let (wins, draws, losses) = week;
        let played = wins + draws + losses;

        if pressure >= 0.80 && form < 0.30 {
            return PressMood::Crisis;
        }
        if played > 0 && losses == played && form < 0.40 {
            return PressMood::Crisis;
        }
        // A perfect week, but the stamp is reserved: either the club won
        // more than once, or it is in the middle of a run good enough
        // that a single win still reads as a celebration. Otherwise a
        // routine Saturday victory would carry the same banner as a
        // title-winning month, and the banner would stop meaning
        // anything.
        if played > 0 && wins == played && form >= 0.55 && (played >= 2 || form >= 0.75) {
            return PressMood::Triumph;
        }
        if form >= 0.60 || (wins > losses && form >= 0.45) {
            return PressMood::Upbeat;
        }
        if pressure >= 0.60 || form < 0.25 {
            return PressMood::Uneasy;
        }
        PressMood::Steady
    }

    pub fn i18n_key(self) -> &'static str {
        match self {
            PressMood::Triumph => "press_mood_triumph",
            PressMood::Upbeat => "press_mood_upbeat",
            PressMood::Steady => "press_mood_steady",
            PressMood::Uneasy => "press_mood_uneasy",
            PressMood::Crisis => "press_mood_crisis",
        }
    }

    /// CSS modifier suffix so the sheet can carry the week's tone.
    pub fn slug(self) -> &'static str {
        match self {
            PressMood::Triumph => "triumph",
            PressMood::Upbeat => "upbeat",
            PressMood::Steady => "steady",
            PressMood::Uneasy => "uneasy",
            PressMood::Crisis => "crisis",
        }
    }

    /// Only the extremes earn the front-page stamp. A steady week gets
    /// no editorial flourish at all — that restraint is what makes the
    /// crisis stamp mean something when it appears.
    pub fn is_stamped(self) -> bool {
        matches!(self, PressMood::Triumph | PressMood::Crisis)
    }
}

/// A single printed edition, frozen at publication. Later events never
/// rewrite an issue — an old paper says what it said on the day.
#[derive(Debug, Clone)]
pub struct NewspaperIssue {
    /// Consecutive edition number, starting at 1 for a club's first
    /// printed paper.
    pub number: u32,
    pub date: NaiveDate,
    pub mood: PressMood,
    pub stories: Vec<NewsStory>,
    pub results: Vec<IssueResult>,
}

impl NewspaperIssue {
    /// The story that gets the front-page treatment.
    pub fn lead(&self) -> Option<&NewsStory> {
        self.stories.first()
    }
}

/// A side's local paper: its masthead and its back numbers. Bounded to
/// [`Self::MAX_ISSUES`] editions, so the world's full team list keeps a
/// fixed and predictable memory cost however long a save runs.
///
/// One per side competing under its own brand — the first team, the B
/// team, the "{Club} 2" reserve side in a real lower division. Each of
/// those plays its own football in its own league in front of its own
/// crowd, so each gets its own masthead rather than a shared club page
/// that only ever reported the first team. Squads without a brand of
/// their own (Reserve, U18..U23) are covered by the first team's paper.
#[derive(Debug, Clone)]
pub struct TeamNewsroom {
    /// Which masthead noun this side's paper uses. Stable for the life
    /// of the team so the title never changes under the reader.
    pub masthead: u8,
    /// Number the next edition will carry.
    pub next_number: u32,
    /// Newest edition first.
    pub issues: VecDeque<NewspaperIssue>,
}

impl TeamNewsroom {
    /// Masthead nouns available in the translation bundles.
    pub const MASTHEAD_COUNT: u8 = 6;
    /// Editions kept on the shelf — the presses run weekly, so a
    /// hundred of them is about two seasons of back numbers.
    ///
    /// Deep enough that the paper is an archive rather than a rolling
    /// window: a transfer, a sacking or a promotion stays findable in
    /// the edition that reported it for as long as anyone is likely to
    /// go looking. A five-week shelf meant the summer's business had
    /// already been thrown out by the time the season it shaped was
    /// under way, which is the one thing a reader actually goes back
    /// for.
    ///
    /// The cost is per branded side, not per club: an edition is
    /// identifiers and numbers only (no strings — the web layer
    /// resolves names and prose at render time), so a full shelf is on
    /// the order of 60 KB per paper.
    pub const MAX_ISSUES: usize = 100;
    /// Stories one edition can hold: a lead, two secondaries, and a
    /// column of briefs deep enough to carry the loan watch and the
    /// rumour mill alongside the week's football.
    pub const MAX_STORIES: usize = 12;

    /// Assign a masthead deterministically from the team id so the same
    /// world always prints the same paper titles, and two sides of one
    /// club rarely share a nameplate.
    pub fn for_team(team_id: u32) -> Self {
        TeamNewsroom {
            masthead: (team_id.wrapping_mul(2_654_435_761) >> 13) as u8 % Self::MASTHEAD_COUNT,
            next_number: 1,
            issues: VecDeque::new(),
        }
    }

    /// File a finished edition, dropping the oldest once the shelf is
    /// full.
    pub fn publish(&mut self, issue: NewspaperIssue) {
        self.issues.push_front(issue);
        while self.issues.len() > Self::MAX_ISSUES {
            self.issues.pop_back();
        }
        self.next_number = self.next_number.saturating_add(1);
    }

    pub fn latest(&self) -> Option<&NewspaperIssue> {
        self.issues.front()
    }

    pub fn is_empty(&self) -> bool {
        self.issues.is_empty()
    }

    /// i18n key for this newsroom's masthead pattern.
    pub fn masthead_key(&self) -> &'static str {
        const KEYS: [&str; TeamNewsroom::MASTHEAD_COUNT as usize] = [
            "masthead_gazette",
            "masthead_chronicle",
            "masthead_herald",
            "masthead_courier",
            "masthead_post",
            "masthead_sentinel",
        ];
        KEYS[(self.masthead as usize) % KEYS.len()]
    }
}

impl Default for TeamNewsroom {
    fn default() -> Self {
        TeamNewsroom::for_team(0)
    }
}

#[cfg(test)]
mod tests {
    use super::{NewsStoryKind, NewspaperIssue, PressMood, TeamNewsroom};
    use chrono::NaiveDate;
    use std::collections::HashSet;

    struct Press;

    impl Press {
        fn issue(number: u32) -> NewspaperIssue {
            NewspaperIssue {
                number,
                date: NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(),
                mood: PressMood::Steady,
                stories: Vec::new(),
                results: Vec::new(),
            }
        }
    }

    #[test]
    fn every_kind_has_its_own_translation_stem() {
        let stems: HashSet<&str> = NewsStoryKind::ALL
            .iter()
            .map(|kind| kind.key_stem())
            .collect();

        assert_eq!(
            stems.len(),
            NewsStoryKind::ALL.len(),
            "two story kinds share a key stem, so one would print the other's copy"
        );
    }

    /// `ALL` is what the locale tests walk, so a kind left out of it
    /// would print a raw translation key on a front page and nothing
    /// would fail until a reader saw it. The compiler enforces the
    /// match arms; only the array length can silently drift.
    ///
    /// A desk is a standing section with a kicker over it, so it has to
    /// have enough to say to be worth naming. Four kinds for the six a
    /// club paper prints; two for `Charts`, which is a table — the man
    /// at the top and the field behind him is the whole of what a
    /// scoring chart contains, and padding it to four would mean
    /// inventing copy to clear a threshold.
    #[test]
    fn every_desk_is_represented_on_the_page() {
        use crate::club::news::types::NewsDesk;

        for desk in NewsDesk::ALL {
            let floor = match desk {
                NewsDesk::Charts => 2,
                _ => 4,
            };
            // The ratings page is the one section that is expected to be
            // wide: it exists because the paper had almost no individual
            // performance in it at all.
            let floor = if desk == NewsDesk::Verdicts { 8 } else { floor };
            let filed = NewsStoryKind::ALL
                .iter()
                .filter(|kind| kind.desk() == desk)
                .count();
            assert!(
                filed >= floor,
                "{:?} has almost nothing to file ({} kinds, needs {})",
                desk,
                filed,
                floor
            );
        }
    }

    /// `NewsRecurrence::Event` means "never consult the back
    /// catalogue" — correct only for a fact the desk can detect exactly
    /// once. Two things break that, and both shipped:
    ///
    /// * a story read off a **status** (`Req`, `Lst`) that sits on the
    ///   player for months, so every Monday re-detects it;
    /// * a story read off the rumour desk's **16-day** event window on
    ///   a **7-day** tick, so two consecutive Mondays see the same
    ///   event.
    ///
    /// Either way the same lead prints week after week — which is
    /// exactly what a reader notices first. Anything sourced that way
    /// must be `Standing` (waits `NewsEditor::MEMORY_ISSUES`) or
    /// `Progress` (waits until its figure moves).
    #[test]
    fn a_story_read_from_a_lingering_source_is_never_a_dated_event() {
        use crate::club::news::types::NewsRecurrence;

        // Everything the rumour and verdict desks file: all of it comes
        // from `RecentEvents::fortnight` or from a persistent status.
        const LINGERING: [NewsStoryKind; 36] = [
            // A foreign player's life and his standing in the building.
            // Every one of these is a condition read from the fortnight
            // window on a weekly tick, so every one is re-detected next
            // Monday — the exact shape that made a transfer request lead
            // the paper twice.
            NewsStoryKind::HomesickAbroad,
            NewsStoryKind::StrugglingToSettle,
            NewsStoryKind::HomeCalling,
            NewsStoryKind::SettledAtLast,
            NewsStoryKind::ColdShoulder,
            NewsStoryKind::TakenUnderWing,
            NewsStoryKind::TrainingGroundBuzz,
            NewsStoryKind::TrainingConcerns,
            NewsStoryKind::BenchFrustration,
            NewsStoryKind::WageEnvy,
            NewsStoryKind::OutgrownDivision,
            NewsStoryKind::RelegationNerves,
            NewsStoryKind::LeftOutOfSquadList,
            NewsStoryKind::TransferAgreed,
            NewsStoryKind::RumourCools,
            NewsStoryKind::CommitsToClub,
            NewsStoryKind::ContractRunningDown,
            NewsStoryKind::MoveCollapsed,
            NewsStoryKind::AmbitionWarning,
            NewsStoryKind::UnsoldStillHere,
            NewsStoryKind::LoanFedUp,
            NewsStoryKind::TransferRequestFiled,
            NewsStoryKind::ToldNotInPlans,
            NewsStoryKind::TransferListed,
            NewsStoryKind::ContractTalksStalled,
            NewsStoryKind::BidRejected,
            NewsStoryKind::TalksExpected,
            NewsStoryKind::RumourRival,
            NewsStoryKind::RumourInterest,
            NewsStoryKind::HomecomingLink,
            NewsStoryKind::AgentTouting,
            NewsStoryKind::ScoutsWatching,
            NewsStoryKind::TransferSpeculation,
            NewsStoryKind::SigningNotWorking,
            NewsStoryKind::SigningComesGood,
            NewsStoryKind::LoanFlop,
        ];

        for kind in LINGERING {
            assert_ne!(
                kind.recurrence(),
                NewsRecurrence::Event,
                "{:?} is re-detected every week; as an Event it would lead the paper every week",
                kind
            );
        }
    }

    /// The other half of the same trap. `Progress` re-runs the moment
    /// its figure moves, which is right for a goal tally and wrong for
    /// an appearance count — appearances move every week, so the story
    /// gets a fresh key every week and behaves exactly like an `Event`.
    #[test]
    fn no_weekly_ticking_figure_drives_a_progress_story() {
        use crate::club::news::types::NewsRecurrence;

        // Kinds whose `a` is a goal / clean-sheet / run-length tally —
        // figures that stand still unless something actually happened.
        const OCCASIONAL: [NewsStoryKind; 14] = [
            NewsStoryKind::KeeperShutoutMilestone,
            NewsStoryKind::WinningRun,
            NewsStoryKind::UnbeatenRun,
            NewsStoryKind::WinlessRun,
            // Run lengths off the same match log: each of these stands
            // still until a result actually changes it, and a run that
            // ends stops printing rather than printing a smaller number.
            NewsStoryKind::GoalsDriedUp,
            NewsStoryKind::DefensiveCrisis,
            NewsStoryKind::FortressHome,
            NewsStoryKind::AwayDayForm,
            NewsStoryKind::StarForm,
            NewsStoryKind::KeeperWall,
            NewsStoryKind::MilestoneApps,
            NewsStoryKind::MilestoneGoals,
            NewsStoryKind::GoalDrought,
            NewsStoryKind::LoanWatchGoals,
        ];

        for kind in NewsStoryKind::ALL
            .iter()
            .filter(|kind| kind.recurrence() == NewsRecurrence::Progress)
        {
            assert!(
                OCCASIONAL.contains(kind),
                "{:?} is Progress but is not on the occasional-figure list — if its number \
                 ticks up every week it will print every week",
                kind
            );
        }
    }

    /// A match report is the correspondent describing ninety minutes;
    /// nobody says it out loud. Every other desk can carry somebody's
    /// words, but a pull-quote over a scoreline is a quote nobody gave.
    #[test]
    fn a_match_report_is_never_somebody_talking() {
        use crate::club::news::types::NewsDesk;

        for kind in NewsStoryKind::ALL.iter().filter(|kind| kind.is_quote()) {
            assert_ne!(
                kind.desk(),
                NewsDesk::Match,
                "{:?} is set as a pull-quote but is a match report",
                kind
            );
        }
    }

    /// The shelf is a fixed depth: the newest edition is always on top,
    /// the oldest falls off the bottom, and the run in between is
    /// unbroken. A save left running for a decade costs exactly what one
    /// left running for a season does.
    #[test]
    fn the_shelf_keeps_a_full_run_and_drops_the_oldest() {
        let mut newsroom = TeamNewsroom::for_team(42);
        let printed = TeamNewsroom::MAX_ISSUES as u32 + 3;

        for number in 1..=printed {
            newsroom.publish(Press::issue(number));
        }

        assert_eq!(newsroom.issues.len(), TeamNewsroom::MAX_ISSUES);
        assert_eq!(newsroom.latest().unwrap().number, printed);
        assert_eq!(
            newsroom.issues.back().unwrap().number,
            printed - TeamNewsroom::MAX_ISSUES as u32 + 1,
            "the shelf holds an unbroken run back from the newest edition"
        );
        assert_eq!(newsroom.next_number, printed + 1);
    }

    /// A paper that has not yet filled its shelf keeps everything it has
    /// printed — the bound is a ceiling, not a window that opens late.
    #[test]
    fn a_young_paper_keeps_every_edition_it_has_printed() {
        let mut newsroom = TeamNewsroom::for_team(42);

        for number in 1..=6 {
            newsroom.publish(Press::issue(number));
        }

        assert_eq!(newsroom.issues.len(), 6);
        assert_eq!(newsroom.issues.back().unwrap().number, 1);
    }

    #[test]
    fn a_masthead_is_stable_and_in_range() {
        for team_id in [1u32, 7, 4242, 999_999] {
            let newsroom = TeamNewsroom::for_team(team_id);
            assert_eq!(newsroom.masthead, TeamNewsroom::for_team(team_id).masthead);
            assert!(newsroom.masthead < TeamNewsroom::MASTHEAD_COUNT);
            assert!(newsroom.masthead_key().starts_with("masthead_"));
        }
    }

    #[test]
    fn a_clean_sweep_reads_as_triumph_and_a_wipeout_as_crisis() {
        assert_eq!(PressMood::read((2, 0, 0), 0.80, 0.10), PressMood::Triumph);
        assert_eq!(PressMood::read((0, 0, 2), 0.10, 0.60), PressMood::Crisis);
        assert_eq!(PressMood::read((0, 1, 0), 0.45, 0.30), PressMood::Steady);
        assert_eq!(PressMood::read((1, 0, 0), 0.65, 0.20), PressMood::Upbeat);
    }

    #[test]
    fn a_board_on_the_brink_overrides_a_quiet_week() {
        assert_eq!(PressMood::read((0, 1, 0), 0.20, 0.90), PressMood::Crisis);
    }
}
