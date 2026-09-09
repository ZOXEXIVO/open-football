//! Monthly squad-status assignment.
//!
//! Each player gets a `PlayerSquadStatus` (KeyPlayer / FirstTeamRegular /
//! RotationPlayer / Backup / NotNeeded / etc.) derived from their CA rank
//! *within their own position group*. Ranking against the whole squad puts
//! a backup goalkeeper at the bottom of a CA-sorted list dominated by
//! outfield stars — you'd get `NotNeeded` for every 3rd/4th keeper at an
//! elite club, and every downstream code path keyed on squad status would
//! treat them as surplus.
//!
//! A genuine move on the senior ladder is no longer silent: squad status
//! drives the player's expected start share, so a promotion or demotion
//! is one of the most consequential things a coach can tell a player.
//! The updater emits a `SquadStatusChange` morale event, shaped by the
//! direction of the move, the player's recent form (demoted while
//! performing reads as an injustice), personality, and whether the head
//! coach has the man-management to deliver the news in a conversation
//! rather than via the team-sheet.

use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::happiness::PlayingTimeFrustrationConfig;
use crate::club::team::Team;
use crate::league::Season;
use crate::utils::DateUtils;
use crate::{
    ClubLevelAnchor, HappinessEventType, MatchExperienceBackground, Player,
    PlayerFieldPositionGroup, PlayerSquadStatus,
};
use chrono::{Duration, NaiveDate};
use std::collections::HashMap;

/// The squad's match sample over a rolling window — this season and the
/// last completed one — so the honesty cap on a label can read what a
/// player has been getting LATELY rather than over his whole stay.
///
/// The since-join counters the cap used to read never forget: a man who
/// started ninety games in his first three seasons and eight in the next
/// three still carried a lifetime share above the regular bar, so the
/// label the renewal clock, the asset classifier and the listing sweeps
/// all trusted said "First Team Regular" about a 35-year-old the coach
/// had stopped picking years earlier.
struct RecentInvolvementSample {
    /// Official matches the club has played this season — the busiest
    /// squad member's appearances, the same proxy the evidence context
    /// reads.
    club_matches_this_season: u16,
    /// The most recent completed season across the squad, if any.
    last_completed_year: Option<u16>,
    /// The same proxy for that season.
    club_matches_last_season: u16,
}

impl RecentInvolvementSample {
    fn of_team(team: &Team, date: NaiveDate) -> Self {
        let current_year = Season::from_date(date).start_year;
        let own = || team.players.iter().filter(|p| !p.is_on_loan());
        let club_matches_this_season = own().map(Self::official_appearances).max().unwrap_or(0);
        let last_completed_year = own()
            .flat_map(|p| p.statistics_history.items.iter())
            .map(|h| h.season.start_year)
            .filter(|&year| year < current_year)
            .max();
        let club_matches_last_season = last_completed_year
            .map(|year| {
                own()
                    .map(|p| Self::season_games(p, year, 1.0, 1.0))
                    .fold(0.0_f32, f32::max)
                    .round() as u16
            })
            .unwrap_or(0);
        RecentInvolvementSample {
            club_matches_this_season,
            last_completed_year,
            club_matches_last_season,
        }
    }

    /// Official (league + cup) appearances this season.
    fn official_appearances(player: &Player) -> u16 {
        player.statistics.played
            + player.statistics.played_subs
            + player.cup_statistics.played
            + player.cup_statistics.played_subs
    }

    /// Weighted games in one completed season, parent and loan spells
    /// summed: a start counts `start_weight`, a substitute appearance
    /// `sub_weight`.
    fn season_games(player: &Player, year: u16, start_weight: f32, sub_weight: f32) -> f32 {
        player
            .statistics_history
            .items
            .iter()
            .filter(|h| h.season.start_year == year)
            .map(|h| {
                h.statistics.played as f32 * start_weight
                    + h.statistics.played_subs as f32 * sub_weight
            })
            .sum()
    }

    /// `(weighted involvement, eligible matches)` for one player over the
    /// window, on the same start / sub weighting as the since-join score.
    fn player_window(&self, player: &Player, cfg: &PlayingTimeFrustrationConfig) -> (f32, f32) {
        let this_season = (player.statistics.played + player.cup_statistics.played) as f32
            * cfg.start_weight
            + (player.statistics.played_subs + player.cup_statistics.played_subs) as f32
                * cfg.sub_app_weight;
        let last_season = self
            .last_completed_year
            .map(|year| Self::season_games(player, year, cfg.start_weight, cfg.sub_app_weight))
            .unwrap_or(0.0);
        let eligible = (self.club_matches_this_season + self.club_matches_last_season) as f32;
        (this_season + last_season, eligible)
    }
}

pub struct SquadStatusUpdater;

impl SquadStatusUpdater {
    /// Head-coach man-management at or above this delivers role changes
    /// in a conversation, halving the sting of a demotion.
    const MAN_MANAGEMENT_TO_EXPLAIN: u8 = 12;
    /// Morale points per senior-ladder step.
    const MAGNITUDE_PER_STEP: f32 = 2.2;
    /// Days before another status-change event can land on the same
    /// player — CA-rank boundaries can oscillate month to month and the
    /// ledger shouldn't churn with them.
    const EVENT_COOLDOWN_DAYS: u16 = 90;

    /// Recompute every player's `contract.squad_status` against the CA
    /// distribution of their position group, emitting a morale event
    /// for genuine senior-ladder moves. Only squads that own role labels
    /// ([`crate::TeamType::owns_squad_status`]) run the full re-rank;
    /// development and parking squads take the administrative pass in
    /// [`Self::apply_development_labels`] instead.
    pub fn apply(team: &mut Team, date: NaiveDate) {
        if !team.team_type.owns_squad_status() {
            Self::apply_development_labels(team, date);
            return;
        }
        let mut by_group: HashMap<PlayerFieldPositionGroup, Vec<u8>> = HashMap::new();
        for p in team.players.iter() {
            let g = p.position().position_group();
            by_group
                .entry(g)
                .or_default()
                .push(p.player_attributes.current_ability);
        }
        for cas in by_group.values_mut() {
            cas.sort_unstable_by(|a, b| b.cmp(a));
        }

        let explains_role_changes = team
            .staffs
            .social_head_coach()
            .map(|s| s.staff_attributes.mental.man_management >= Self::MAN_MANAGEMENT_TO_EXPLAIN)
            .unwrap_or(false);
        let team_reputation = team.reputation.world;
        // What this club expects of a starter: the rank inside the group
        // hands out the labels, the club's level caps them.
        let club_level = ClubLevelAnchor::for_reputation(team.reputation.overall_score());
        let recent = RecentInvolvementSample::of_team(team, date);

        for player in team.players.iter_mut() {
            let group = player.position().position_group();
            let ca = player.player_attributes.current_ability;
            let age = DateUtils::age(player.birth_date, date);

            // Honesty ceiling from actual involvement, computed before the
            // mutable contract borrow (it reads the whole player).
            let involvement_ceiling = Self::involvement_status_ceiling(player, date, &recent);
            // The returnee verdict: a fresh returnee whose loan record
            // holds up at this club's level gets his label graduated.
            let record_floor = Self::returnee_record_floor(player, team_reputation, date);

            let mut transition: Option<(u8, u8)> = None;
            let mut backed_after_loan = false;
            if let Some(ref mut contract) = player.contract {
                let group_cas = by_group.get(&group).map(|v| v.as_slice()).unwrap_or(&[]);
                let old_rank = Self::senior_rank(&contract.squad_status);
                let mut new_status = PlayerSquadStatus::calculate(ca, age, group, group_cas);
                // Don't let a CA-strong label over-promise a role the player
                // isn't actually getting: a keeper stuck behind the number
                // one, or an out-of-favour senior, reads as a backup rather
                // than a first-team regular. Only demotes senior labels, and
                // only with enough match evidence.
                if let (Some(ca_rank), Some(ceiling)) =
                    (Self::senior_rank(&new_status), involvement_ceiling.as_ref())
                {
                    if Self::senior_rank(ceiling).unwrap_or(u8::MAX) < ca_rank {
                        new_status = ceiling.clone();
                    }
                }
                // Bidirectional: a player whose actual minutes still justify
                // his role isn't demoted just because a pricier signing now
                // outranks him on CA — a regular still starting every week is
                // not relabelled a backup (which also suppresses the unearned
                // demotion morale event). Never floats him above his prior
                // standing, so it only ever cancels an unjust demotion.
                if let Some(ceiling) = involvement_ceiling.as_ref() {
                    let ceil_rank = Self::senior_rank(ceiling).unwrap_or(0);
                    let floor_rank = ceil_rank.min(old_rank.unwrap_or(0));
                    if Self::senior_rank(&new_status).unwrap_or(0) < floor_rank {
                        new_status = if old_rank.unwrap_or(0) <= ceil_rank {
                            contract.squad_status.clone()
                        } else {
                            ceiling.clone()
                        };
                    }
                }
                // The club's own level caps whatever rank and minutes
                // agreed on: a man far below what this club recruits at is
                // rotation depth or a backup even while he starts every
                // week, because he starts for want of anyone better. The
                // returnee verdict and a signing promise below still floor
                // it — the record was already read at this club's level,
                // and a promise is a promise.
                new_status =
                    PlayerSquadStatus::cap_at_level(new_status, ca, age, group, Some(club_level));
                // The returnee verdict: the loan record is admissible
                // evidence. A young returnee otherwise stays a "prospect"
                // by age and a fringe senior stays a backup by CA rank —
                // while every club-side system (selection, depth, renewal,
                // asset class) reads the stale label. The record floor
                // graduates the label, and the commitment is bound as a
                // role promise so next month's CA-rank recompute can't
                // silently walk the plan back while he settles in; from
                // there the ordinary promise/breach machinery owns it.
                if let Some(floor) = record_floor.as_ref() {
                    let floor_rank = Self::senior_rank(floor).unwrap_or(0);
                    if Self::senior_rank(&new_status).unwrap_or(0) < floor_rank {
                        new_status = floor.clone();
                        backed_after_loan = true;
                        let commit_until = date + Duration::days(150);
                        let keep_existing = contract
                            .promised_squad_status
                            .as_ref()
                            .map(|(promised, _)| {
                                Self::senior_rank(promised).unwrap_or(0) >= floor_rank
                            })
                            .unwrap_or(false);
                        if !keep_existing {
                            contract.promised_squad_status = Some((floor.clone(), commit_until));
                        }
                    }
                }
                // Honor an unexpired role promise as a floor: the club
                // committed to it at signing, so never recompute below it —
                // otherwise the promise the buyer paid for is wiped within a
                // month and the breach is hidden (expected_start_share reads
                // this same field). Past its expiry the promise stops binding.
                if let Some((promised, until)) = contract.promised_squad_status.clone() {
                    if date <= until {
                        if Self::senior_rank(&promised).unwrap_or(0)
                            > Self::senior_rank(&new_status).unwrap_or(0)
                        {
                            new_status = promised;
                        }
                    } else {
                        contract.promised_squad_status = None;
                    }
                }
                contract.squad_status = new_status;
                let new_rank = Self::senior_rank(&contract.squad_status);
                if let (Some(old), Some(new)) = (old_rank, new_rank) {
                    if old != new {
                        transition = Some((old, new));
                    }
                }
            }

            if backed_after_loan {
                // The verdict said out loud — the specific beat replaces
                // the generic status-change note (a prospect→senior move
                // has no senior-ladder transition anyway).
                let magnitude = HappinessConfig::default().catalog.backed_after_loan_return;
                player.happiness.add_event_with_cooldown(
                    HappinessEventType::BackedAfterLoanReturn,
                    magnitude,
                    300,
                );
                continue;
            }
            if let Some((old, new)) = transition {
                let steps = new as f32 - old as f32;
                let mut magnitude = Self::MAGNITUDE_PER_STEP * steps;
                if steps < 0.0 {
                    // Demoted while performing reads as an injustice;
                    // ambition amplifies the wound, professionalism
                    // absorbs some of it, and a coach who actually
                    // explains the decision takes most of the edge off.
                    let pos_group = player.position().position_group();
                    let form = player.statistics.average_rating_realistic(pos_group);
                    let apps = player.statistics.played + player.statistics.played_subs;
                    if apps >= 3 && form >= 7.0 {
                        magnitude *= 1.5;
                    }
                    let ambition = (player.attributes.ambition / 20.0).clamp(0.0, 1.0);
                    let professionalism =
                        (player.attributes.professionalism / 20.0).clamp(0.0, 1.0);
                    magnitude *= 1.0 + ambition * 0.3 - professionalism * 0.25;
                    if explains_role_changes {
                        magnitude *= 0.6;
                    }
                } else {
                    // Promotions land softer than demotions sting.
                    magnitude *= 0.7;
                }
                player.happiness.add_event_with_cooldown(
                    HappinessEventType::SquadStatusChange,
                    magnitude.clamp(-6.0, 5.0),
                    Self::EVENT_COOLDOWN_DAYS,
                );
            }
        }
    }

    /// Label pass for squads that don't own role labels (Reserve,
    /// U20..U23 — see [`crate::TeamType::owns_squad_status`]). Youngsters
    /// get their prospect label refreshed; a senior keeps his club-level
    /// label collapsed to at most backup via
    /// [`PlayerSquadStatus::calculate_for_team`] — a Main-team backup
    /// moved down stays a backup, and a stale "Key Player" tag can't
    /// survive in a squad that has no key players. An unexpired signing
    /// promise is still honored as a floor: parking a promised regular in
    /// the reserves must surface as the broken promise it is (playing-time
    /// expectations read this label), not be relabelled away. Purely
    /// administrative — no `SquadStatusChange` events; the demotion story
    /// belongs to the move that parked the player here, not to this
    /// relabel.
    fn apply_development_labels(team: &mut Team, date: NaiveDate) {
        let team_type = team.team_type;
        let mut by_group: HashMap<PlayerFieldPositionGroup, Vec<u8>> = HashMap::new();
        for p in team.players.iter() {
            let g = p.position().position_group();
            by_group
                .entry(g)
                .or_default()
                .push(p.player_attributes.current_ability);
        }
        for cas in by_group.values_mut() {
            cas.sort_unstable_by(|a, b| b.cmp(a));
        }

        for player in team.players.iter_mut() {
            let group = player.position().position_group();
            let ca = player.player_attributes.current_ability;
            let age = DateUtils::age(player.birth_date, date);
            if let Some(ref mut contract) = player.contract {
                let group_cas = by_group.get(&group).map(|v| v.as_slice()).unwrap_or(&[]);
                let mut new_status = PlayerSquadStatus::calculate_for_team(
                    team_type,
                    &contract.squad_status,
                    ca,
                    age,
                    group,
                    group_cas,
                );
                if let Some((promised, until)) = contract.promised_squad_status.clone() {
                    if date <= until {
                        if Self::senior_rank(&promised).unwrap_or(0)
                            > Self::senior_rank(&new_status).unwrap_or(0)
                        {
                            new_status = promised;
                        }
                    } else {
                        contract.promised_squad_status = None;
                    }
                }
                contract.squad_status = new_status;
            }
        }
    }

    /// Senior-ladder rank for promotion/demotion detection. Youth
    /// labels return `None` — a prospect's label shifting with age is
    /// re-classification, not a conversation about his role.
    fn senior_rank(status: &PlayerSquadStatus) -> Option<u8> {
        match status {
            PlayerSquadStatus::KeyPlayer => Some(5),
            PlayerSquadStatus::FirstTeamRegular => Some(4),
            PlayerSquadStatus::FirstTeamSquadRotation => Some(3),
            PlayerSquadStatus::MainBackupPlayer => Some(2),
            PlayerSquadStatus::NotNeeded => Some(0),
            _ => None,
        }
    }

    /// The returnee verdict's record floor: for a player freshly back
    /// from a loan he genuinely played through, the senior status his
    /// record justifies at THIS club's level — or `None` when there is
    /// no fresh return, no real loan record, or the record doesn't
    /// clear the rotation bar once level-adjusted. Derived from the
    /// same [`MatchExperienceBackground`] expectation floor the player
    /// himself reasons with, so the club's verdict and the player's
    /// own bar can never disagree about what the record was worth.
    fn returnee_record_floor(
        player: &Player,
        team_reputation: u16,
        date: NaiveDate,
    ) -> Option<PlayerSquadStatus> {
        /// The verdict belongs to the arrival window — one look at the
        /// record, not a rolling entitlement.
        const VERDICT_WINDOW_DAYS: i64 = 45;
        /// A real run of loan starts before the record speaks.
        const MIN_RECORD_LOAN_STARTS: u16 = 12;

        let fresh = player
            .days_since_transfer(date)
            .map(|d| (0..=VERDICT_WINDOW_DAYS).contains(&d))
            .unwrap_or(false);
        if !fresh {
            return None;
        }
        let background = MatchExperienceBackground::from_player(player);
        if background.recent_loan_starts < MIN_RECORD_LOAN_STARTS {
            return None;
        }
        let floor_share = background.expected_start_share_floor(team_reputation);
        if floor_share >= 0.50 {
            Some(PlayerSquadStatus::FirstTeamRegular)
        } else if floor_share >= 0.28 {
            Some(PlayerSquadStatus::FirstTeamSquadRotation)
        } else {
            None
        }
    }

    /// Highest senior status the player's ACTUAL match involvement justifies,
    /// or `None` when there isn't enough evidence to judge — too few eligible
    /// matches, or still settling in after a transfer. This is the honesty
    /// cap: a label may not claim a bigger role than the player is getting.
    ///
    /// Never demotes below `MainBackupPlayer` on appearances alone — a
    /// contracted senior who simply isn't picked is a backup, not surplus;
    /// genuine `NotNeeded` stays a CA-rank verdict (and is handled by the
    /// surplus-release systems, not here). The share thresholds sit a little
    /// under each status's `expected_start_share` so only a player clearly
    /// below his tier is demoted, giving hysteresis against month-to-month
    /// CA-rank wobble.
    fn involvement_status_ceiling(
        player: &Player,
        date: NaiveDate,
        recent: &RecentInvolvementSample,
    ) -> Option<PlayerSquadStatus> {
        /// Enough eligible matches (~a third of a season) to trust the share.
        const MIN_ELIGIBLE_TO_JUDGE: u16 = 10;
        /// Give a player a fair chunk of a season at the club before his
        /// label can be demoted on appearances.
        const MIN_DAYS_AT_CLUB: i64 = 120;
        /// Past a year at the club the since-join counters stop being a
        /// recent record and become a career one; from here the cap reads
        /// this season and the last.
        const ROLLING_WINDOW_DAYS: i64 = 365;

        let opp = player.playing_time_opportunity(date);
        if opp.days_since_join < MIN_DAYS_AT_CLUB {
            return None;
        }

        let cfg = PlayingTimeFrustrationConfig::default();
        let (involvement, eligible) = if opp.days_since_join >= ROLLING_WINDOW_DAYS {
            recent.player_window(player, &cfg)
        } else {
            (
                opp.actual_involvement_score(&cfg),
                opp.eligible_official_matches_since_join as f32,
            )
        };
        if eligible < MIN_ELIGIBLE_TO_JUDGE as f32 {
            return None;
        }
        let share = (involvement / eligible).clamp(0.0, 1.0);

        let ceiling = if share >= 0.50 {
            PlayerSquadStatus::KeyPlayer
        } else if share >= 0.33 {
            PlayerSquadStatus::FirstTeamRegular
        } else if share >= 0.15 {
            PlayerSquadStatus::FirstTeamSquadRotation
        } else {
            PlayerSquadStatus::MainBackupPlayer
        };
        Some(ceiling)
    }
}

#[cfg(test)]
mod development_squad_tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerClubContract, PlayerCollection, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills, StaffCollection, Team, TeamBuilder,
        TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::NaiveTime;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()
    }

    fn keeper(id: u32, birth_year: i32, ca: u8, status: PlayerSquadStatus) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.current_ability = ca;
        let mut contract =
            PlayerClubContract::new(20_000, NaiveDate::from_ymd_opt(2030, 6, 30).unwrap());
        contract.squad_status = status;
        let mut p = PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("D".into(), format!("P{id}")))
            .birth_date(NaiveDate::from_ymd_opt(birth_year, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Goalkeeper,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        p.contract = Some(contract);
        p
    }

    fn squad_of(team_type: TeamType, players: Vec<Player>) -> Team {
        TeamBuilder::new()
            .id(1)
            .league_id(None)
            .club_id(1)
            .name("Dev".into())
            .slug("dev".into())
            .team_type(team_type)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(100, 100, 200))
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            ))
            .build()
            .unwrap()
    }

    fn status_of(team: &Team, id: u32) -> PlayerSquadStatus {
        team.players
            .players
            .iter()
            .find(|p| p.id == id)
            .unwrap()
            .contract
            .as_ref()
            .unwrap()
            .squad_status
            .clone()
    }

    /// The Pinsoglio case: a veteran whose CA tops a development squad's
    /// position group must NOT be crowned its "Key Player" — the label
    /// collapses to backup, silently.
    #[test]
    fn veteran_label_collapses_to_backup_on_development_squad() {
        let mut team = squad_of(
            TeamType::U20,
            vec![
                keeper(1, 1990, 140, PlayerSquadStatus::KeyPlayer),
                keeper(2, 2007, 90, PlayerSquadStatus::NotYetSet),
                keeper(3, 2008, 80, PlayerSquadStatus::NotYetSet),
            ],
        );
        SquadStatusUpdater::apply(&mut team, today());

        assert_eq!(status_of(&team, 1), PlayerSquadStatus::MainBackupPlayer);
        let vet = team.players.players.iter().find(|p| p.id == 1).unwrap();
        assert!(
            vet.happiness
                .recent_events
                .iter()
                .all(|e| e.event_type != HappinessEventType::SquadStatusChange),
            "administrative relabel must not narrate a demotion"
        );
    }

    #[test]
    fn backup_stays_backup_on_reserve_even_as_best_in_group() {
        let mut team = squad_of(
            TeamType::Reserve,
            vec![
                keeper(1, 1989, 120, PlayerSquadStatus::MainBackupPlayer),
                keeper(2, 2006, 85, PlayerSquadStatus::NotYetSet),
            ],
        );
        SquadStatusUpdater::apply(&mut team, today());

        assert_eq!(
            status_of(&team, 1),
            PlayerSquadStatus::MainBackupPlayer,
            "a parked backup must not become Key Player of the reserves"
        );
        // A 20-year-old senior gets the same backup label, for the same
        // reason the veteran above does: this squad does not own first-team
        // standing, and "backup" is what standing on it means.
        //
        // He used to keep `NotYetSet` indefinitely — no first-team label was
        // invented for him, but no label was ever settled either, and that
        // turned out to be a trap rather than neutrality. "Not yet
        // evaluated" reads as protected everywhere it is consulted: the
        // renewal gate's no-football-case test only considers backups and
        // unwanted players, so his deal was renewed for as long as he
        // stayed unlabelled. A senior parked on a reserve squad could
        // therefore never be moved on by any path at all.
        assert_eq!(status_of(&team, 2), PlayerSquadStatus::MainBackupPlayer);
    }

    /// Prospect labels are club-level assessments, not team-role claims —
    /// they stay live on development squads.
    #[test]
    fn youngster_still_gets_prospect_label_on_development_squad() {
        let mut team = squad_of(
            TeamType::Reserve,
            vec![
                keeper(1, 2009, 110, PlayerSquadStatus::NotYetSet),
                keeper(2, 2009, 70, PlayerSquadStatus::NotYetSet),
            ],
        );
        SquadStatusUpdater::apply(&mut team, today());

        assert_eq!(
            status_of(&team, 1),
            PlayerSquadStatus::HotProspectForTheFuture
        );
        assert_eq!(status_of(&team, 2), PlayerSquadStatus::DecentYoungster);
    }

    #[test]
    fn unexpired_role_promise_floors_the_label_on_development_squad() {
        let mut vet = keeper(1, 1994, 130, PlayerSquadStatus::FirstTeamRegular);
        vet.contract.as_mut().unwrap().promised_squad_status = Some((
            PlayerSquadStatus::FirstTeamRegular,
            today() + Duration::days(100),
        ));
        let mut team = squad_of(TeamType::Reserve, vec![vet]);
        SquadStatusUpdater::apply(&mut team, today());

        assert_eq!(
            status_of(&team, 1),
            PlayerSquadStatus::FirstTeamRegular,
            "an unexpired signing promise must survive the reserve relabel so the breach stays visible"
        );
    }

    #[test]
    fn expired_promise_clears_and_label_collapses_on_development_squad() {
        let mut vet = keeper(1, 1994, 130, PlayerSquadStatus::KeyPlayer);
        vet.contract.as_mut().unwrap().promised_squad_status =
            Some((PlayerSquadStatus::KeyPlayer, today() - Duration::days(1)));
        let mut team = squad_of(TeamType::Reserve, vec![vet]);
        SquadStatusUpdater::apply(&mut team, today());

        let c = team.players.players[0].contract.as_ref().unwrap();
        assert_eq!(c.squad_status, PlayerSquadStatus::MainBackupPlayer);
        assert!(c.promised_squad_status.is_none());
    }
}

#[cfg(test)]
mod club_level_tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerClubContract, PlayerCollection, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills, PlayerStatistics,
        PlayerStatisticsHistoryItem, StaffCollection, Team, TeamBuilder, TeamReputation, TeamType,
        TrainingSchedule,
    };
    use chrono::NaiveTime;

    /// October: the season is two months old and the last completed one
    /// is unambiguous.
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
    }

    fn forward(id: u32, birth_year: i32, ca: u8) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.current_ability = ca;
        let mut contract =
            PlayerClubContract::new(20_000, NaiveDate::from_ymd_opt(2030, 6, 30).unwrap());
        contract.squad_status = PlayerSquadStatus::NotYetSet;
        let mut p = PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("F".into(), format!("P{id}")))
            .birth_date(NaiveDate::from_ymd_opt(birth_year, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Striker,
                    level: 20,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        p.contract = Some(contract);
        p
    }

    fn main_squad(reputation: u16, players: Vec<Player>) -> Team {
        TeamBuilder::new()
            .id(1)
            .league_id(None)
            .club_id(1)
            .name("Main".into())
            .slug("main".into())
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(reputation, reputation, reputation))
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            ))
            .build()
            .unwrap()
    }

    fn status_of(team: &Team, id: u32) -> PlayerSquadStatus {
        team.players
            .players
            .iter()
            .find(|p| p.id == id)
            .unwrap()
            .contract
            .as_ref()
            .unwrap()
            .squad_status
            .clone()
    }

    fn last_season(played: u16) -> PlayerStatisticsHistoryItem {
        PlayerStatisticsHistoryItem {
            season: Season::new(2025),
            team_name: "Main".into(),
            team_slug: "main".into(),
            team_reputation: 9_000,
            league_name: "L".into(),
            league_slug: "l".into(),
            is_loan: false,
            transfer_fee: None,
            statistics: PlayerStatistics {
                played,
                ..Default::default()
            },
            seq_id: 0,
        }
    }

    #[test]
    fn a_giant_labels_its_second_forward_by_its_own_level() {
        // The same two men at a giant and at a modest club: rank crowns the
        // second body a regular in both; only the giant's level says he is
        // rotation depth.
        let mut giant = main_squad(9_000, vec![forward(1, 1998, 158), forward(2, 1997, 128)]);
        SquadStatusUpdater::apply(&mut giant, today());
        assert_eq!(status_of(&giant, 1), PlayerSquadStatus::KeyPlayer);
        assert_eq!(
            status_of(&giant, 2),
            PlayerSquadStatus::FirstTeamSquadRotation,
            "a 128 is not a first-team regular at a club that recruits at 147"
        );

        let mut modest = main_squad(3_000, vec![forward(1, 1998, 158), forward(2, 1997, 128)]);
        SquadStatusUpdater::apply(&mut modest, today());
        assert_eq!(status_of(&modest, 2), PlayerSquadStatus::FirstTeamRegular);
    }

    /// The Sobolev case: ninety starts in his first three seasons, eight in
    /// the next three. The lifetime since-join share still cleared the
    /// regular bar, so the label said "First Team Regular" about a man the
    /// coach had stopped picking years before. The window reads what he is
    /// getting now.
    #[test]
    fn a_long_serving_forward_the_coach_stopped_picking_is_a_backup() {
        let joined = today() - Duration::days(6 * 365);
        let mut stalwart = forward(2, 1997, 150);
        stalwart.last_transfer_date = Some(joined);
        stalwart.happiness.eligible_official_matches_since_join = 270;
        stalwart.happiness.starts_since_join = 92;
        stalwart.statistics.played = 1;
        stalwart.statistics_history.items.push(last_season(3));

        let mut ever_present = forward(1, 1999, 158);
        ever_present.last_transfer_date = Some(joined);
        ever_present.happiness.eligible_official_matches_since_join = 270;
        ever_present.happiness.starts_since_join = 240;
        ever_present.statistics.played = 8;
        ever_present.statistics_history.items.push(last_season(40));

        let mut team = main_squad(9_000, vec![ever_present, stalwart]);
        SquadStatusUpdater::apply(&mut team, today());

        assert_eq!(status_of(&team, 1), PlayerSquadStatus::KeyPlayer);
        assert_eq!(
            status_of(&team, 2),
            PlayerSquadStatus::MainBackupPlayer,
            "four games in forty-eight is a backup, whatever the career share says"
        );
    }

    #[test]
    fn a_recent_signing_is_still_judged_on_his_since_join_record() {
        // Joined last winter: eight months at the club, twenty eligible
        // matches, started sixteen — a regular on the only record that
        // exists for him, whatever last season's history at another club
        // would say.
        let mut newcomer = forward(2, 1997, 150);
        newcomer.last_transfer_date = Some(today() - Duration::days(240));
        newcomer.happiness.eligible_official_matches_since_join = 20;
        newcomer.happiness.starts_since_join = 16;
        newcomer.statistics.played = 8;

        let mut star = forward(1, 1999, 158);
        star.statistics.played = 8;
        star.statistics_history.items.push(last_season(40));

        let mut team = main_squad(9_000, vec![star, newcomer]);
        SquadStatusUpdater::apply(&mut team, today());
        assert_eq!(status_of(&team, 2), PlayerSquadStatus::FirstTeamRegular);
    }
}

#[cfg(test)]
mod returnee_verdict_tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills, PlayerStatCompetitionKind, PlayerStatLedgerEntry, PlayerStatistics,
    };
    use chrono::NaiveDate;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
    }

    fn returnee(days_since_return: i64) -> Player {
        let mut p = PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("T".into(), "1".into()))
            .birth_date(NaiveDate::from_ymd_opt(2006, 3, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Striker,
                    level: 20,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap();
        p.last_transfer_date = Some(today() - Duration::days(days_since_return));
        p
    }

    fn loan_season(year: u16, starts: u16, reputation: u16) -> PlayerStatLedgerEntry {
        PlayerStatLedgerEntry {
            seq_id: 0,
            season_start_year: year,
            team_slug: "borrower".into(),
            team_name: "B".into(),
            team_reputation: reputation,
            league_slug: "l".into(),
            league_name: "L".into(),
            competition_kind: PlayerStatCompetitionKind::League,
            competition_slug: String::new(),
            is_loan: true,
            transfer_fee: None,
            coverage_days: None,
            spell_end: None,
            statistics: PlayerStatistics {
                played: starts,
                ..Default::default()
            },
        }
    }

    #[test]
    fn full_loan_season_graduates_the_label_to_rotation() {
        let mut p = returnee(20);
        p.statistics_history
            .season_ledger
            .push(loan_season(2025, 24, 5_000));
        assert_eq!(
            SquadStatusUpdater::returnee_record_floor(&p, 5_000, today()),
            Some(PlayerSquadStatus::FirstTeamSquadRotation),
            "a full same-level loan season graduates the prospect label"
        );
    }

    #[test]
    fn two_proven_loan_seasons_graduate_to_regular() {
        let mut p = returnee(20);
        p.statistics_history
            .season_ledger
            .push(loan_season(2024, 28, 5_000));
        p.statistics_history
            .season_ledger
            .push(loan_season(2025, 30, 5_000));
        assert_eq!(
            SquadStatusUpdater::returnee_record_floor(&p, 5_000, today()),
            Some(PlayerSquadStatus::FirstTeamRegular),
            "two near-ever-present loan seasons earn a regular's role"
        );
    }

    #[test]
    fn stale_return_gets_no_verdict() {
        let mut p = returnee(100);
        p.statistics_history
            .season_ledger
            .push(loan_season(2025, 24, 5_000));
        assert_eq!(
            SquadStatusUpdater::returnee_record_floor(&p, 5_000, today()),
            None,
            "the verdict belongs to the arrival window"
        );
    }

    #[test]
    fn record_earned_far_below_gets_no_verdict_at_a_giant() {
        let mut p = returnee(20);
        p.statistics_history
            .season_ledger
            .push(loan_season(2025, 24, 3_000));
        assert_eq!(
            SquadStatusUpdater::returnee_record_floor(&p, 9_000, today()),
            None,
            "a lower-league record does not graduate a label at a giant"
        );
    }
}
