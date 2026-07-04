#[derive(Debug, Clone, PartialEq)]
pub enum HappinessEventType {
    // Manager interactions
    ManagerPraise,
    ManagerDiscipline,
    ManagerPlayingTimePromise,
    ManagerCriticism,
    ManagerEncouragement,
    ManagerTacticalInstruction,
    // Training
    GoodTraining,
    PoorTraining,
    // Match selection
    MatchDropped,
    // Contract & transfers
    ContractOffer,
    ContractRenewal,
    SquadStatusChange,
    LackOfPlayingTime,
    LoanListingAccepted,
    // Injury
    InjuryReturn,
    // Match performance
    PlayerOfTheMatch,
    /// Named the league's Player of the Week — chosen Mondays based on the
    /// previous calendar week's performances. Bigger than POM (which is one
    /// match) and rarer (one player per league per week). Career-visible.
    PlayerOfTheWeek,
    // Team/squad relationship
    TeammateBonding,
    ConflictWithTeammate,
    DressingRoomSpeech,
    SettledIntoSquad,
    FeelingIsolated,
    /// Teammate signed a meaningfully bigger deal and this player noticed —
    /// drags salary_satisfaction. Typically only fires if the friendship
    /// with the newly-signed teammate is low.
    SalaryGapNoticed,
    /// Manager kept a concrete promise (e.g. more playing time).
    PromiseKept,
    /// Manager broke a concrete promise. Big morale hit, erodes trust.
    PromiseBroken,
    /// Fresh transfer landed the player at a club whose reputation sits well
    /// below what his ambition expects. Lingers while the gap exists.
    AmbitionShock,
    /// New contract is dramatically worse than the pre-transfer salary —
    /// e.g. Messi moving to a Maltese club on a 1/100th deal.
    SalaryShock,
    /// Team's primary formation has no slot for the player's preferred
    /// position. Degrades ambition_fit until a compatible role opens.
    RoleMismatch,
    /// Signed for a club well above the player's expectations — an
    /// unambiguous step up (small-club talent joining Barça / Madrid).
    /// Reserved for **permanent** moves whose destination club / league
    /// reputation is materially above the source. Loans use
    /// `DreamLoanOpportunity`; sentimental favourite-club moves that
    /// don't pass the reputation gate use `HomeReturnOpportunity`.
    DreamMove,
    /// High-profile loan to a club whose reputation dwarfs the parent
    /// club — the "loan of a lifetime" framing without claiming a
    /// career-defining move. Magnitude is intentionally lower than
    /// `DreamMove` because loans are temporary.
    DreamLoanOpportunity,
    /// New contract pays materially more than the previous deal — the
    /// positive counterpart to SalaryShock.
    SalaryBoost,
    /// Joined a genuinely elite club (top-tier reputation). Fires only
    /// when the move is also a step up relative to the player's own
    /// reputation, to avoid stacking with DreamMove at mid-table moves.
    JoiningElite,
    /// Club bought the player out of his contract — a mild blow to pride
    /// softened by the severance payout. Emitted on mutual termination.
    ContractTerminated,
    /// Head coach was replaced. Fires per-player: strongly negative for
    /// players who had a close bond with the outgoing manager, mildly
    /// positive for players whose relationship had soured.
    ManagerDeparture,
    /// Called up to the senior national team. Big prestige moment for
    /// younger players, routine for established internationals.
    NationalTeamCallup,
    /// Dropped from the national team squad after previous caps — hurts
    /// pride more than a non-selection would.
    NationalTeamDropped,
    /// Promoted to a prestigious shirt number (1-11, esp. #10 / #7 / #9).
    /// Small ongoing pride boost while the number holds.
    ShirtNumberPromotion,
    /// Had a controversial incident (media or dressing room) — fallout
    /// tied to `controversy` personality attribute.
    ControversyIncident,

    // ── Match performance ────────────────────────────────────────
    /// First competitive goal scored for this club. Career milestone —
    /// one-shot per club, lingers in memory for the season.
    FirstClubGoal,
    /// Scored or assisted a goal that decided a tight match. Bigger
    /// than a routine goal, smaller than POM unless paired with it.
    DecisiveGoal,
    /// Came on as a substitute and made a clear positive impact —
    /// scored, assisted, or finished with a high rating off the bench.
    SubstituteImpact,
    /// Defender or goalkeeper kept a clean sheet. Position-gated —
    /// strikers don't care about clean sheets.
    CleanSheetPride,
    /// Finished a match with a costly low rating, often paired with
    /// a goal conceded the player was directly responsible for.
    CostlyMistake,
    /// Sent off (direct red or two yellows). Lingers as embarrassment
    /// plus the suspension fallout.
    RedCardFallout,
    /// Standout performer in a derby win — scorer, assister, POM, or
    /// high-rated display. Reserved for players who carried the win;
    /// ordinary participants get the squad-wide [`DerbyWin`] instead.
    DerbyHero,
    /// Squad-wide moderate positive for being on the winning side of a
    /// derby. Distinct from [`DerbyHero`], which is reserved for the
    /// match's standout performers.
    DerbyWin,
    /// Lost a derby — meaningfully bigger blow than a generic defeat.
    /// Lingers; rivalry loss isn't shaken off in a week.
    DerbyDefeat,

    // ── Team season events ──────────────────────────────────────
    /// Team won a trophy (league, continental). Major career moment.
    /// Note: a domestic cup win fires the dedicated [`DomesticCupWon`]
    /// variant so its cooldown is independent — a double-winning side
    /// must produce both events on the same player, not collapse one
    /// into the other.
    TrophyWon,
    /// Team won the country's main knockout cup (FA Cup, Copa del Rey,
    /// Coppa Italia, …). Distinct from `TrophyWon` so league + cup in
    /// the same season can both register on a player — the league
    /// title doesn't suppress the cup medal via shared cooldown.
    /// Magnitude lives in [`MoraleEventCatalog::domestic_cup_won`].
    DomesticCupWon,
    /// Team lost a cup final. The flip side of TrophyWon — tournament
    /// runs that ended in heartbreak weigh on a squad.
    CupFinalDefeat,
    /// Team confirmed promotion to a higher division.
    PromotionCelebration,
    /// Team is in the relegation fight late in the season — ambient
    /// dread that builds with the season trajectory.
    RelegationFear,
    /// Team was relegated. Major morale hit, particularly for ambitious
    /// players who'll often want a transfer afterwards.
    Relegated,
    /// Team qualified for European competition — a real boost for
    /// ambitious squads who treat continental football as the floor.
    QualifiedForEurope,

    // ── Role / status ───────────────────────────────────────────
    /// Cemented a place in the starting XI after fighting for it. Fires
    /// once per spell — the moment the manager's trust is established.
    WonStartingPlace,
    /// Lost the starting place to a teammate / new signing. Fires once
    /// per spell on the cusp of being benched, not every dropped match.
    LostStartingPlace,
    /// Awarded the captain's armband. Big prestige and trust signal.
    CaptaincyAwarded,
    /// Stripped of the captain's armband. Wounding — rarely forgotten.
    CaptaincyRemoved,
    /// Young player promoted from academy / development squad to senior
    /// matchday duty for the first time. One-shot career milestone.
    YouthBreakthrough,
    /// Left out of the squad registration list for a competition. Frozen
    /// out of matchday minutes for the duration of that registration window.
    ///
    /// **Reserved.** No emit site exists today — the simulation has
    /// `ForeignPlayerLimits` / `YouthRequirements` placeholders in
    /// `continent::regulations::types`, but no per-club registration list
    /// is enforced and `match_squad` picks XI matchday-by-matchday with
    /// no separate roster gate. When a registration window is added
    /// (continental cup squad lists, foreign-player caps), emit this for
    /// `KeyPlayer` / `FirstTeamRegular` who were expected to be included
    /// but weren't. Do **not** infer it from match-day non-selection —
    /// that's a manager call, not a roster lockout, and a different event.
    SquadRegistrationOmitted,

    // ── Transfer / media ────────────────────────────────────────
    /// Confirmed concrete interest from a club meaningfully bigger than
    /// the current one. Flattery for ambitious players, distraction for
    /// settled ones — replaces the old "manager-encouragement" misuse.
    WantedByBiggerClub,
    /// Bid for the player from another club was rejected by the selling
    /// side. Frustrating for an ambitious player who saw the move coming.
    TransferBidRejected,
    /// A transfer the player was set on collapsed at a late stage —
    /// medical, registration, or club back-out. Lingering bitterness.
    DreamMoveCollapsed,
    /// Scout from a meaningful club has been watching the player. Often
    /// invisible, but lands as a small confidence note when it leaks /
    /// repeats / coincides with an ambition trigger.
    ScoutedByClub,
    /// Loose rumour — the player heard the link but the interested club
    /// has not put concrete weight behind it.
    TransferRumour,
    /// Agent / representatives have been actively stirring interest with
    /// other clubs. Distinct from a leaked club briefing.
    AgentStirsInterest,
    /// Concrete interest from a club well above this player's current
    /// level. Distinct from the legacy `WantedByBiggerClub` in that it
    /// carries the full `TransferInterestContext` payload.
    InterestFromBiggerClub,
    /// Concrete interest from a known sporting rival. Even at lateral
    /// rep this raises pressure / fan backlash risk.
    InterestFromRival,
    /// Rumour or approach links the player to a club in their home
    /// country. Emotionally charged regardless of pure rep gap.
    HomecomingRumour,
    /// Approach from a club the player previously played for. Pulls on
    /// loyalty / unfinished-business strings.
    FormerClubInterest,
    /// Approach from a club listed as the player's favourite. Strong
    /// emotional pull; often produces excitement even before a bid.
    FavoriteClubInterest,
    /// Repeated speculation that the player has not yet shaken off —
    /// distracts focus, drags pressure load.
    TransferSpeculationDistracts,
    /// Player publicly dismisses the speculation and reaffirms focus
    /// on the current club. Small positive PR + dressing-room effect.
    TransferInterestDismissed,
    /// Talks with the interested club are imminent / opening — the
    /// player is now in the final stages of a possible move.
    TransferTalksExpected,
    /// Previously concrete interest has cooled — the buying club has
    /// moved on without a bid. Mild disappointment for the player.
    InterestCooled,
    /// Player used external interest as leverage during contract
    /// renewal — produces a small confidence effect plus a follow-up
    /// risk flag.
    UsedInterestForContractLeverage,
    /// Supporter reaction to an active transfer rumour — split between
    /// "stay" and "go" voices, tracked as fan pressure.
    FansReactToTransferRumour,
    /// Praised by the supporters — banners, songs, fan-poll wins.
    FanPraise,
    /// Targeted by fan criticism — bad displays, off-field controversy.
    FanCriticism,
    /// Praised in the media. Reputation-boosting profile pieces, top
    /// pundit ratings.
    MediaPraise,
    /// Targeted by media criticism. Hatchet jobs, tabloid drama.
    MediaCriticism,

    // ── Social / culture ────────────────────────────────────────
    /// A close friend / mentor / linchpin teammate left the club. Players
    /// with strong relationships at the dressing-room core feel this.
    CloseFriendSold,
    /// A compatriot (same primary nationality) joined the club. Big
    /// integration boost for foreign players battling language/culture.
    CompatriotJoined,
    /// Veteran mentor on whom this young player relied departed. Hits
    /// developing players who lost an established guidance figure.
    MentorDeparted,
    /// Made meaningful progress with the local language. Self-reinforcing
    /// integration milestone, only fires for foreign players.
    LanguageProgress,

    // ── Awards / nominations ────────────────────────────────────
    PlayerOfTheMonth,
    YoungPlayerOfTheMonth,
    /// Named the league's Young Player of the Week (age ≤ 20). Career
    /// memory for emerging talent — distinct from the broader Player
    /// of the Week so a 19-year-old who edged the senior award doesn't
    /// suppress the under-20 recognition.
    YoungPlayerOfTheWeek,
    TeamOfTheWeekSelection,
    /// Selected in the Young Team of the Week (age ≤ 20). Recognition
    /// for an under-20 starting in the weekly young XI.
    YoungTeamOfTheWeekSelection,
    /// Selected in the league's monthly XI. Career-visible recognition
    /// covering a full month of fixtures — distinct from
    /// `TeamOfTheWeekSelection` (single matchweek) and
    /// `TeamOfTheSeasonSelection` (whole campaign).
    TeamOfTheMonthSelection,
    /// Selected in the Young Team of the Month (age ≤ 21).
    YoungTeamOfTheMonthSelection,
    TeamOfTheSeasonSelection,
    /// Selected in the league's calendar-year XI (Team of the Year).
    /// Distinct from `TeamOfTheSeasonSelection`, which is per-season.
    TeamOfTheYearSelection,
    PlayerOfTheSeason,
    YoungPlayerOfTheSeason,
    LeagueTopScorer,
    LeagueTopAssists,
    LeagueGoldenGlove,
    ContinentalPlayerOfYearNomination,
    ContinentalPlayerOfYear,
    WorldPlayerOfYearNomination,
    WorldPlayerOfYear,

    // ── Real-life football events ────────────────────────────────
    /// First competitive senior appearance for the current club.
    SeniorDebut,
    /// First international appearance after being capped (transitions
    /// from 0 to >0 international apps).
    NationalTeamDebut,
    /// Three or more goals in a non-friendly match.
    HatTrick,
    /// Three or more assists in a non-friendly match.
    AssistHatTrick,
    /// Returned to scoring after a long competitive drought.
    GoalDroughtEnded,
    /// Forward facing a sustained scoring drought.
    ScoringDroughtConcern,
    /// Reached a competitive appearances milestone.
    AppearanceMilestone,
    /// Reached a competitive goals milestone.
    GoalMilestone,
    /// Reached a competitive clean sheets milestone (GK only).
    CleanSheetMilestone,
    /// High-controversy / low-temperament training-ground confrontation.
    TrainingGroundBustUp,
    /// Public apology following a controversy / bust-up.
    PublicApology,
    /// Supporters chanted the player's name in a strong performance.
    FansChantPlayerName,
    /// Sustained negative media coverage at high-profile reputation.
    MediaPressureMounting,
    /// Veteran captain / senior pro stepping up as dressing-room leader.
    LeadershipEmergence,

    // ── Career-desire moods ──────────────────────────────────────
    /// Foreign player who has failed to settle and is openly hoping for
    /// a move back toward his home country / former club / favourite
    /// club. Negative ongoing mood, fed by the chronic-adaptation helper.
    WantsReturnHome,
    /// Ambitious player whose current club cannot offer European
    /// competition — Champions League / Europa / Conference. Negative
    /// ambition mood while the gap exists; cleared when the team
    /// qualifies or the player moves on.
    WantsEuropeanCompetition,
    /// South-American player whose current setup cannot offer Copa
    /// Libertadores football. Same shape as `WantsEuropeanCompetition`
    /// but routed via the South American continent / heritage path.
    WantsCopaLibertadores,
    /// Positive counterpart to `WantsReturnHome` — concrete approach
    /// from a home-country / former / favourite-club destination has
    /// surfaced and the player feels relief.
    HomeReturnOpportunity,
    /// Positive counterpart to `WantsEuropeanCompetition` /
    /// `WantsCopaLibertadores` — the team has secured the desired
    /// continental path the player was missing.
    ContinentalAmbitionSatisfied,
    /// Catch-all for the broader life-simulation mood / request
    /// categories — family events, role / pressure preferences,
    /// language tutor asks, NT visibility, loyalty refusals. The
    /// `LifeSimulationDesireContext` on the event identifies which
    /// flavour was emitted; renderer picks copy off `kind`.
    LifeSimulationDesire,

    // ── Transfer-environment realism (weak↔elite, star↔weak) ────
    //
    // Emitted at the first sim tick after a transfer (one-shot via
    // `pending_signing`) and/or weekly during the integration window
    // (`process_transfer_environment_story`). Driven by the
    // `TransferEnvironmentProfile` built from the source/dest club &
    // league reputation, the player's own world rep / CA, destination
    // position depth, ambition, and language fit. See
    // `personality::adaptation` for the gates and `MoraleEventCatalog`
    // for the base magnitudes.
    //
    // Positive / aspirational
    /// Weak or low-reputation player suddenly thrust onto a top-club
    /// stage. Pride lift, even when the player isn't expected to start.
    TopClubOpportunity,
    /// Player benefits from the higher training standards / coaching
    /// at the new club. Quiet ongoing boost.
    EliteTrainingLift,
    /// Player visibly settles after a rocky start at the new club.
    AdaptationBreakthrough,
    /// Manager has started trusting the player after their step-up.
    TrustedAfterStepUp,
    /// Player's performances prove he belongs at the bigger stage.
    ProvedLevelAfterMove,
    /// An established teammate has taken the new signing under his wing.
    SeniorMentorSupport,
    //
    // Negative / pressure
    /// Weak player struggles with the standards / pace at a top club.
    OverawedByEliteClub,
    /// Destination's depth chart blocks the new arrival's minutes path.
    RolePathBlockedAtEliteClub,
    /// Sudden national-media spotlight after a big-club move.
    MediaSpotlightPressure,
    /// Player is no longer a dressing-room star at the new club.
    DressingRoomStatusShock,
    /// High-reputation player feels he's playing below his level after
    /// a step-down move.
    TooGoodForLevel,
    /// Star frustrated by sub-standard coaching / facilities at the
    /// new club.
    TrainingStandardFrustration,
    /// High fee or high incoming reputation creates fan-pressure burden.
    FanExpectationBurden,
    /// Reputational embarrassment after a clear step-down move.
    StepDownEmbarrassment,
    /// Loan level is either too easy or too hard for the player's tier.
    LoanLevelMismatch,

    // ── Career-stage / late-career arc ───────────────────────────
    /// Older player has started to weigh up retirement — reduced role,
    /// recurring injuries, long free-agency, or physical decline. A
    /// mostly-informational lead-up to [`RetirementAnnounced`]; it does
    /// not itself retire the player. Carries a `CareerStageEventContext`.
    RetirementConsidering,
    /// Player has formally announced retirement. Career-visible event,
    /// not a morale complaint — emitted on every retirement path before
    /// the player is moved to retired storage. Magnitude is positive for
    /// a planned / legend farewell, negative for forced (long
    /// unemployment) or injury-forced early retirement.
    RetirementAnnounced,
    /// Veteran leader has signalled interest in coaching after hanging up
    /// his boots — a bridge from playing career to future staff supply.
    /// Positive / neutral career event; never advances retirement itself.
    CoachingCareerInterest,

    // ── Career-desire / squad-ambition pressure ──────────────────
    /// Ambitious star wants the board to strengthen the squad before he
    /// commits his future — sold key players unreplaced, a unit far below
    /// his level, or board ambition concern. Pressure signal, not a
    /// transfer request. Carries a `CareerDesireEventContext`.
    WantsStrongerSquad,
    /// Elite, ambitious player wants to play for a genuine title
    /// challenger — more specific than `WantsEuropeanCompetition`. Driven
    /// by league-table context (position, points off the leader). Rare,
    /// mostly affects stars at mid-table clubs.
    WantsTitleChallenge,
    /// Senior player stuck in a reserve / B / second squad dreams of
    /// genuine first-team football — his own club's first team or a
    /// club where he'd actually start. Distinct from
    /// `LackOfPlayingTime`: he may be playing every reserve fixture;
    /// the grievance is the *level*, not the minutes. Chronic mood
    /// that escalates into loan / transfer requests via the weekly
    /// playing-time complaint pass.
    WantsFirstTeamFootball,
    /// The club has just been relegated and this player is clearly
    /// above the new division — he wants to keep playing at the level
    /// he just lost. The classic post-relegation exodus: quality
    /// players leave, often at a discount. Chronic mood that escalates
    /// into a formal transfer request when it lingers. Carries a
    /// `CareerDesireEventContext`.
    WantsToLeaveAfterRelegation,

    // ── Loan management pressure ─────────────────────────────────
    /// A young player's loan is failing to develop him — benched, wrong
    /// role, wrong level, weak training, or stalled progress. Aggregates
    /// several weak signals into one monthly development warning. Distinct
    /// from `LackOfPlayingTime` (player morale) and `LoanRecallRequested`
    /// (action pressure).
    LoanDevelopmentConcern,
    /// Parent club / player is pushing to recall a loaned player because
    /// the loan is failing on minutes / role / level. More serious than
    /// the minutes-concern note, less severe than a permanent transfer
    /// request — the request / pressure layer above the recall window.
    LoanRecallRequested,
    /// A loanee who is thriving at the borrowing club — starting
    /// regularly, performing — wants the move made permanent instead of
    /// returning to the parent's bench or the loan carousel. Longing,
    /// not a grievance; carries a `CareerDesireEventContext`.
    WantsLoanMadePermanent,
    /// The player has come back from a loan where he was first-choice
    /// and walked straight into a fringe role at the parent club — the
    /// return he didn't choose. Emitted at loan-return time, after the
    /// clean-slate reset, so the unsettled mood carries into the new
    /// parent spell and feeds the stuck-career machinery.
    UnsettledAfterLoanReturn,

    // ── Contract negotiation tension ─────────────────────────────
    /// Player / agent explicitly demands a release clause in the next
    /// contract — ambition, bigger-club interest, or mistrust after
    /// rejected bids. A mild tension event that models leverage / exit
    /// planning, not automatic conflict.
    ReleaseClauseDemanded,
    /// Contract negotiations have visibly stalled — club and player are
    /// far apart on wage, release clause, role, length, or ambition. A
    /// signal (raises unhappiness, makes listing likelier) rather than a
    /// transfer request. Sits between `ContractOffer` and
    /// `TransferBidRejected` in severity.
    ContractTalksStalled,
    /// Player offer was formally rejected by the player / agent. Carries
    /// a `ContractEventContext` whose `kind` is `RejectedLowStatusOffer`
    /// or related, plus evidence for the reason (wage / length / role /
    /// release clause / ambition). Bigger than `ContractTalksStalled`
    /// because the decision has been made — the deal is dead, not paused.
    RejectedContractOffer,
    /// Player has entered the final year of his deal and the club has
    /// not opened renewal talks — no offer, no stalled negotiation,
    /// just silence. Anxiety for the loyal, leverage for the ambitious.
    /// Distinct from `ContractTalksStalled` (talks happened and broke
    /// down). Monthly, cooldowned.
    ContractExpiryAnxiety,
    /// Positive twin of `ContractExpiryAnxiety` — a final-year player
    /// in real form treats the run-in as a shop window: extra
    /// motivation while his future is unsigned.
    PlayingForNewContract,
    /// The transfer window closed with the player still on the transfer
    /// list, unsold — stuck at a club that wants him gone until the
    /// next window. Visible beat between listing and the availability
    /// broadcast / free-exit machinery.
    UnsoldWindowClosed,
    /// The club rejected a concrete bid for a player who had formally
    /// asked to leave — the move he wanted was vetoed. Much sharper
    /// than the neutral `TransferBidRejected`, which also covers bids
    /// the player never cared about.
    MoveVetoedByClub,

    // ── Manager-relationship arc & match-trust ───────────────────
    /// Player formally asked the manager for a private conversation
    /// about a serious concern — role, minutes, contract, transfer
    /// status, captaincy / status, or a tactical role he can no longer
    /// stomach. Rare and cooldowned; never emitted for every weekly
    /// unhappy player. Carries a `ManagerInteractionEventContext` whose
    /// topic + tone hint the underlying driver.
    AskedForPrivateTalk,
    /// Transfer-listed player, still unsold months after being listed,
    /// formally asked the club to find him a new team — the trigger for
    /// the scouts' availability broadcast that shops him around other
    /// clubs. Mild frustration note, cooldowned; the exit itself is
    /// handled by the sale / free-exit machinery.
    AskedClubToArrangeTransfer,
    /// Ambitious or senior player worried about where the club is
    /// heading: key sales unreplaced, weakened squad, persistent
    /// underperformance, board direction. Distinct from
    /// `WantsStrongerSquad` (squad-strength pressure) — this is a
    /// broader club-direction concern. Cooldowned.
    ConcernedByClubDirection,
    /// Counterpart of `ConcernedByClubDirection` — board invested in a
    /// meaningful signing or visibly upgraded squad quality. Bigger lift
    /// for ambitious players and those who had previously asked for a
    /// stronger squad.
    EncouragedBySquadInvestment,
    /// Player repeatedly deployed in an unsuitable position / role or
    /// inside a team system with no natural fit for him. Distinct from
    /// `RoleMismatch` (a long-running formation/system mismatch) — this
    /// is tactical / matchday-usage frustration.
    UnhappyWithTacticalRole,
    /// Manager picked the player for a high-pressure fixture (derby,
    /// cup final, title-race decider, promotion / relegation play-off,
    /// continental knockout). Bigger lift for young / fringe / form-
    /// recovering players. Cooldowned to keep stars from spamming this
    /// every big night.
    TrustedInBigMatch,
    /// Established starter / key player dropped for a major fixture
    /// when he was expected to start. Not emitted for injury,
    /// suspension, rest, or low-importance rotation.
    BenchedForBigMatch,
    /// Repeated early substitutions, removal while playing well, or
    /// pulled off in a big match before expected. Suppressed for
    /// injury, fatigue, tactical red-card response, or normal late
    /// substitutions. Cooldowned.
    SubstitutionFrustration,
    /// Reinjury, recovery delay, failed fitness test, or recurring
    /// injury concern. Distinct from `InjuryReturn` (positive). Carries
    /// an `InjuryRecoveryEventContext` whose `stage` is
    /// `RecoverySetback` or `InjuryRecurrenceConcern`.
    InjurySetback,
    /// New signing directly competes for the player's position / status
    /// / minutes. Stronger for older / fringe players and those already
    /// lacking minutes. Suppressed when the signing plays an unrelated
    /// position.
    ThreatenedByNewSigning,
    /// Repeated selections, public backing, improved role, or big-match
    /// trust signal the manager relationship is improving. Low-frequency
    /// positive aggregate of small good signs.
    ManagerTrustGrowing,
    /// Repeated benching, criticism, broken promises, tactical mismatch,
    /// or loss of role signal the manager relationship is worsening. An
    /// aggregate so repeated `MatchDropped` rows don't each fire their
    /// own. Cooldowned.
    ManagerTrustEroding,
    /// A new head coach has just taken charge — the squad-wide bounce of
    /// fresh expectation. Players the old regime had frozen out (low
    /// morale, unhappy, club-listed) hope hardest: the new coach's
    /// selection memory starts empty, so the clean slate is real.
    /// Counterpart of `ManagerDeparture`, which carries the loyalists'
    /// grief for the outgoing coach.
    NewManagerBounce,

    // ── Manager pressure, club discipline & collective reaction ──
    /// The board has publicly put the manager on notice and this player
    /// is with him — extra motivation to save the coach's job. The
    /// loyalist fork of the pressure story.
    RalliesBehindManager,
    /// The board has the manager on notice and this player senses a
    /// change coming — unsettling if he depends on the current coach's
    /// trust, quietly liberating if he doesn't. Doubter fork; also used
    /// by the renewal machinery as a "wait and see" signal.
    SensesManagerChange,
    /// Formal club warning for misconduct — the first rung of the
    /// disciplinary ladder. Stings less for professionals who take it
    /// on the chin.
    FormalWarningIssued,
    /// Club fine (a chunk of wages) for repeated or serious misconduct —
    /// the second rung. Professionals accept and straighten out;
    /// hot-headed players resent it.
    FinedByClub,
    /// The manager held an inquest after a humiliating result — a
    /// heavy defeat or a collapse against far weaker opposition. Squad-
    /// wide sting; the fair response to a bad day, not a persecution.
    DressingRoomInquest,
    /// The manager publicly praised the squad's response after bouncing
    /// back from a bad run — the collective positive mirror of the
    /// inquest.
    ResiliencePraised,
    /// The club formally told the player he is not in the manager's
    /// plans and can find a new club — emitted when a club-decision
    /// transfer listing first appears for a player who had not asked
    /// out. The honest conversation that used to happen silently.
    ToldNotInPlans,
    /// The coaching staff set a personal development plan — position
    /// retraining toward a thin spot, a fitness block for the injury-
    /// prone, or focused skill work. Positive for players who feel the
    /// club investing in them; emitted negative when a role change is
    /// resented.
    PersonalTrainingPlanSet,
}
