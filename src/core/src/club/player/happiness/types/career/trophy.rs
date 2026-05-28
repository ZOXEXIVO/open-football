//! Trophy-win explanation payload — carried alongside `TrophyWon` and
//! `DomesticCupWon` happiness events so the renderer can name the
//! competition the player just won, summarise how involved they were in
//! the campaign (apps / starts / sub apps / goals / assists / clean
//! sheets), and flag whether they made the final.
//!
//! Storing this on the event (rather than reconstructing it from cup
//! stats at render time) makes the event self-contained: by the time
//! the UI renders it, the player may have moved clubs, the cup season
//! may have rolled over, or their per-competition cup stats may have
//! been snapshotted — none of which can corrupt the original moment.

/// Which silverware the player just won. Lets the renderer pick
/// competition-specific copy ("Won the FA Cup" vs "Won the league
/// title") without parsing the event type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrophyKind {
    /// League / divisional championship.
    LeagueTitle,
    /// Country's main knockout cup (FA Cup, Copa del Rey, Coppa Italia,
    /// …). Distinct from `LeagueTitle` so the renderer can produce
    /// trophy-specific copy and the cooldown can be keyed separately.
    DomesticCup,
    /// Continental knockout trophy (UCL, UEL, Copa Libertadores, …).
    ContinentalCup,
    /// Lower-division championship that also produced promotion — the
    /// title is the silverware, promotion is the headline.
    PromotionTitle,
}

impl TrophyKind {
    pub fn as_token(&self) -> &'static str {
        match self {
            TrophyKind::LeagueTitle => "trophy_kind_league_title",
            TrophyKind::DomesticCup => "trophy_kind_domestic_cup",
            TrophyKind::ContinentalCup => "trophy_kind_continental_cup",
            TrophyKind::PromotionTitle => "trophy_kind_promotion_title",
        }
    }
}

/// Trophy-event explanation payload. All quantitative fields are
/// `Option` so emit sites attach what they know — missing fields
/// collapse to the trophy-kind line on the renderer.
#[derive(Debug, Clone)]
pub struct TrophyEventContext {
    pub trophy_kind: TrophyKind,
    /// Identifier of the underlying competition. For domestic cups this
    /// is the cup's inner league id; for league titles it's the league
    /// id; for continental trophies the continental league id.
    pub competition_id: Option<u32>,
    /// Stable slug for routing — the player awards UI uses this to
    /// build the right link target (e.g. `/cups/<slug>` for cup medals
    /// rather than the league-awards page).
    pub competition_slug: Option<String>,
    /// Display name of the competition. Renderer falls back to a
    /// translated form of [`TrophyKind`] when absent.
    pub competition_name: Option<String>,
    /// Which side won. Useful for the renderer to fetch the club name
    /// from the world without re-deriving the champion from cup state.
    pub winner_team_id: Option<u32>,
    /// Total cup appearances this edition (`starts + used_sub_apps`).
    pub apps: Option<u16>,
    /// Cup starts this edition.
    pub starts: Option<u16>,
    /// Cup substitute appearances this edition.
    pub used_sub_apps: Option<u16>,
    /// Cup goals this edition.
    pub goals: Option<u16>,
    /// Cup assists this edition.
    pub assists: Option<u16>,
    /// Goalkeeper / defender clean sheets in the cup edition. Renderer
    /// hides it for outfield non-defenders even if populated.
    pub clean_sheets: Option<u16>,
    /// Reliability-adjusted average cup rating for the edition.
    pub avg_rating: Option<f32>,
    /// True iff the player was on the pitch (starter or used sub) in
    /// the final. Lets the renderer headline "Played in the final"
    /// without re-reading match details.
    pub final_appearance: bool,
}

impl TrophyEventContext {
    pub fn new(trophy_kind: TrophyKind) -> Self {
        Self {
            trophy_kind,
            competition_id: None,
            competition_slug: None,
            competition_name: None,
            winner_team_id: None,
            apps: None,
            starts: None,
            used_sub_apps: None,
            goals: None,
            assists: None,
            clean_sheets: None,
            avg_rating: None,
            final_appearance: false,
        }
    }

    pub fn with_competition_id(mut self, id: u32) -> Self {
        self.competition_id = Some(id);
        self
    }

    pub fn with_competition_slug(mut self, slug: String) -> Self {
        self.competition_slug = Some(slug);
        self
    }

    pub fn with_competition_name(mut self, name: String) -> Self {
        self.competition_name = Some(name);
        self
    }

    pub fn with_winner_team_id(mut self, team_id: u32) -> Self {
        self.winner_team_id = Some(team_id);
        self
    }

    pub fn with_apps(mut self, apps: u16) -> Self {
        self.apps = Some(apps);
        self
    }

    pub fn with_starts(mut self, starts: u16) -> Self {
        self.starts = Some(starts);
        self
    }

    pub fn with_used_sub_apps(mut self, sub_apps: u16) -> Self {
        self.used_sub_apps = Some(sub_apps);
        self
    }

    pub fn with_goals(mut self, goals: u16) -> Self {
        self.goals = Some(goals);
        self
    }

    pub fn with_assists(mut self, assists: u16) -> Self {
        self.assists = Some(assists);
        self
    }

    pub fn with_clean_sheets(mut self, cs: u16) -> Self {
        self.clean_sheets = Some(cs);
        self
    }

    pub fn with_avg_rating(mut self, rating: f32) -> Self {
        self.avg_rating = Some(rating);
        self
    }

    pub fn with_final_appearance(mut self, on_pitch: bool) -> Self {
        self.final_appearance = on_pitch;
        self
    }
}
