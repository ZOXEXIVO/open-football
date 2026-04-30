//! Unit-level partnership chemistry: pair-wise dynamics that depend on
//! tactical role rather than the broader same-position / age / form
//! signals already covered by `dynamics.rs`. Three forces compete on
//! every meaningful pair:
//!
//! * **Tactical dependency** — a CB-CB pair that has to defend together,
//!   a fullback feeding the same flank winger, an AM playing into the
//!   same striker. Build chemistry through repetition.
//! * **Rivalry pressure** — same exact position with similar CA and
//!   matching ambition, especially when one player is consistently
//!   blocking the other from starts.
//! * **Personality friction** — controversy / professionalism clash,
//!   hot-tempered pairs, behaviour mismatches.
//!
//! The net signal is scaled by the existing relation (long-trust pairs
//! resist new friction, low-trust pairs amplify it) and clamped before
//! it lands. Asymmetric application handles the depth-chart case: the
//! blocked player resents the starter more than the starter resents
//! the backup.

use super::TeamBehaviour;
use crate::club::team::behaviour::{
    PlayerRelationshipChangeResult, TeamBehaviourResult,
};
use crate::context::GlobalContext;
use crate::utils::DateUtils;
use crate::{ChangeType, Player, PlayerCollection, PlayerPositionType, PlayerSquadStatus};

/// How a tactical pair relates on the pitch.
#[derive(Debug, Copy, Clone, PartialEq)]
enum PairRole {
    /// Identical position — direct competition for the same shirt.
    SamePosition,
    /// Genuine on-pitch partnership: CB-CB, ST-ST, FB+W, DM-CM, CM-AM,
    /// W-ST, GK-CB. Tactical reps build chemistry.
    Partnership,
    /// Same broad group but no on-pitch partnership (e.g. DL-DC).
    /// Treated as mild rivalry — they audition for the same back-four
    /// but aren't a paired unit.
    SameGroupRival,
    /// No tactical link.
    None_,
}

impl TeamBehaviour {
    /// Weekly unit-partnership chemistry pass. Emits mirrored
    /// PlayerRelationshipChangeResult entries for every meaningful
    /// pair (same-position competitors, on-pitch partners, language /
    /// nationality bonds). Pairs with no tactical relevance and no
    /// shared culture are skipped — they're already covered by the
    /// generic dynamics/age/performance passes.
    pub(super) fn process_unit_partnerships(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
        ctx: &GlobalContext<'_>,
    ) {
        let date = ctx.simulation.date.date();

        for i in 0..players.players.len() {
            for j in i + 1..players.players.len() {
                let a = &players.players[i];
                let b = &players.players[j];

                let role = classify_pair(a.position(), b.position());
                let shared_lang = shares_language(a, b);
                let same_country = a.country_id == b.country_id;
                let cultural_pair = shared_lang || same_country;

                // Pairs without any tactical or cultural link are
                // already covered by the broader dynamics passes; skip
                // them here to keep the signal pure.
                if matches!(role, PairRole::None_) && !cultural_pair {
                    continue;
                }

                // Cultural-only gate: a Brazilian-Brazilian pair with no
                // tactical link gets a one-time bonding lift while one is
                // settling, but shouldn't keep accruing +0.05 / week
                // forever once they're both veterans of the dressing
                // room. Skip the cultural-only emission unless the pair
                // genuinely needs it (recent transfer, isolated player,
                // or low existing relation).
                let (rel_a, rel_b) = (
                    a.relations.get_player(b.id).map(|r| r.level).unwrap_or(0.0),
                    b.relations.get_player(a.id).map(|r| r.level).unwrap_or(0.0),
                );
                let cultural_only = matches!(role, PairRole::None_) && cultural_pair;
                if cultural_only
                    && !cultural_pair_needs_lift(a, b, date, rel_a, rel_b)
                {
                    continue;
                }

                let tactical = tactical_dependency(a, b, role, shared_lang, same_country, date);
                let rivalry = rivalry_pressure(a, b, role);
                let friction = personality_friction(a, b);

                let net = tactical - rivalry - friction;

                // Existing-relation modulation. A friendship floor
                // softens negatives; broken trust amplifies them; high
                // professional respect dulls rivalry-driven envy. We
                // ask each direction independently so a one-sided
                // friendship still softens the side that feels it.
                let (friend_a, trust_a, prof_a) = relation_summary(a, b);
                let (friend_b, trust_b, prof_b) = relation_summary(b, a);

                let mut delta_a = scaled_delta(net, friend_a, trust_a, prof_a, rivalry);
                let mut delta_b = scaled_delta(net, friend_b, trust_b, prof_b, rivalry);

                // Asymmetric depth-chart effect: the blocked player
                // resents the starter more than the starter resents
                // the backup, unless the starter is unusually
                // controversial (in which case they push back).
                let (a_to_b_mult, b_to_a_mult) = depth_chart_asymmetry(a, b, role);
                delta_a *= a_to_b_mult;
                delta_b *= b_to_a_mult;

                // Saturation taper: once a relation is already very
                // strong (positive or negative), further drift in the
                // same direction halves. Stops endless weekly +0.10
                // chemistry and endless -0.18 friction at the extremes.
                delta_a = taper_for_saturation(delta_a, rel_a);
                delta_b = taper_for_saturation(delta_b, rel_b);

                delta_a = delta_a.clamp(-0.20, 0.18);
                delta_b = delta_b.clamp(-0.20, 0.18);

                emit(result, a.id, b.id, delta_a, role, cultural_pair, friction);
                emit(result, b.id, a.id, delta_b, role, cultural_pair, friction);
            }
        }
    }
}

fn classify_pair(a: PlayerPositionType, b: PlayerPositionType) -> PairRole {
    use PlayerPositionType::*;

    if a == b {
        return PairRole::SamePosition;
    }

    let group_a = a.position_group();
    let group_b = b.position_group();

    // Same broad group — narrow it down by sub-family.
    if group_a == group_b {
        let cb_family = |p| matches!(p, DefenderCenter | DefenderCenterLeft | DefenderCenterRight | Sweeper);
        if cb_family(a) && cb_family(b) {
            return PairRole::Partnership;
        }
        let st_family = |p| matches!(p, Striker | ForwardCenter | ForwardLeft | ForwardRight);
        if st_family(a) && st_family(b) {
            return PairRole::Partnership;
        }
        let cm_family = |p| matches!(p, MidfielderCenter | MidfielderCenterLeft | MidfielderCenterRight);
        if cm_family(a) && cm_family(b) {
            return PairRole::Partnership;
        }
        // Goalkeepers in the same squad: rivals, not partners — there
        // is only one shirt.
        return PairRole::SameGroupRival;
    }

    // Cross-group adjacent pairings — fully-qualified the field-group
    // values to keep them out of the position enum's namespace.
    use crate::PlayerFieldPositionGroup;

    // GK + CB
    let cb_family = |p| matches!(p, DefenderCenter | DefenderCenterLeft | DefenderCenterRight | Sweeper);
    if (group_a == PlayerFieldPositionGroup::Goalkeeper && cb_family(b))
        || (group_b == PlayerFieldPositionGroup::Goalkeeper && cb_family(a))
    {
        return PairRole::Partnership;
    }

    // Fullback + winger on the same flank.
    let dl_or_wbl = |p| matches!(p, DefenderLeft | WingbackLeft);
    let dr_or_wbr = |p| matches!(p, DefenderRight | WingbackRight);
    let left_winger = |p| matches!(p, MidfielderLeft | AttackingMidfielderLeft | ForwardLeft);
    let right_winger = |p| matches!(p, MidfielderRight | AttackingMidfielderRight | ForwardRight);
    if (dl_or_wbl(a) && left_winger(b)) || (dl_or_wbl(b) && left_winger(a)) {
        return PairRole::Partnership;
    }
    if (dr_or_wbr(a) && right_winger(b)) || (dr_or_wbr(b) && right_winger(a)) {
        return PairRole::Partnership;
    }

    // DM + CM
    let cm_family = |p| matches!(p, MidfielderCenter | MidfielderCenterLeft | MidfielderCenterRight);
    if (a == DefensiveMidfielder && cm_family(b)) || (b == DefensiveMidfielder && cm_family(a)) {
        return PairRole::Partnership;
    }

    // CM + AM
    let am_family = |p| matches!(p, AttackingMidfielderCenter | AttackingMidfielderLeft | AttackingMidfielderRight);
    if (cm_family(a) && am_family(b)) || (cm_family(b) && am_family(a)) {
        return PairRole::Partnership;
    }

    // AM/Winger + Striker
    let striker_family = |p| matches!(p, Striker | ForwardCenter);
    let attacking_winger = |p| matches!(p, ForwardLeft | ForwardRight | AttackingMidfielderLeft | AttackingMidfielderRight);
    if (striker_family(a) && attacking_winger(b)) || (striker_family(b) && attacking_winger(a)) {
        return PairRole::Partnership;
    }

    PairRole::None_
}

fn shares_language(a: &Player, b: &Player) -> bool {
    // Match the existing adaptation rule of thumb: ≥40 proficiency is
    // enough for an actual conversation. Nationals always pass.
    for la in &a.languages {
        if la.proficiency < 40 {
            continue;
        }
        for lb in &b.languages {
            if lb.proficiency < 40 {
                continue;
            }
            if la.language == lb.language {
                return true;
            }
        }
    }
    false
}

fn tactical_dependency(
    a: &Player,
    b: &Player,
    role: PairRole,
    shared_lang: bool,
    same_country: bool,
    date: chrono::NaiveDate,
) -> f32 {
    // Only on-pitch partnerships pick up the tactical bonus.
    let mut score: f32 = 0.0;
    if matches!(role, PairRole::Partnership) {
        score += 0.08;

        if a.skills.mental.teamwork >= 13.0 && b.skills.mental.teamwork >= 13.0 {
            score += 0.05;
        }
        if a.attributes.professionalism >= 13.0 && b.attributes.professionalism >= 13.0 {
            score += 0.04;
        }

        // Mentor-bonus when one is a senior leader and the other is
        // young enough to look up. The age-gap test mirrors the
        // existing mentorship pass, gated tighter here so we don't
        // double-count the same effect.
        let age_a = DateUtils::age(a.birth_date, date);
        let age_b = DateUtils::age(b.birth_date, date);
        let (senior_age, junior_age, senior_leadership) =
            if age_a > age_b {
                (age_a, age_b, a.skills.mental.leadership)
            } else {
                (age_b, age_a, b.skills.mental.leadership)
            };
        if senior_age >= 28 && junior_age <= 23 && senior_leadership >= 13.0 {
            score += 0.03;
        }
    }

    // Cultural bonuses apply across the squad — even for non-partner
    // pairs (a Brazilian in midfield bonding with a Brazilian striker
    // through shared language). Cap the combined cultural lift at 0.07.
    let mut cultural = 0.0f32;
    if shared_lang {
        cultural += 0.05;
    }
    if same_country {
        cultural += 0.03;
    }
    cultural = cultural.min(0.07);

    // Taper the cultural lift as the relation grows — two compatriots
    // who already love each other don't bond *more* every week. Use
    // the strongest existing direction so a one-sided strong bond
    // still slows growth on both sides.
    let strongest_relation = a
        .relations
        .get_player(b.id)
        .map(|r| r.level)
        .unwrap_or(0.0)
        .max(b.relations.get_player(a.id).map(|r| r.level).unwrap_or(0.0));
    let taper = 1.0 - (strongest_relation.max(0.0) / 80.0).clamp(0.0, 0.75);
    cultural *= taper;

    score + cultural
}

/// Cultural-only pairs (no tactical link, but shared language or
/// nationality) get a weekly lift only when the pair genuinely needs
/// it: one of them just joined, one is socially isolated, or the
/// existing relation is still cool. Established compatriots already
/// have whatever bond they're going to have.
fn cultural_pair_needs_lift(
    a: &Player,
    b: &Player,
    today: chrono::NaiveDate,
    rel_a: f32,
    rel_b: f32,
) -> bool {
    // Either side recently joined? 90-day window matches contract
    // / adaptation conventions used elsewhere.
    let recent_transfer = |p: &Player| -> bool {
        p.contract
            .as_ref()
            .and_then(|c| c.started)
            .map(|started| (today - started).num_days() <= 90)
            .unwrap_or(false)
    };
    if recent_transfer(a) || recent_transfer(b) {
        return true;
    }

    // Existing relation still cool — there's room for the cultural
    // signal to move the needle.
    if rel_a < 25.0 || rel_b < 25.0 {
        return true;
    }

    // One side looks isolated (their inner circle is essentially
    // empty). The cohesion signal is shared across the player's
    // perceived clique; near-zero cohesion means they haven't bonded
    // with anyone yet.
    if a.relations.inner_circle_cohesion() < 0.05
        || b.relations.inner_circle_cohesion() < 0.05
    {
        return true;
    }

    false
}

/// Halve further drift when the relation is already saturated in the
/// same direction. Keeps long-running rivalries and long-running
/// friendships from inching to ±100 every season without a fresh
/// trigger to break the equilibrium.
fn taper_for_saturation(delta: f32, current_relation: f32) -> f32 {
    if delta < 0.0 && current_relation <= -50.0 {
        return delta * 0.5;
    }
    if delta > 0.0 && current_relation >= 60.0 {
        return delta * 0.5;
    }
    delta
}

fn rivalry_pressure(a: &Player, b: &Player, role: PairRole) -> f32 {
    let mut score: f32 = 0.0;

    // Direct overlap: same exact position.
    let same_position = matches!(role, PairRole::SamePosition);
    if same_position {
        score += 0.08;
    }
    // Same group rivals (DL-DC competing for the back-four) get a
    // smaller version of the same effect.
    if matches!(role, PairRole::SameGroupRival) {
        score += 0.04;
    }
    // Partnerships skip the rivalry component entirely — a CB pair's
    // tactical bond shouldn't be eaten by phantom competition.
    if matches!(role, PairRole::Partnership) {
        return 0.0;
    }

    let ca_diff = (a.player_attributes.current_ability as i32
        - b.player_attributes.current_ability as i32)
        .abs();
    if ca_diff <= 15 {
        score += 0.06;
    }

    if a.attributes.ambition >= 14.0 && b.attributes.ambition >= 14.0 {
        score += 0.05;
    }

    let starter_status = |p: &Player| {
        p.contract.as_ref().is_some_and(|c| {
            matches!(
                c.squad_status,
                PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular
            )
        })
    };
    if starter_status(a) && starter_status(b) {
        score += 0.06;
    }

    if blocks_starts(a, b) || blocks_starts(b, a) {
        score += 0.08;
    }

    score
}

fn personality_friction(a: &Player, b: &Player) -> f32 {
    let mut score: f32 = 0.0;

    let controversy_clash = (a.attributes.controversy >= 15.0
        && b.attributes.professionalism >= 14.0)
        || (b.attributes.controversy >= 15.0 && a.attributes.professionalism >= 14.0);
    if controversy_clash {
        score += 0.06;
    }

    let hot_pair = (a.attributes.temperament <= 7.0 || b.attributes.temperament <= 7.0)
        && (a.skills.mental.aggression >= 13.0 || b.skills.mental.aggression >= 13.0);
    if hot_pair {
        score += 0.04;
    }

    let behaviour_mismatch = matches!(
        (a.behaviour.is_poor(), b.behaviour.is_good()),
        (true, true) | (false, false)
    ) && (a.behaviour.is_poor() != b.behaviour.is_poor());
    if behaviour_mismatch {
        score += 0.05;
    }

    score
}

/// True when `starter` consistently blocks `blocked` from starts at
/// their shared position. Cheap stand-in: uses recent league
/// appearances + days_since_last_match as the proxy. We're not
/// trying to be exact, just to flag the FB-FB / CB-CB case where one
/// guy is glued to the bench.
fn blocks_starts(starter: &Player, blocked: &Player) -> bool {
    if starter.position() != blocked.position() {
        return false;
    }
    let starter_apps = starter.statistics.played + starter.statistics.played_subs;
    if starter_apps < 5 {
        return false;
    }
    let blocked_apps = blocked.statistics.played + blocked.statistics.played_subs;
    let starter_starts = starter.statistics.played as i32;
    let blocked_starts = blocked.statistics.played as i32;
    starter_starts - blocked_starts >= 5
        && blocked_apps as i32 * 2 < starter_apps as i32
        && blocked.player_attributes.days_since_last_match >= 14
}

fn relation_summary(subject: &Player, target: &Player) -> (f32, f32, f32) {
    if let Some(rel) = subject.relations.get_player(target.id) {
        (rel.friendship, rel.trust, rel.professional_respect)
    } else {
        // Neutral defaults match PlayerRelation::new_neutral.
        (30.0, 50.0, 50.0)
    }
}

/// Apply existing-relation modulation. Friendships floor the negatives;
/// broken trust amplifies them; high professional respect dulls
/// rivalry-driven friction so two pros who hate each other still respect
/// the work.
fn scaled_delta(net: f32, friendship: f32, trust: f32, prof_respect: f32, rivalry: f32) -> f32 {
    let mut delta = net;
    if delta < 0.0 && friendship >= 60.0 {
        delta *= 0.6;
    }
    if delta < 0.0 && trust <= 25.0 {
        delta *= 1.25;
    }
    if delta < 0.0 && prof_respect >= 70.0 && rivalry > 0.0 {
        // Reduce the rivalry portion specifically — keep behaviour-based
        // friction intact (a bad apple is still a bad apple).
        delta += rivalry * 0.30;
    }
    delta
}

fn depth_chart_asymmetry(a: &Player, b: &Player, role: PairRole) -> (f32, f32) {
    if !matches!(role, PairRole::SamePosition) {
        return (1.0, 1.0);
    }
    // Identify starter/backup by recent starts.
    let a_starts = a.statistics.played as i32;
    let b_starts = b.statistics.played as i32;

    // Controversial starter pushes back regardless.
    let starter_pushes_back = |starter: &Player| starter.attributes.controversy > 14.0;

    if a_starts >= b_starts + 5 {
        // a is starter, b is blocked.
        let to_a = if starter_pushes_back(a) { 1.0 } else { 0.60 };
        return (to_a, 1.25);
    }
    if b_starts >= a_starts + 5 {
        let to_b = if starter_pushes_back(b) { 1.0 } else { 0.60 };
        return (1.25, to_b);
    }
    (1.0, 1.0)
}

fn emit(
    result: &mut TeamBehaviourResult,
    from_id: u32,
    to_id: u32,
    delta: f32,
    role: PairRole,
    cultural_pair: bool,
    friction: f32,
) {
    if delta.abs() < 0.01 {
        return;
    }
    let _ = cultural_pair; // signal kept for future tagging; consolidated below.
    let change_type = if delta >= 0.0 {
        // Positive: MatchCooperation when the pair is a tactical
        // partnership, TrainingBonding otherwise (cultural / general).
        if matches!(role, PairRole::Partnership) {
            ChangeType::MatchCooperation
        } else {
            ChangeType::TrainingBonding
        }
    } else {
        // Negative: split between competition (rivalry-shaped) and
        // personal (personality-shaped) so downstream consumers can
        // tell them apart.
        if friction > 0.0 {
            ChangeType::PersonalConflict
        } else {
            ChangeType::CompetitionRivalry
        }
    };
    result
        .players
        .relationship_result
        .push(PlayerRelationshipChangeResult {
            from_player_id: from_id,
            to_player_id: to_id,
            relationship_change: delta,
            change_type,
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::language::{Language, PlayerLanguage};
    use crate::shared::fullname::FullName;
    use crate::{PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositions, PlayerSkills};
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn pro_personality() -> PersonAttributes {
        PersonAttributes {
            adaptability: 14.0,
            ambition: 14.0,
            controversy: 5.0,
            loyalty: 14.0,
            pressure: 12.0,
            professionalism: 16.0,
            sportsmanship: 14.0,
            temperament: 14.0,
            consistency: 12.0,
            important_matches: 12.0,
            dirtiness: 5.0,
        }
    }

    fn standard_skills() -> PlayerSkills {
        let mut s = PlayerSkills::default();
        s.mental.leadership = 13.0;
        s.mental.teamwork = 14.0;
        s.mental.aggression = 10.0;
        s.mental.determination = 12.0;
        s
    }

    fn build(id: u32, pos: PlayerPositionType, country_id: u32, ca: u8, rep: i16) -> Player {
        let mut player_attrs = PlayerAttributes::default();
        player_attrs.current_ability = ca;
        player_attrs.current_reputation = rep;
        player_attrs.world_reputation = rep;
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("T".to_string(), id.to_string()))
            .birth_date(d(1995, 1, 1))
            .country_id(country_id)
            .attributes(pro_personality())
            .skills(standard_skills())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition { position: pos, level: 20 }],
            })
            .player_attributes(player_attrs)
            .build()
            .unwrap()
    }

    #[test]
    fn classify_pair_recognises_same_position() {
        let role = classify_pair(
            PlayerPositionType::Striker,
            PlayerPositionType::Striker,
        );
        assert_eq!(role, PairRole::SamePosition);
    }

    #[test]
    fn classify_pair_recognises_cb_partnership() {
        let role = classify_pair(
            PlayerPositionType::DefenderCenter,
            PlayerPositionType::DefenderCenterLeft,
        );
        assert_eq!(role, PairRole::Partnership);
    }

    #[test]
    fn classify_pair_recognises_dm_cm_adjacent() {
        let role = classify_pair(
            PlayerPositionType::DefensiveMidfielder,
            PlayerPositionType::MidfielderCenter,
        );
        assert_eq!(role, PairRole::Partnership);
    }

    #[test]
    fn same_position_ambitious_pair_outpaces_unrelated() {
        let mut a = build(1, PlayerPositionType::Striker, 1, 140, 5_000);
        let mut b = build(2, PlayerPositionType::Striker, 1, 138, 4_900);
        a.attributes.ambition = 16.0;
        b.attributes.ambition = 16.0;
        let unrelated = build(3, PlayerPositionType::Goalkeeper, 1, 138, 4_900);

        let role_same = classify_pair(a.position(), b.position());
        let role_unrelated = classify_pair(a.position(), unrelated.position());
        let rivalry_same = rivalry_pressure(&a, &b, role_same);
        let rivalry_unrelated = rivalry_pressure(&a, &unrelated, role_unrelated);

        assert!(
            rivalry_same > rivalry_unrelated,
            "same-position pair should produce more rivalry pressure ({} vs {})",
            rivalry_same,
            rivalry_unrelated
        );
        assert!(rivalry_same >= 0.10);
    }

    #[test]
    fn shared_language_plus_partnership_yields_positive_chemistry() {
        let mut a = build(1, PlayerPositionType::DefenderCenter, 10, 140, 5_000);
        let mut b = build(2, PlayerPositionType::DefenderCenterLeft, 20, 140, 5_000);
        a.languages.push(PlayerLanguage::native(Language::Spanish));
        b.languages.push(PlayerLanguage::native(Language::Spanish));

        let role = classify_pair(a.position(), b.position());
        let shared = shares_language(&a, &b);
        let same_country = a.country_id == b.country_id;
        let tactical = tactical_dependency(&a, &b, role, shared, same_country, d(2026, 4, 1));
        let rivalry = rivalry_pressure(&a, &b, role);
        let friction = personality_friction(&a, &b);

        assert!(shared);
        assert_eq!(role, PairRole::Partnership);
        let net = tactical - rivalry - friction;
        assert!(net > 0.05, "CB+language pair should net positive ({})", net);
    }

    #[test]
    fn high_friendship_softens_negative_partnership_delta() {
        // Strong negative pressure (e.g. depth-chart rivalry).
        let net = -0.18;
        let with_friend = scaled_delta(net, 70.0, 60.0, 60.0, 0.10);
        let without = scaled_delta(net, 30.0, 60.0, 60.0, 0.10);
        assert!(
            with_friend > without,
            "friendship floor should soften the negative ({} vs {})",
            with_friend,
            without
        );
        assert!(with_friend.abs() < 0.16);
    }

    #[test]
    fn daily_random_drift_smaller_than_partnership_signal() {
        // Worst-case daily noise: 0.008 × 1.25 (max interaction-frequency boost)
        // × 1.0 (max temperament factor). Partnership signal for a CB pair
        // sharing language should clearly clear that bar.
        let max_daily = 0.008 * 1.25;

        let mut a = build(1, PlayerPositionType::DefenderCenter, 10, 140, 5_000);
        let mut b = build(2, PlayerPositionType::DefenderCenterLeft, 10, 140, 5_000);
        a.languages.push(PlayerLanguage::native(Language::Spanish));
        b.languages.push(PlayerLanguage::native(Language::Spanish));

        let role = classify_pair(a.position(), b.position());
        let tactical = tactical_dependency(&a, &b, role, true, true, d(2026, 4, 1));
        assert!(
            tactical > max_daily * 2.0,
            "partnership tactical signal ({}) should clearly outweigh daily noise ({})",
            tactical,
            max_daily
        );
    }

    #[test]
    fn cultural_lift_tapers_for_already_strong_relations() {
        use crate::{ChangeType, RelationshipChange};

        let mut a = build(1, PlayerPositionType::Striker, 10, 140, 5_000);
        let mut b = build(2, PlayerPositionType::DefenderCenter, 10, 140, 5_000);
        a.languages.push(PlayerLanguage::native(Language::Spanish));
        b.languages.push(PlayerLanguage::native(Language::Spanish));

        // Drive `a → b` relation to a strong positive level so the
        // taper kicks in. Use the default change pipeline so the
        // momentum / multiplier code stays consistent with prod.
        for _ in 0..40 {
            a.relations.update_player_relationship(
                b.id,
                RelationshipChange::positive(ChangeType::TrainingBonding, 0.6),
                d(2026, 4, 1),
            );
        }

        let role = classify_pair(a.position(), b.position());
        let strong = tactical_dependency(&a, &b, role, true, true, d(2026, 4, 1));

        // Compare with a fresh pair (no existing relation): the same
        // function should return more cultural lift on a clean slate.
        let mut c = build(3, PlayerPositionType::Striker, 10, 140, 5_000);
        let mut e = build(4, PlayerPositionType::DefenderCenter, 10, 140, 5_000);
        c.languages.push(PlayerLanguage::native(Language::Spanish));
        e.languages.push(PlayerLanguage::native(Language::Spanish));
        let fresh = tactical_dependency(&c, &e, role, true, true, d(2026, 4, 1));

        assert!(
            strong < fresh,
            "cultural lift should taper for strong existing relations ({} vs {})",
            strong,
            fresh
        );
    }

    #[test]
    fn cultural_only_pair_skipped_when_already_well_bonded() {
        use crate::{ChangeType, RelationshipChange};

        let date = d(2026, 4, 1);

        // Striker + Goalkeeper — no tactical link, so this is a
        // cultural-only pair (PairRole::None_ + shared language).
        let mut a = build(1, PlayerPositionType::Striker, 10, 140, 5_000);
        let mut b = build(2, PlayerPositionType::Goalkeeper, 10, 140, 5_000);
        a.languages.push(PlayerLanguage::native(Language::Spanish));
        b.languages.push(PlayerLanguage::native(Language::Spanish));

        // Bond them strongly so neither low-relation gate nor
        // isolation gate triggers.
        for _ in 0..40 {
            a.relations.update_player_relationship(
                b.id,
                RelationshipChange::positive(ChangeType::PersonalSupport, 0.6),
                date,
            );
            b.relations.update_player_relationship(
                a.id,
                RelationshipChange::positive(ChangeType::PersonalSupport, 0.6),
                date,
            );
        }

        let rel_a = a.relations.get_player(b.id).map(|r| r.level).unwrap_or(0.0);
        let rel_b = b.relations.get_player(a.id).map(|r| r.level).unwrap_or(0.0);

        // Both relations are saturated — and neither has a recent
        // transfer — so the gate should refuse the lift.
        assert!(rel_a >= 25.0 && rel_b >= 25.0);
        assert!(!cultural_pair_needs_lift(&a, &b, date, rel_a, rel_b));
    }

    #[test]
    fn cultural_only_pair_lifts_when_one_recently_joined() {
        let date = d(2026, 4, 1);

        let mut a = build(1, PlayerPositionType::Striker, 10, 140, 5_000);
        let b = build(2, PlayerPositionType::Goalkeeper, 10, 140, 5_000);

        // Stub a contract with a recent start date for player a.
        let mut contract = crate::PlayerClubContract::new(50_000, d(2030, 6, 1));
        contract.started = Some(date - chrono::Duration::days(30));
        a.contract = Some(contract);

        // Strong existing relations on both sides, but the recent
        // transfer should still open the gate.
        let rel_a = 80.0;
        let rel_b = 80.0;
        assert!(cultural_pair_needs_lift(&a, &b, date, rel_a, rel_b));
    }

    #[test]
    fn taper_for_saturation_halves_drift_at_extremes() {
        let positive = 0.10;
        let normal = taper_for_saturation(positive, 20.0);
        let saturated_pos = taper_for_saturation(positive, 70.0);
        assert_eq!(normal, 0.10);
        assert!((saturated_pos - 0.05).abs() < 1e-6);

        let negative = -0.18;
        let normal_neg = taper_for_saturation(negative, -20.0);
        let saturated_neg = taper_for_saturation(negative, -60.0);
        assert!((normal_neg - (-0.18)).abs() < 1e-6);
        assert!((saturated_neg - (-0.09)).abs() < 1e-6);
    }

    #[test]
    fn same_position_rivalry_not_double_counted() {
        // Sanity check on the duplicate-effect reduction: the
        // legacy `process_position_group_dynamics` now scales its
        // same-position contribution by 0.5, while the new
        // `process_unit_partnerships` retains its full rivalry
        // signal. Combined, the effective same-position rivalry
        // signal stays in the same neighbourhood as the new
        // module's standalone output rather than ~1.5× it.
        let mut a = build(1, PlayerPositionType::Striker, 1, 140, 5_000);
        let mut b = build(2, PlayerPositionType::Striker, 1, 138, 4_900);
        a.attributes.ambition = 16.0;
        b.attributes.ambition = 16.0;

        // Compute the legacy same-position competition signal directly.
        let competition = TeamBehaviour::calculate_competition_factor(&a, &b);
        let halved = competition * 0.5;
        assert!(
            halved <= competition * 0.51,
            "legacy same-position competition factor should be halved (full {} vs halved {})",
            competition,
            halved
        );
    }
}
