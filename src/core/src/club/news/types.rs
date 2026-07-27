use chrono::NaiveDate;
use std::collections::VecDeque;

/// Which desk filed a story. Rendered by the web layer as the standing
/// kicker above a headline — the same section furniture a real sports
/// page carries, and the only grouping a reader needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewsDesk {
    /// Match reports and anything decided on the pitch.
    Match,
    /// Squad news: form, fitness, discipline, milestones, contracts.
    Squad,
    /// The transfer market — arrivals, departures, speculation.
    Market,
    /// Boardroom and balance sheet.
    Boardroom,
}

impl NewsDesk {
    /// i18n key for the kicker label.
    pub fn i18n_key(self) -> &'static str {
        match self {
            NewsDesk::Match => "news_desk_match",
            NewsDesk::Squad => "news_desk_squad",
            NewsDesk::Market => "news_desk_market",
            NewsDesk::Boardroom => "news_desk_board",
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
    LeagueDefeat,
    Rout,
    HeavyDefeat,
    DerbyWin,
    DerbyDefeat,

    // ── Match desk: run-of-form and standings ─────────────────────
    WinningRun,
    WinlessRun,
    TitleCharge,
    RelegationFight,

    // ── Squad desk ────────────────────────────────────────────────
    HatTrick,
    StarForm,
    KeeperWall,
    InjuryBlow,
    InjuryReturn,
    RedCard,
    YouthDebut,
    MilestoneApps,
    MilestoneGoals,
    PlayerOfMonth,

    // ── Market desk ───────────────────────────────────────────────
    NewSigning,
    RecordSigning,
    FreeSigning,
    LoanArrival,
    PlayerSold,
    StarSold,
    LoanExit,
    TransferSpeculation,
    TransferListed,
    ContractStandoff,

    // ── Boardroom ─────────────────────────────────────────────────
    ManagerPressure,
    BoardBacking,
    MoneyWorries,
}

impl NewsStoryKind {
    /// Every kind the presses can set. The web layer walks this to prove
    /// each one has a headline and a body in every translation bundle,
    /// so adding a variant without its copy fails a test rather than
    /// printing a raw key on the front page.
    pub const ALL: [NewsStoryKind; 34] = [
        NewsStoryKind::LeagueWin,
        NewsStoryKind::LeagueDraw,
        NewsStoryKind::LeagueDefeat,
        NewsStoryKind::Rout,
        NewsStoryKind::HeavyDefeat,
        NewsStoryKind::DerbyWin,
        NewsStoryKind::DerbyDefeat,
        NewsStoryKind::WinningRun,
        NewsStoryKind::WinlessRun,
        NewsStoryKind::TitleCharge,
        NewsStoryKind::RelegationFight,
        NewsStoryKind::HatTrick,
        NewsStoryKind::StarForm,
        NewsStoryKind::KeeperWall,
        NewsStoryKind::InjuryBlow,
        NewsStoryKind::InjuryReturn,
        NewsStoryKind::RedCard,
        NewsStoryKind::YouthDebut,
        NewsStoryKind::MilestoneApps,
        NewsStoryKind::MilestoneGoals,
        NewsStoryKind::PlayerOfMonth,
        NewsStoryKind::NewSigning,
        NewsStoryKind::RecordSigning,
        NewsStoryKind::FreeSigning,
        NewsStoryKind::LoanArrival,
        NewsStoryKind::PlayerSold,
        NewsStoryKind::StarSold,
        NewsStoryKind::LoanExit,
        NewsStoryKind::TransferSpeculation,
        NewsStoryKind::TransferListed,
        NewsStoryKind::ContractStandoff,
        NewsStoryKind::ManagerPressure,
        NewsStoryKind::BoardBacking,
        NewsStoryKind::MoneyWorries,
    ];

    pub fn desk(self) -> NewsDesk {
        match self {
            NewsStoryKind::LeagueWin
            | NewsStoryKind::LeagueDraw
            | NewsStoryKind::LeagueDefeat
            | NewsStoryKind::Rout
            | NewsStoryKind::HeavyDefeat
            | NewsStoryKind::DerbyWin
            | NewsStoryKind::DerbyDefeat
            | NewsStoryKind::WinningRun
            | NewsStoryKind::WinlessRun
            | NewsStoryKind::TitleCharge
            | NewsStoryKind::RelegationFight => NewsDesk::Match,

            NewsStoryKind::HatTrick
            | NewsStoryKind::StarForm
            | NewsStoryKind::KeeperWall
            | NewsStoryKind::InjuryBlow
            | NewsStoryKind::InjuryReturn
            | NewsStoryKind::RedCard
            | NewsStoryKind::YouthDebut
            | NewsStoryKind::MilestoneApps
            | NewsStoryKind::MilestoneGoals
            | NewsStoryKind::PlayerOfMonth => NewsDesk::Squad,

            NewsStoryKind::NewSigning
            | NewsStoryKind::RecordSigning
            | NewsStoryKind::FreeSigning
            | NewsStoryKind::LoanArrival
            | NewsStoryKind::PlayerSold
            | NewsStoryKind::StarSold
            | NewsStoryKind::LoanExit
            | NewsStoryKind::TransferSpeculation
            | NewsStoryKind::TransferListed
            | NewsStoryKind::ContractStandoff => NewsDesk::Market,

            NewsStoryKind::ManagerPressure
            | NewsStoryKind::BoardBacking
            | NewsStoryKind::MoneyWorries => NewsDesk::Boardroom,
        }
    }

    /// Stable identifier the web layer expands into the headline and
    /// body translation keys. Never reuse a stem across kinds.
    pub fn key_stem(self) -> &'static str {
        match self {
            NewsStoryKind::LeagueWin => "league_win",
            NewsStoryKind::LeagueDraw => "league_draw",
            NewsStoryKind::LeagueDefeat => "league_defeat",
            NewsStoryKind::Rout => "rout",
            NewsStoryKind::HeavyDefeat => "heavy_defeat",
            NewsStoryKind::DerbyWin => "derby_win",
            NewsStoryKind::DerbyDefeat => "derby_defeat",
            NewsStoryKind::WinningRun => "winning_run",
            NewsStoryKind::WinlessRun => "winless_run",
            NewsStoryKind::TitleCharge => "title_charge",
            NewsStoryKind::RelegationFight => "relegation_fight",
            NewsStoryKind::HatTrick => "hat_trick",
            NewsStoryKind::StarForm => "star_form",
            NewsStoryKind::KeeperWall => "keeper_wall",
            NewsStoryKind::InjuryBlow => "injury_blow",
            NewsStoryKind::InjuryReturn => "injury_return",
            NewsStoryKind::RedCard => "red_card",
            NewsStoryKind::YouthDebut => "youth_debut",
            NewsStoryKind::MilestoneApps => "milestone_apps",
            NewsStoryKind::MilestoneGoals => "milestone_goals",
            NewsStoryKind::PlayerOfMonth => "player_of_month",
            NewsStoryKind::NewSigning => "new_signing",
            NewsStoryKind::RecordSigning => "record_signing",
            NewsStoryKind::FreeSigning => "free_signing",
            NewsStoryKind::LoanArrival => "loan_arrival",
            NewsStoryKind::PlayerSold => "player_sold",
            NewsStoryKind::StarSold => "star_sold",
            NewsStoryKind::LoanExit => "loan_exit",
            NewsStoryKind::TransferSpeculation => "transfer_speculation",
            NewsStoryKind::TransferListed => "transfer_listed",
            NewsStoryKind::ContractStandoff => "contract_standoff",
            NewsStoryKind::ManagerPressure => "manager_pressure",
            NewsStoryKind::BoardBacking => "board_backing",
            NewsStoryKind::MoneyWorries => "money_worries",
        }
    }

    /// Newsworthiness before per-story modifiers. Calibrated against
    /// how a local paper really ranks its page: a derby and a record
    /// signing lead over a routine win, a routine win leads over a
    /// contract standoff, and boardroom mood only reaches the front
    /// page in a crisis.
    pub fn base_priority(self) -> u16 {
        match self {
            NewsStoryKind::DerbyWin | NewsStoryKind::DerbyDefeat => 720,
            NewsStoryKind::RecordSigning => 700,
            NewsStoryKind::StarSold => 660,
            NewsStoryKind::Rout | NewsStoryKind::HeavyDefeat => 600,
            NewsStoryKind::HatTrick => 580,
            NewsStoryKind::TitleCharge | NewsStoryKind::RelegationFight => 540,
            NewsStoryKind::ManagerPressure => 520,
            NewsStoryKind::WinningRun | NewsStoryKind::WinlessRun => 500,
            NewsStoryKind::NewSigning => 480,
            NewsStoryKind::PlayerSold => 460,
            NewsStoryKind::LeagueWin | NewsStoryKind::LeagueDefeat => 440,
            NewsStoryKind::MilestoneGoals | NewsStoryKind::MilestoneApps => 420,
            NewsStoryKind::PlayerOfMonth => 410,
            NewsStoryKind::InjuryBlow => 400,
            NewsStoryKind::LeagueDraw => 380,
            NewsStoryKind::RedCard => 370,
            NewsStoryKind::YouthDebut => 360,
            NewsStoryKind::StarForm => 350,
            NewsStoryKind::MoneyWorries => 340,
            NewsStoryKind::FreeSigning | NewsStoryKind::LoanArrival => 320,
            NewsStoryKind::KeeperWall => 300,
            NewsStoryKind::TransferListed => 290,
            NewsStoryKind::ContractStandoff => 280,
            NewsStoryKind::TransferSpeculation => 260,
            NewsStoryKind::LoanExit => 240,
            NewsStoryKind::BoardBacking => 220,
            NewsStoryKind::InjuryReturn => 200,
        }
    }

    /// How the editor decides whether a story has already been printed.
    pub fn recurrence(self) -> NewsRecurrence {
        match self {
            // Something that happened on a date. It can only be
            // detected once, so the back catalogue is never consulted.
            NewsStoryKind::LeagueWin
            | NewsStoryKind::LeagueDraw
            | NewsStoryKind::LeagueDefeat
            | NewsStoryKind::Rout
            | NewsStoryKind::HeavyDefeat
            | NewsStoryKind::DerbyWin
            | NewsStoryKind::DerbyDefeat
            | NewsStoryKind::HatTrick
            | NewsStoryKind::RedCard
            | NewsStoryKind::PlayerOfMonth
            | NewsStoryKind::NewSigning
            | NewsStoryKind::RecordSigning
            | NewsStoryKind::FreeSigning
            | NewsStoryKind::LoanArrival
            | NewsStoryKind::PlayerSold
            | NewsStoryKind::StarSold
            | NewsStoryKind::LoanExit => NewsRecurrence::Event,

            // A number that moves. The paper runs it again as soon as
            // the number does — "make it five in a row".
            NewsStoryKind::WinningRun
            | NewsStoryKind::WinlessRun
            | NewsStoryKind::StarForm
            | NewsStoryKind::KeeperWall
            | NewsStoryKind::MilestoneApps
            | NewsStoryKind::MilestoneGoals => NewsRecurrence::Progress,

            // A condition that persists. Printing it every week would
            // read like a stuck record, so it waits its turn.
            NewsStoryKind::TitleCharge
            | NewsStoryKind::RelegationFight
            | NewsStoryKind::InjuryBlow
            | NewsStoryKind::InjuryReturn
            | NewsStoryKind::YouthDebut
            | NewsStoryKind::TransferSpeculation
            | NewsStoryKind::TransferListed
            | NewsStoryKind::ContractStandoff
            | NewsStoryKind::ManagerPressure
            | NewsStoryKind::BoardBacking
            | NewsStoryKind::MoneyWorries => NewsRecurrence::Standing,
        }
    }

    /// True when several stories of this kind can share one edition.
    /// Match reports can (a club plays twice in a week); a paper never
    /// runs two "board backs the manager" pieces side by side.
    pub fn allows_repeat(self) -> bool {
        matches!(
            self,
            NewsStoryKind::LeagueWin
                | NewsStoryKind::LeagueDraw
                | NewsStoryKind::LeagueDefeat
                | NewsStoryKind::Rout
                | NewsStoryKind::HeavyDefeat
                | NewsStoryKind::DerbyWin
                | NewsStoryKind::DerbyDefeat
                | NewsStoryKind::HatTrick
                | NewsStoryKind::InjuryBlow
                | NewsStoryKind::NewSigning
                | NewsStoryKind::PlayerSold
                | NewsStoryKind::LoanArrival
                | NewsStoryKind::LoanExit
                | NewsStoryKind::FreeSigning
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
    /// Opponent team (match reports) or the other club (transfers).
    /// `0` when the story has no second party.
    pub other_id: u32,
    /// Primary figure: goals scored, days out, league position, …
    pub a: i32,
    /// Secondary figure: goals conceded, points, rating × 100, …
    pub b: i32,
    /// Transfer fee or other money amount. `0` when not a money story.
    pub money: i64,
}

impl NewsStory {
    pub fn new(kind: NewsStoryKind, date: NaiveDate) -> Self {
        NewsStory {
            kind,
            date,
            priority: kind.base_priority(),
            player_id: 0,
            other_id: 0,
            a: 0,
            b: 0,
            money: 0,
        }
    }

    pub fn about(mut self, player_id: u32) -> Self {
        self.player_id = player_id;
        self
    }

    pub fn against(mut self, other_id: u32) -> Self {
        self.other_id = other_id;
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

/// One line in the ruled results panel — the fixtures column every
/// football paper prints regardless of what else happened that week.
#[derive(Debug, Clone, Copy)]
pub struct IssueResult {
    pub date: NaiveDate,
    pub opponent_team_id: u32,
    pub goals_for: u8,
    pub goals_against: u8,
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

/// The club's local paper: who prints it and what it has printed
/// lately. Bounded to [`Self::MAX_ISSUES`] editions so the world's full
/// club list keeps a fixed, small memory cost.
#[derive(Debug, Clone)]
pub struct ClubNewsroom {
    /// Which masthead noun this club's paper uses. Stable for the life
    /// of the club so the title never changes under the reader.
    pub masthead: u8,
    /// Number the next edition will carry.
    pub next_number: u32,
    /// Newest edition first.
    pub issues: VecDeque<NewspaperIssue>,
}

impl ClubNewsroom {
    /// Masthead nouns available in the translation bundles.
    pub const MASTHEAD_COUNT: u8 = 6;
    /// Editions kept on the shelf.
    pub const MAX_ISSUES: usize = 5;
    /// Stories one edition can hold: a lead, two secondaries, and a
    /// column of briefs.
    pub const MAX_STORIES: usize = 9;

    /// Assign a masthead deterministically from the club id so the same
    /// world always prints the same paper titles.
    pub fn for_club(club_id: u32) -> Self {
        ClubNewsroom {
            masthead: (club_id.wrapping_mul(2_654_435_761) >> 13) as u8 % Self::MASTHEAD_COUNT,
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
        const KEYS: [&str; ClubNewsroom::MASTHEAD_COUNT as usize] = [
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

impl Default for ClubNewsroom {
    fn default() -> Self {
        ClubNewsroom::for_club(0)
    }
}

#[cfg(test)]
mod tests {
    use super::{ClubNewsroom, NewsStoryKind, NewspaperIssue, PressMood};
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

    #[test]
    fn the_shelf_holds_only_the_last_five_editions() {
        let mut newsroom = ClubNewsroom::for_club(42);

        for number in 1..=8 {
            newsroom.publish(Press::issue(number));
        }

        assert_eq!(newsroom.issues.len(), ClubNewsroom::MAX_ISSUES);
        assert_eq!(newsroom.latest().unwrap().number, 8);
        assert_eq!(newsroom.issues.back().unwrap().number, 4);
        assert_eq!(newsroom.next_number, 9);
    }

    #[test]
    fn a_masthead_is_stable_and_in_range() {
        for club_id in [1u32, 7, 4242, 999_999] {
            let newsroom = ClubNewsroom::for_club(club_id);
            assert_eq!(newsroom.masthead, ClubNewsroom::for_club(club_id).masthead);
            assert!(newsroom.masthead < ClubNewsroom::MASTHEAD_COUNT);
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
