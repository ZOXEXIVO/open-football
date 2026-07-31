use crate::transfers::pipeline::ScoutingRecommendation;
use crate::transfers::{CompletedTransfer, TransferType};
use crate::{
    HappinessEvent, HappinessEventType, LifeSimulationDesireKind, LoanSpellRecord, Player,
    PlayerSquadStatus, RegulationSlotKind, RetirementReason, SelectionOmissionReason,
    TeammateConflictReason,
};
use rustc_hash::{FxHashMap, FxHashSet};

/// One knockout tie a club played this week. Domestic cup results never
/// reach a league table, so the match desk has no other way of telling
/// "lost 0-1 away" from "knocked out of the cup".
#[derive(Debug, Clone, Copy)]
pub struct CupTie {
    pub opponent_team_id: u32,
    pub goals_for: u8,
    pub goals_against: u8,
    /// Shoot-out tally when the tie went that far, home side first.
    pub shootout: (u8, u8),
}

impl CupTie {
    /// Who goes through. A drawn 90 minutes is settled on the shoot-out
    /// tally, which is exactly how a supporter reads the result.
    pub fn advanced(&self) -> bool {
        if self.goals_for != self.goals_against {
            return self.goals_for > self.goals_against;
        }
        self.shootout.0 > self.shootout.1
    }
}

/// A week's worth of one player's football, folded down to the single
/// loudest afternoon in it. Implemented by both per-player fact types so
/// the "keep the louder game" rule lives in one place per type and the
/// readers can share a fold.
pub trait Absorbing: Copy {
    fn absorb(&mut self, other: Self);
}

/// A goalkeeper's afternoon, as the match engine recorded it.
///
/// The one position whose week is invisible in the numbers every other
/// desk reads: a keeper scores nothing, assists nothing, and the team's
/// scoreline says as much about the ten in front of him as about him.
/// What actually happened to him is in the per-match stat line, so the
/// press run reads it directly.
///
/// Each field is the SINGLE most newsworthy match of the week rather
/// than a total — a story is about one afternoon, and "he made nine
/// saves" is a piece where "he made nine saves across two games" is
/// not.
///
/// Held per SIDE rather than per player — see [`WeeklyMatchFacts`].
#[derive(Debug, Clone, Copy, Default)]
pub struct KeeperMatchFacts {
    pub saves: u16,
    /// Goals past him. The engine defines `shots_faced` as saves plus
    /// goals conceded, so this is what is left when the saves come out.
    pub conceded: u16,
    /// Spot-kicks he kept out in a shoot-out.
    pub penalties_saved: u8,
    /// Mistakes of his own that ended in the net.
    pub errors_leading_to_goal: u16,
}

impl Absorbing for KeeperMatchFacts {
    /// Keep the loudest version of each figure across a week in which he
    /// played more than once.
    fn absorb(&mut self, other: KeeperMatchFacts) {
        self.saves = self.saves.max(other.saves);
        self.conceded = self.conceded.max(other.conceded);
        self.penalties_saved = self.penalties_saved.max(other.penalties_saved);
        self.errors_leading_to_goal = self
            .errors_leading_to_goal
            .max(other.errors_leading_to_goal);
    }
}

/// One outfield player's afternoon, as the match engine recorded it.
///
/// The gap this closes is the same one [`KeeperMatchFacts`] closed for
/// goalkeepers, and it was the larger of the two: for the other ten
/// players the desks could see a goal, a red card and a SEASON average,
/// and nothing else. So a striker who put four clear chances into the
/// stands, a centre-half whose error gifted the winner, a midfielder who
/// made three for other people and a full-back marked 5.1 in a hiding
/// all left exactly the same trace on the page — none.
///
/// Everything here is in the per-match stat line, which survives into
/// stored results (`copy_without_data_positions` strips only the
/// position replay), so the press run reads it directly.
///
/// Each field keeps the SINGLE most newsworthy match of the week rather
/// than a total, because a verdict is about one afternoon: "he was
/// marked 8.4" is a piece and "he averaged 7.1 across two games" is a
/// table. The two ratings are kept apart for the same reason — a player
/// can be magnificent on Wednesday and dreadful on Saturday, and both
/// are real stories that a single averaged figure would erase.
///
/// Held per SIDE rather than per player — see [`WeeklyMatchFacts`].
#[derive(Debug, Clone, Copy, Default)]
pub struct OutfieldMatchFacts {
    /// His best mark of the week, ×100.
    pub best_rating: i32,
    /// His worst mark of the week, ×100. Non-zero only once he has
    /// actually been marked.
    pub worst_rating: i32,
    /// Minutes in the match his worst mark came from, and whether he
    /// started it — a substitute cannot be "hooked", and a man given
    /// twenty minutes is not to be marked down for them.
    pub worst_rating_minutes: u16,
    pub worst_rating_started: bool,
    /// He started that match and was taken off by CHOICE. Load-bearing
    /// for the hooked-early piece: a man carried off injured on the half
    /// hour has also "been taken off before the hour having been marked
    /// down", and reporting it as a manager's verdict would invent a
    /// decision nobody made. The engine tags every substitution with its
    /// reason, so the distinction is read rather than guessed.
    pub worst_rating_hooked: bool,
    pub goals: u16,
    pub assists: u16,
    pub key_passes: u16,
    pub successful_dribbles: u16,
    /// Tackles, interceptions, blocks and clearances in one afternoon —
    /// a defender's whole contribution, and the one that never reaches
    /// a scoreline.
    pub defensive_actions: u16,
    /// Shots and the expected goals behind them, from the same match.
    /// Kept as a pair: shots alone cannot separate a man who had six
    /// efforts from distance from one who missed three open goals.
    pub shots: u16,
    pub xg_x100: i32,
    /// True when the side he was playing for kept a clean sheet in the
    /// afternoon his defensive shift came from. Read from the result
    /// rather than the stat line — a defender's stat line has no idea
    /// what happened at the other end.
    pub shut_out: bool,
    pub own_goals: u16,
    pub errors_leading_to_goal: u16,
    pub fouls: u16,
    pub yellow_cards: u16,
    /// Spot-kicks he took in a shoot-out and missed.
    pub penalties_missed: u8,
    /// He was the match's outstanding player, by the engine's own
    /// verdict rather than the desk's arithmetic.
    pub man_of_the_match: bool,
    /// Goals and assists he produced after coming on as a substitute.
    pub impact_off_the_bench: u16,
}

impl Absorbing for OutfieldMatchFacts {
    /// Keep the loudest version of each figure across a week in which he
    /// played more than once.
    ///
    /// The ratings are the exception to the plain maximum: `worst_rating`
    /// travels with the minutes and the starting flag from ITS OWN
    /// match, because "taken off at 55 having been marked 5.2" is only
    /// true of one afternoon and reading the minutes off the other one
    /// would invent a substitution that never happened.
    fn absorb(&mut self, other: OutfieldMatchFacts) {
        self.best_rating = self.best_rating.max(other.best_rating);

        if other.worst_rating > 0
            && (self.worst_rating == 0 || other.worst_rating < self.worst_rating)
        {
            self.worst_rating = other.worst_rating;
            self.worst_rating_minutes = other.worst_rating_minutes;
            self.worst_rating_started = other.worst_rating_started;
            self.worst_rating_hooked = other.worst_rating_hooked;
        }

        self.goals = self.goals.max(other.goals);
        self.assists = self.assists.max(other.assists);
        self.key_passes = self.key_passes.max(other.key_passes);
        self.successful_dribbles = self.successful_dribbles.max(other.successful_dribbles);
        self.own_goals = self.own_goals.max(other.own_goals);
        self.errors_leading_to_goal = self
            .errors_leading_to_goal
            .max(other.errors_leading_to_goal);
        self.fouls = self.fouls.max(other.fouls);
        self.yellow_cards = self.yellow_cards.max(other.yellow_cards);
        self.penalties_missed = self.penalties_missed.max(other.penalties_missed);
        self.man_of_the_match |= other.man_of_the_match;
        self.impact_off_the_bench = self.impact_off_the_bench.max(other.impact_off_the_bench);

        // The defensive shift keeps the clean sheet it was actually
        // performed in front of: forty clearances in a 4-0 defeat is not
        // the rearguard piece, and pairing the bigger shift with the
        // other afternoon's clean sheet would print one.
        if other.defensive_actions > self.defensive_actions {
            self.defensive_actions = other.defensive_actions;
            self.shut_out = other.shut_out;
        }

        // Same pairing rule for the wasteful-finishing piece: the shots
        // and the expected goals have to come from one game or the
        // sentence is arithmetic across two.
        if other.xg_x100 > self.xg_x100 {
            self.xg_x100 = other.xg_x100;
            self.shots = other.shots;
        }
    }
}

impl OutfieldMatchFacts {
    /// True when nothing here is worth a line. Most players in most
    /// weeks — which is the point of a ratings column having a bar.
    pub fn is_routine(&self) -> bool {
        self.best_rating == 0 && self.worst_rating == 0
    }
}

/// The man a match report gets to name: the side's top scorer that
/// afternoon, read off one match's goal details.
///
/// A report that only ever says "{club} beat {opponent} {score}" reads
/// as a generated page however the phrasing rotates — the thing a real
/// report always carries is WHO did it. The desk attaches him to the
/// report, and the composer only reaches for player-flavoured copy when
/// he is actually there, so a 0-2 defeat never names a hero it did not
/// have.
///
/// `team_goals` pins the star to the result he starred in: a side can
/// play twice in one week, and "his afternoon" printed under the other
/// afternoon's scoreline is worse than no name at all.
#[derive(Debug, Clone, Copy)]
pub struct MatchStarFacts {
    pub player_id: u32,
    /// His goals in that match.
    pub goals: u8,
    /// The team's full tally in the same match.
    pub team_goals: u8,
}

/// The shape of one afternoon, read off the minute each goal was
/// scored in it.
///
/// A scoreline is the least interesting true thing about a football
/// match. 2-1 covers a side that led from the eighth minute and saw it
/// out, a side that equalised twice and won it in stoppage time, and a
/// side that was two down at the break — and a supporter would tell you
/// about the second and third for the rest of his life. The engine has
/// always stamped every goal with the minute; nothing had ever read
/// them.
///
/// Held per SIDE, keyed `(team, opponent)` like [`MatchStarFacts`], and
/// pinned by `team_goals` for the same reason: a club can play the same
/// opponent twice in a week, and one afternoon's drama printed under
/// the other's scoreline is worse than printing no drama at all.
#[derive(Debug, Clone, Copy, Default)]
pub struct MatchDramaFacts {
    /// The side's own tally in the match these were read from.
    pub team_goals: u8,
    /// Both sides' goals in it.
    pub total_goals: u8,
    /// The minute it took the lead it never gave back, when it won.
    /// Zero when it did not.
    pub winner_minute: u16,
    /// The most it ever trailed by.
    pub max_deficit: u8,
    /// The most it was ever ahead by.
    pub max_lead: u8,
    /// How many of its goals came inside the opening twenty minutes.
    pub early_goals: u8,
    /// Minutes it took to draw level again after falling behind, when
    /// it managed it inside the reply window. Zero when it did not —
    /// and never zero when it did, because "level again after nought
    /// minutes" is not a sentence.
    pub reply_minutes: u8,
    /// One of its players was sent off.
    pub red_card: bool,
    /// It won.
    pub won: bool,
}

impl MatchDramaFacts {
    /// Goals inside this window count as an early blitz.
    pub const EARLY_WINDOW_MS: u64 = 20 * 60 * 1000;
    /// A reply inside this long is a reply the ground felt.
    pub const REPLY_WINDOW_MS: u64 = 5 * 60 * 1000;
    /// From this minute on, a winning goal is a late winner.
    pub const LATE_MINUTE: u16 = 85;
    /// …and from this one it is stoppage time, whatever the fourth
    /// official held up.
    pub const STOPPAGE_MINUTE: u16 = 90;

    /// Rank of how much of a story one afternoon was, used only to keep
    /// the louder of two meetings when a club plays the same opponent
    /// twice inside a week.
    pub fn loudness(&self) -> u32 {
        let comeback = if self.won { self.max_deficit as u32 } else { 0 };
        comeback * 100 + self.total_goals as u32 * 10 + self.max_lead as u32
    }
}

/// Everything the desks need to know about the week just played that
/// they cannot read off a `Club`. Built once per Monday from the world's
/// competitions and shared by every club in it.
///
/// Every per-player entry is keyed by `(player, side he played for)`
/// rather than by the player alone, and that pairing is load-bearing.
/// The map is built from every competitive fixture in the world — which
/// includes internationals, where the "side" is a country — and it is
/// then read by a club paper walking its own roster. Keyed by player
/// alone, "is he on my books today" was the only question asked, so a
/// keeper's afternoon for his country, or for the club that sold him on
/// Friday, came out under the buying club's nameplate: "{player} keeps
/// {club} in it on his own" about ninety minutes {club} did not play.
/// With the side recorded, a paper can ask the question that actually
/// decides it — was this OUR afternoon — and the answer for an
/// international or a previous employer is no.
pub struct WeeklyMatchFacts {
    /// Goals scored in a single match this week, for players who
    /// managed at least three in one game.
    pub hat_tricks: FxHashMap<(u32, u32), u8>,
    /// Players sent off this week, and the side they were sent off for.
    pub red_cards: FxHashSet<(u32, u32)>,
    /// Knockout ties, keyed by the team that played them.
    pub cup_ties: FxHashMap<u32, CupTie>,
    /// What the week did to each goalkeeper who played in it.
    pub keepers: FxHashMap<(u32, u32), KeeperMatchFacts>,
    /// …and the same for the other ten, which is where nearly all of a
    /// football paper's individual copy comes from.
    pub outfield: FxHashMap<(u32, u32), OutfieldMatchFacts>,
    /// Each side's top scorer per match, keyed by (team, opponent) —
    /// the pair a match report already carries, so the desk can hand
    /// the report its protagonist without a second lookup anywhere.
    pub stars: FxHashMap<(u32, u32), MatchStarFacts>,
    /// …and how each side's afternoon actually went, on the same key.
    pub drama: FxHashMap<(u32, u32), MatchDramaFacts>,
    /// Continental nights, keyed by the team that played them.
    ///
    /// The per-player facts from these fixtures have always reached
    /// club papers — continental results are pushed to the same global
    /// store this gather already walks — so a European hat-trick was
    /// reported correctly and the match it happened in was filed as an
    /// ordinary league game. This is the label that was missing.
    ///
    /// Deliberately NOT folded into `cup_ties`: a group-stage fixture
    /// is not a knockout tie, and `CupTie::advanced()` would report
    /// every drawn group game as an elimination.
    pub continental: FxHashMap<u32, ContinentalNight>,
}

/// One continental fixture, from one side's point of view.
#[derive(Debug, Clone, Copy)]
pub struct ContinentalNight {
    pub opponent_team_id: u32,
    pub goals_for: u8,
    pub goals_against: u8,
}

impl WeeklyMatchFacts {
    pub fn empty() -> Self {
        WeeklyMatchFacts {
            hat_tricks: FxHashMap::default(),
            red_cards: FxHashSet::default(),
            cup_ties: FxHashMap::default(),
            keepers: FxHashMap::default(),
            outfield: FxHashMap::default(),
            stars: FxHashMap::default(),
            drama: FxHashMap::default(),
            continental: FxHashMap::default(),
        }
    }

    /// A goalkeeper's week as one of `sides` played it, or `None` when
    /// none of them fielded him. `sides` is every squad the club owns —
    /// wide enough that a youth keeper's cup start for the first team is
    /// still his club's news, narrow enough that an international or a
    /// former employer's afternoon is not.
    ///
    /// Folded rather than picked, for the same reason [`KeeperMatchFacts::absorb`]
    /// folds: a man who played twice for the club in one week has one
    /// week, and the loudest half of it is the story.
    pub fn keeper_of(&self, player_id: u32, sides: &FxHashSet<u32>) -> Option<KeeperMatchFacts> {
        Self::folded(sides, |team_id| {
            self.keepers.get(&(player_id, team_id)).copied()
        })
    }

    /// The outfield twin of [`Self::keeper_of`].
    pub fn outfield_of(
        &self,
        player_id: u32,
        sides: &FxHashSet<u32>,
    ) -> Option<OutfieldMatchFacts> {
        Self::folded(sides, |team_id| {
            self.outfield.get(&(player_id, team_id)).copied()
        })
    }

    /// Goals in one game for one of `sides`, when he managed three or
    /// more. A hat-trick for his country is his country's story.
    pub fn hat_trick_of(&self, player_id: u32, sides: &FxHashSet<u32>) -> Option<u8> {
        sides
            .iter()
            .filter_map(|team_id| self.hat_tricks.get(&(player_id, *team_id)).copied())
            .max()
    }

    /// Whether he was sent off playing for one of `sides`.
    pub fn was_sent_off(&self, player_id: u32, sides: &FxHashSet<u32>) -> bool {
        sides
            .iter()
            .any(|team_id| self.red_cards.contains(&(player_id, *team_id)))
    }

    /// Fold whatever `lookup` finds across the club's sides into one
    /// week, using each fact type's own "loudest afternoon" rule.
    fn folded<T: Absorbing>(
        sides: &FxHashSet<u32>,
        lookup: impl Fn(u32) -> Option<T>,
    ) -> Option<T> {
        let mut week: Option<T> = None;
        for team_id in sides {
            let Some(found) = lookup(*team_id) else {
                continue;
            };
            match week.as_mut() {
                Some(week) => week.absorb(found),
                None => week = Some(found),
            }
        }
        week
    }

    /// The star to print under one specific result, if the week
    /// recorded one for exactly that afternoon. The tally check is what
    /// keeps a side's two matches in one week from crediting the wrong
    /// man: the entry survives only for the result it was read from.
    pub fn star_of(&self, team_id: u32, opponent_team_id: u32, goals_for: u8) -> u32 {
        self.stars
            .get(&(team_id, opponent_team_id))
            .filter(|star| star.team_goals == goals_for)
            .map(|star| star.player_id)
            .unwrap_or(0)
    }

    /// How one specific afternoon went, on the same pin as [`Self::star_of`]
    /// and for the same reason.
    /// The continental night one of  played this week, if any.
    /// Read by the match desk to label the result and to reach for the
    /// European copy rather than the league report.
    pub fn continental_of(&self, sides: &FxHashSet<u32>) -> Option<ContinentalNight> {
        sides.iter().find_map(|team_id| self.continental.get(team_id).copied())
    }

    pub fn drama_of(
        &self,
        team_id: u32,
        opponent_team_id: u32,
        goals_for: u8,
    ) -> Option<MatchDramaFacts> {
        self.drama
            .get(&(team_id, opponent_team_id))
            .filter(|drama| drama.team_goals == goals_for)
            .copied()
    }
}

/// What kind of move a completed transfer actually was. The distinction
/// the old code missed is the one between "sold" and "left for nothing":
/// a departure with no fee is a release or an expiring contract, and a
/// paper that prints "sold for $0.00" is not a paper anybody believes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferMoveKind {
    /// Money changed hands.
    Paid,
    /// No fee: a free transfer, a released player, an expiring deal.
    Free,
    /// A temporary move.
    Loan,
}

impl TransferMoveKind {
    fn read(transfer: &CompletedTransfer, fee: i64) -> Self {
        if matches!(transfer.transfer_type, TransferType::Loan(_)) {
            return TransferMoveKind::Loan;
        }
        // A `Permanent` record with nothing on it is still a free move
        // as far as a reader is concerned, whatever the type says.
        if fee <= 0 || matches!(transfer.transfer_type, TransferType::Free) {
            return TransferMoveKind::Free;
        }
        TransferMoveKind::Paid
    }
}

/// Why a club actually made a signing, as the press can print it.
///
/// Every completed deal already records its motive as an i18n key, and
/// the market desk had never opened one: a depth signing, a succession
/// plan, a bargain and a marquee upgrade all reached the page as "the
/// club have signed a player", which is the one thing a reader could
/// already see from the squad list.
///
/// Grouped rather than mapped one-for-one, because the vocabulary was
/// written for a transfers table and a paper does not have seven
/// different words for cover. The seven `loan_*` keys in particular are
/// one story from the borrowing club's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransferMotive {
    /// A hole in the shape that had to be filled.
    FormationGap,
    /// Straightforwardly better than what was there.
    QualityUpgrade,
    /// Cover, padding, an injury, a body for the bench.
    #[default]
    DepthCover,
    /// Signed to replace somebody who is not finished yet.
    Succession,
    /// Signed for what he might become.
    Development,
    /// The scouting department's own recommendation.
    ScoutFind,
    /// Signed for the miles on him.
    Experience,
    /// Cheap, and the cheapness is the point.
    Bargain,
    /// Not signed at all — promoted out of the club's own academy.
    AcademyPromotion,
    /// A motive the vocabulary does not record, or a generic one
    /// ("a transfer happened") that says nothing worth printing.
    Unknown,
}

impl TransferMotive {
    /// Read the motive off a completed deal's reason key.
    ///
    /// An unrecognised key degrades to [`TransferMotive::Unknown`],
    /// which prints the ordinary signing report — the same page the
    /// desk produced before any of this existed. A typo in an emitter
    /// therefore costs a flavour, never a story; the
    /// `every_signing_reason_key_is_understood` test is what stops it
    /// costing a flavour quietly.
    pub fn from_key(key: &str) -> Self {
        match key {
            "signing_reason_formation_gap" => TransferMotive::FormationGap,
            "signing_reason_quality_upgrade"
            | "signing_reason_loan_opportunistic_upgrade"
            | "signing_reason_opportunistic_loan_upgrade" => TransferMotive::QualityUpgrade,
            "signing_reason_depth_cover"
            | "signing_reason_squad_padding"
            | "signing_reason_loan_to_fill_squad"
            | "signing_reason_injury_cover_loan"
            | "signing_reason_loan_midseason_reinforcement" => TransferMotive::DepthCover,
            "signing_reason_succession_planning" => TransferMotive::Succession,
            "signing_reason_development_signing"
            | "signing_reason_loan_development_approach"
            | "signing_reason_loan_foreign_prospect" => TransferMotive::Development,
            "signing_reason_staff_recommendation" => TransferMotive::ScoutFind,
            "signing_reason_experienced_head" => TransferMotive::Experience,
            "signing_reason_cheap_reinforcement" => TransferMotive::Bargain,
            "signing_reason_academy_graduation" => TransferMotive::AcademyPromotion,
            // "a loan happened", "a transfer happened", a broadcast, a
            // manual move: true, and not worth a headline.
            _ => TransferMotive::Unknown,
        }
    }

    /// True when somebody borrowed the player to give him football —
    /// which, read from the parent club's paper, is the story of a lad
    /// being sent out to play.
    pub fn is_development(self) -> bool {
        self == TransferMotive::Development
    }
}

/// One side of one completed deal, as the market desk needs it.
#[derive(Debug, Clone, Copy)]
pub struct TransferMove {
    pub player_id: u32,
    /// The club at the other end. `0` when the counterparty is the free
    /// agent pool rather than a club.
    pub other_club_id: u32,
    pub fee: i64,
    pub kind: TransferMoveKind,
    /// The player's age on publication day. `0` when the newsroom could
    /// not resolve him, which reads as "age unknown" — never as a
    /// teenager.
    pub age: u8,
    /// An arrival who has worn these colours before. What separates a
    /// homecoming from an ordinary signing of the same size.
    pub returning: bool,
    /// An arrival who was already here on loan — the option exercised
    /// rather than a stranger unveiled.
    pub was_loan_here: bool,
    /// Why the buying club did it.
    pub motive: TransferMotive,
    /// How sure the scout who filed the report was, 0..100. Zero when
    /// no report backed the move, which is most of them — the desk only
    /// prints a scout's verdict when there was a scout.
    pub scout_confidence_pct: u8,
    /// The scout's report said sign him without hesitating.
    pub scout_urged_it: bool,
    /// Taken from a club this one counts as a rival.
    pub rival: bool,
}

/// One club's slice of the week's completed transfer business.
#[derive(Default)]
pub struct ClubTransferWeek {
    pub arrivals: Vec<TransferMove>,
    pub departures: Vec<TransferMove>,
}

impl ClubTransferWeek {
    /// Fold a completed move into the buying club's arrivals or the
    /// selling club's departures. `club_id` says which side is being
    /// filled in.
    pub fn absorb(&mut self, club_id: u32, transfer: &CompletedTransfer) {
        let fee = transfer.fee.amount.max(0.0) as i64;
        let kind = TransferMoveKind::read(transfer, fee);

        // The reason is the single translation point between the
        // transfer ledger's stringly vocabulary and the press, so it is
        // read once here and never carried further: `TransferMove` is
        // `Copy`, and a paper's whole shelf would otherwise hold a
        // hundred editions' worth of duplicated key strings.
        let scout = transfer.reason.scout.as_ref();
        let common = TransferMove {
            player_id: transfer.player_id,
            other_club_id: 0,
            fee,
            kind,
            // Age and history start unresolved; the newsroom's
            // enrichment pass fills them in for arrivals, where they
            // decide the story.
            age: 0,
            returning: false,
            was_loan_here: false,
            motive: TransferMotive::from_key(&transfer.reason.key),
            scout_confidence_pct: scout.map(|s| s.confidence_pct()).unwrap_or(0),
            scout_urged_it: matches!(
                scout.map(|s| &s.recommendation),
                Some(ScoutingRecommendation::StrongBuy)
            ),
            rival: transfer.reason.rival,
        };

        if transfer.to_club_id == club_id {
            self.arrivals.push(TransferMove {
                other_club_id: transfer.from_club_id,
                ..common
            });
        } else if transfer.from_club_id == club_id {
            self.departures.push(TransferMove {
                other_club_id: transfer.to_club_id,
                ..common
            });
        }
    }

    pub fn is_empty(&self) -> bool {
        self.arrivals.is_empty() && self.departures.is_empty()
    }
}

/// Where the club stands in its league on publication day.
#[derive(Debug, Clone, Copy)]
pub struct StandingSnapshot {
    pub position: u8,
    pub teams: u8,
    pub points: u8,
    pub played: u8,
    pub total_rounds: u8,
}

impl StandingSnapshot {
    /// Fraction of the programme completed, 0..1. The press only starts
    /// talking about titles and relegation once there is enough season
    /// behind the table for it to mean something.
    pub fn progress(&self) -> f32 {
        if self.total_rounds == 0 {
            return 0.0;
        }
        (self.played as f32 / self.total_rounds as f32).clamp(0.0, 1.0)
    }
}

/// How one of the club's loaned-out players is getting on, gathered
/// from the borrowing club's roster. The parent club's own squad list
/// no longer holds him, so without this pass the loan column would have
/// nothing to write about — which is exactly the gap a supporter
/// notices first.
#[derive(Debug, Clone, Copy)]
pub struct LoanWatchEntry {
    pub player_id: u32,
    /// Club he is playing for.
    pub loan_club_id: u32,
    /// Starts and substitute appearances in the current spell.
    pub starts: i32,
    pub sub_appearances: i32,
    pub goals: i32,
    pub assists: i32,
    /// Season average rating × 100, sample-regressed.
    pub rating_x100: i32,
    /// Days until the agreement runs out. Negative once it has.
    pub days_left: i32,
    /// Days the player has been at the borrowing club.
    pub days_elapsed: i32,
    /// The parent club can take him back early.
    pub recall_available: bool,
    /// A permanent option or obligation was written into the deal.
    pub permanent_option: bool,
    /// He is young enough that the loan is a development posting rather
    /// than a way of shifting a wage off the books.
    pub is_prospect: bool,
    /// Feed flags read off the player's own event history.
    pub wants_permanent: bool,
    /// He has told anyone who will listen that he has been lent out
    /// enough and wants a pre-season at the club that owns him.
    pub wants_to_prove_himself: bool,
    pub recall_requested: bool,
    pub development_concern: bool,
    pub form_concern: bool,
    pub level_mismatch: bool,
    pub scored_recently: bool,
}

impl LoanWatchEntry {
    pub fn appearances(&self) -> i32 {
        self.starts + self.sub_appearances
    }

    /// Roughly how many fixtures the borrowing club has played since he
    /// arrived — a week is one game in the leagues this simulation runs.
    pub fn expected_appearances(&self) -> i32 {
        (self.days_elapsed / 7).max(0)
    }

    /// True when he is getting a real share of the football on offer.
    pub fn is_first_choice(&self) -> bool {
        let expected = self.expected_appearances();
        expected >= 4 && self.starts * 3 >= expected * 2
    }

    /// True when the move is not doing what it was signed for.
    pub fn is_frozen_out(&self) -> bool {
        let expected = self.expected_appearances();
        expected >= 5 && self.appearances() * 3 <= expected
    }
}

/// Every loaned-out player belonging to one parent club.
#[derive(Default)]
pub struct ClubLoanWatch {
    pub players: Vec<LoanWatchEntry>,
}

/// One in-flight move for a manager, as it looks from a single club's
/// side of it.
///
/// A pursuit involves two clubs and neither can see it in its own state:
/// the approaches live in a world-level registry, so both the club doing
/// the asking and the club being asked have to be handed their half of
/// it. Which half decides which story gets written — chasing somebody
/// else's manager and having your own chased are not the same week.
#[derive(Debug, Clone, Copy)]
pub struct ManagerPursuit {
    /// The manager being pursued.
    pub staff_id: u32,
    /// The club on the other end: his employer when we are the ones
    /// asking, the suitor when we are the ones being asked.
    pub other_club_id: u32,
    /// True when this club is the one doing the chasing.
    pub we_are_asking: bool,
}

/// Everything in flight around one club's dugout this week.
#[derive(Default)]
pub struct ClubDugoutWatch {
    pub pursuits: Vec<ManagerPursuit>,
}

impl ClubDugoutWatch {
    /// The pursuit worth a line, from this club's point of view. At most
    /// one either way — a paper reporting four simultaneous managerial
    /// links is a paper reporting a database.
    pub fn headline_pursuit(&self, we_are_asking: bool) -> Option<&ManagerPursuit> {
        self.pursuits
            .iter()
            .filter(|pursuit| pursuit.we_are_asking == we_are_asking)
            .min_by_key(|pursuit| (pursuit.staff_id, pursuit.other_club_id))
    }
}

/// Career totals a milestone story needs. The live season counters only
/// cover the current spell, so the ledger has to be folded in.
pub struct CareerRecord;

impl CareerRecord {
    pub fn appearances(player: &Player) -> i32 {
        let live = player.statistics.played as i32 + player.statistics.played_subs as i32;
        let historic: i32 = player
            .statistics_history
            .season_ledger
            .iter()
            .map(|entry| entry.statistics.played as i32 + entry.statistics.played_subs as i32)
            .sum();
        live + historic
    }

    pub fn goals(player: &Player) -> i32 {
        let live = player.statistics.goals as i32;
        let historic: i32 = player
            .statistics_history
            .season_ledger
            .iter()
            .map(|entry| entry.statistics.goals as i32)
            .sum();
        live + historic
    }

    /// The tally a goalkeeper's career is really measured in — the one
    /// column on his record that is his own rather than the team's.
    pub fn clean_sheets(player: &Player) -> i32 {
        let live = player.statistics.clean_sheets as i32;
        let historic: i32 = player
            .statistics_history
            .season_ledger
            .iter()
            .map(|entry| entry.statistics.clean_sheets as i32)
            .sum();
        live + historic
    }

    /// Seasons on the books at the club the player is at now — the
    /// number behind a testimonial. The ledger holds one row per
    /// competition, so the seasons are counted distinctly or a player
    /// with a cup run would age three years in one summer.
    pub fn seasons_at_current_club(player: &Player) -> i32 {
        let ledger = &player.statistics_history.season_ledger;
        let Some(latest) = ledger.last() else {
            return 0;
        };
        let slug = latest.team_slug.as_str();

        let mut seasons: Vec<u16> = ledger
            .iter()
            .filter(|entry| entry.team_slug == slug && !entry.is_loan)
            .map(|entry| entry.season_start_year)
            .collect();
        seasons.sort_unstable();
        seasons.dedup();
        seasons.len() as i32
    }
}

/// A read-only window over a player's own event feed. Every squad story
/// that is not a raw statistic starts here: the simulation already
/// records what happened to a player and why, and the paper's job is to
/// pick the handful of beats a reader would care about rather than to
/// re-derive them.
pub struct RecentEvents<'a> {
    events: &'a [HappinessEvent],
    window_days: u16,
}

impl<'a> RecentEvents<'a> {
    /// The week the edition covers.
    pub const WEEK: u16 = 7;
    /// A slightly longer memory for standing conditions — a rumour that
    /// broke ten days ago is still the story if nothing has moved since.
    pub const FORTNIGHT: u16 = 16;
    /// The window a monthly paper covers. A club's weekly edition asks
    /// what has moved since Monday; a division's monthly review is
    /// written about the whole month, and a link that broke on the 3rd
    /// is still part of that month's transfer story on the 31st.
    pub const MONTH: u16 = 31;

    pub fn week(player: &'a Player) -> Self {
        Self::within(player, Self::WEEK)
    }

    pub fn fortnight(player: &'a Player) -> Self {
        Self::within(player, Self::FORTNIGHT)
    }

    pub fn month(player: &'a Player) -> Self {
        Self::within(player, Self::MONTH)
    }

    /// The feed over an arbitrary window. The named constructors above
    /// are the ones desks should reach for; this exists so a desk that
    /// is shared between publications on different press cycles can be
    /// handed the cycle it is being run on.
    pub fn within(player: &'a Player, window_days: u16) -> Self {
        RecentEvents {
            events: &player.happiness.recent_events,
            window_days,
        }
    }

    pub fn happened(&self, kind: HappinessEventType) -> bool {
        self.lookup(&kind).is_some()
    }

    /// Any of the listed beats, in the order given — the first match
    /// wins, so callers list the loudest version of a story first.
    pub fn any_of(&self, kinds: &[HappinessEventType]) -> Option<HappinessEventType> {
        kinds
            .iter()
            .find(|kind| self.lookup(kind).is_some())
            .cloned()
    }

    pub fn find(&self, kind: HappinessEventType) -> Option<&'a HappinessEvent> {
        self.lookup(&kind)
    }

    fn lookup(&self, kind: &HappinessEventType) -> Option<&'a HappinessEvent> {
        self.events
            .iter()
            .find(|event| &event.event_type == kind && event.days_ago <= self.window_days)
    }

    /// What a returning loanee did while he was away.
    ///
    /// The one record on the subject that survives the journey home: the
    /// live counters a loan spell accumulates are frozen into the season
    /// ledger and reset the moment `on_loan_return` runs, so by press day
    /// the player's own `statistics` describe the fresh parent spell —
    /// which is nothing, because he has not played for them yet. Copy
    /// written off those counters says "0 appearances and 0 goals away
    /// from home" about a man who played thirty games. The return event
    /// carries the real record for exactly this reason.
    pub fn loan_spell(&self) -> Option<LoanSpellRecord> {
        self.find(HappinessEventType::LoanSpellReviewed)
            .and_then(|event| event.context.as_ref())
            .and_then(|context| context.loan_context.as_ref())
            .and_then(|loan| loan.spell)
    }

    /// What a life-outside-football beat was actually about.
    ///
    /// The feed carries one event type for the whole of a footballer's
    /// life away from the pitch — a bereavement, a birth, a family that
    /// has not settled, a veteran wanting one last season at home — and
    /// the desks could only ever see that *something* had happened.
    /// Reading the kind back is what lets the page print the right
    /// sentence rather than a shrug, and it is the difference between
    /// a newspaper and a status board.
    pub fn life_event(&self) -> Option<LifeSimulationDesireKind> {
        self.find(HappinessEventType::LifeSimulationDesire)
            .and_then(|event| event.context.as_ref())
            .and_then(|context| context.life_simulation_desire_context.as_ref())
            .map(|life| life.kind)
    }

    /// What a dressing-room row was actually about.
    ///
    /// The feed has always recorded this and the page has always
    /// printed "two of them went at it" — which is true of a row over
    /// training standards, a row between two men after the same shirt
    /// and a senior pro pulling rank, and useless about all three.
    pub fn conflict_reason(&self) -> Option<TeammateConflictReason> {
        self.find(HappinessEventType::ConflictWithTeammate)
            .and_then(|event| event.context.as_ref())
            .and_then(|context| context.teammate_conflict_context.as_ref())
            .map(|conflict| conflict.reason)
    }

    /// Why the manager left him out.
    ///
    /// The selector records a football reason for every omission —
    /// rested, out of form, wrong profile for the opponent, being
    /// disciplined — and the press could only report that he was not
    /// picked. Which of those it was is the entire difference between
    /// a routine rotation note and a story.
    pub fn omission_reason(&self) -> Option<SelectionOmissionReason> {
        self.find(HappinessEventType::MatchDropped)
            .and_then(|event| event.context.as_ref())
            .and_then(|context| context.selection_context.as_ref())
            .map(|selection| selection.reason)
    }

    /// Why a career ended. A planned farewell and a knee that gave out
    /// are the same line on a squad list and nothing alike on a page.
    pub fn retirement_reason(&self) -> Option<RetirementReason> {
        self.find(HappinessEventType::RetirementAnnounced)
            .and_then(|event| event.context.as_ref())
            .and_then(|context| context.career_stage_context.as_ref())
            .and_then(|stage| stage.retirement_reason)
    }

    /// Which registration rule kept him off the list. "Left out of the
    /// squad" is an administrative sentence; "there was no homegrown
    /// slot left" is a story about how the club has been run.
    pub fn regulation_slot(&self, kind: HappinessEventType) -> Option<RegulationSlotKind> {
        self.find(kind)
            .and_then(|event| event.context.as_ref())
            .and_then(|context| context.regulation_context.as_ref())
            .map(|regulation| regulation.slot_kind)
    }

    /// The club named by a transfer-interest event, when the emit site
    /// knew it. `0` means "somebody, and the paper isn't saying who" —
    /// which is fine for copy that never names the suitor.
    pub fn interested_club(&self, kind: HappinessEventType) -> u32 {
        self.find(kind)
            .and_then(|event| event.context.as_ref())
            .and_then(|context| context.transfer_interest_context.as_ref())
            .and_then(|interest| interest.interested_club_id)
            .unwrap_or(0)
    }

    /// The beat, but only when the paper can put a name to whoever is
    /// asking.
    ///
    /// The club a transfer story names is not decoration: "Milan want
    /// him" and "somebody wants him" are different stories, and the copy
    /// for most of these kinds is built around the name. A beat that
    /// fired without one is therefore not the story it looks like — it
    /// is a mood the player is carrying, and printing it as an approach
    /// puts an anonymous club on the page. Desks that name a suitor ask
    /// for it through here, so the two cases cannot be confused.
    pub fn suitor(&self, kind: HappinessEventType) -> Option<u32> {
        match self.interested_club(kind) {
            0 => None,
            club_id => Some(club_id),
        }
    }

    /// The first of the listed beats that both fired and named somebody.
    ///
    /// Ordered like [`Self::any_of`] — loudest first — but a beat with
    /// no name on it does not block the quieter one behind it that has
    /// one. A rejected bid the paper can attribute is a better story
    /// than a veto it cannot.
    pub fn suitor_of(&self, kinds: &[HappinessEventType]) -> Option<(HappinessEventType, u32)> {
        kinds
            .iter()
            .find_map(|kind| self.suitor(kind.clone()).map(|club| (kind.clone(), club)))
    }
}

/// What the week did to the dressing room as a whole, tallied during
/// the single walk over the club's players.
///
/// Several stories are only true in aggregate — one unhappy player is
/// not "the terraces have turned", and one grateful one is not "the
/// supporters are behind them". Counting them during the walk that was
/// happening anyway is what makes those stories affordable: the
/// alternative is a second pass over every squad in the world.
#[derive(Default)]
pub struct SquadPulse {
    /// Senior players seen — the denominator every share is read against.
    pub seniors: u16,
    /// Silverware — a league title, a cup, a continental prize.
    pub trophy: bool,
    /// Promotion confirmed. Split from `trophy` because "going up" is
    /// its own story, and a bigger one at most clubs.
    pub promotion: bool,
    /// The drop confirmed.
    pub relegated: bool,
    /// A cup final reached and lost.
    pub cup_final_lost: bool,
    /// Continental qualification secured.
    pub europe: bool,
    /// The manager held an inquest after a humiliation.
    pub inquest: bool,
    /// The squad has answered a bad run, or closed ranks around the
    /// manager while the board sharpened its knives.
    pub rallied: bool,
    /// The board put money into the team and the squad noticed.
    pub investment: bool,
    /// Players hearing it from the stands, and from the press box.
    pub fan_praise: u16,
    pub fan_criticism: u16,
    pub media_criticism: u16,
}

impl SquadPulse {
    /// A mood is only the crowd's mood once it is more than one or two
    /// players' bad week. Roughly a fifth of a senior squad, and never
    /// fewer than three voices.
    const SHARE: u16 = 5;
    const FLOOR: u16 = 3;

    pub fn is_widespread(&self, count: u16) -> bool {
        count >= Self::FLOOR && count.saturating_mul(Self::SHARE) >= self.seniors
    }
}

/// How much the local paper cares about a given player, from squad role
/// and standing in the game. Used as a priority nudge so the same story
/// about a key player outranks one about a fringe squad man.
pub struct PlayerStanding;

impl PlayerStanding {
    pub fn importance(player: &Player) -> i32 {
        let role = player
            .contract
            .as_ref()
            .map(|contract| match contract.squad_status {
                PlayerSquadStatus::KeyPlayer => 90,
                PlayerSquadStatus::FirstTeamRegular => 65,
                PlayerSquadStatus::FirstTeamSquadRotation => 35,
                PlayerSquadStatus::HotProspectForTheFuture => 30,
                PlayerSquadStatus::MainBackupPlayer => 15,
                PlayerSquadStatus::DecentYoungster => 10,
                _ => 0,
            })
            .unwrap_or(0);

        role + (player.player_attributes.world_reputation as i32 / 250)
    }
}
