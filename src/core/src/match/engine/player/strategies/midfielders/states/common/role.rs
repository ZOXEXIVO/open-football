//! **Who this midfielder is.**
//!
//! # The hole this fills
//!
//! Football's midfield is the one line where two players standing three
//! metres apart are doing completely different jobs. The holder screens
//! and recycles; the regista beside him hits the pass that beats the
//! line; the eight runs past both of them into the box. The engine had
//! none of that. `MidfielderStrategies::process` dispatches every
//! midfielder — the anchor, the deep playmaker, the box-to-box eight,
//! the number ten and the wide man — into **one identical decision
//! tree**, and nothing anywhere in `midfielders/` reads
//! `is_attacking_midfielder()` or the player's slot at all.
//!
//! `PlayerRole` exists (`club::team::tactics::instructions`) and carries
//! `DeepLyingPlaymaker`, `Regista`, `AdvancedPlaymaker`, `Mezzala`,
//! `BoxToBox`, `Anchor`, `BallWinningMidfielder`. Its own doc comment
//! says "the match engine translates them into decision weights" — and
//! the match engine has never once read the enum. It is declared in the
//! club layer and consumed nowhere.
//!
//! Measured, that shows up exactly where you would expect. Over 140
//! matches at level 14: midfielders took **56.4% of every shot in the
//! game and scored 61.4% of every goal** (target 32%) while forwards
//! managed 37.2% (target 58%), and **49.4% of every midfield shot came
//! from 16.5-22 m** — the deep midfielder was shooting from distance
//! because nothing in the engine had ever told him he was the deep
//! midfielder.
//!
//! # The model
//!
//! Not an enum switch. A role in football is a *station* plus a *set of
//! licences*, and both are continuous:
//!
//! * **station** — how far up the pitch the manager stood him, read off
//!   his own formation slot rather than a label, so a 4-2-3-1's ten and
//!   a 4-4-2's left-centre-mid differ by where they start and not by a
//!   string;
//! * **width** — how far off the middle that slot is, for the same
//!   reason;
//! * and five licences — [`creation`](Self::creation),
//!   [`carry`](Self::carry), [`shooting`](Self::shooting),
//!   [`arrival`](Self::arrival), [`tempo`](Self::tempo) — each a blend
//!   of *who he is* (attributes, priced against `MatchStandard` so the
//!   division cannot decide them) and *where he was put*.
//!
//! [`Archetype`] falls out of the licences afterwards, for diagnostics
//! and for the handful of places where a name reads better than four
//! numbers. Nothing branches on it that could not branch on the numbers.
//!
//! # Why every absolute read is shifted
//!
//! Same reason `MidfielderSkillProfile` does it: these are convex curves
//! over ability, and an unshifted read makes the licence a function of
//! the league rather than of the player. See
//! `engine::teamplay::standard::MatchStandard`.

use crate::PlayerPositionType;
use crate::r#match::StateProcessingContext;
use crate::r#match::engine::teamplay::standard::MatchStandard;
use crate::r#match::player::strategies::players::ops::midfielder_skill::MidfielderSkillProfile;

/// **Control arm for the whole role layer.**
///
/// `OF_MID_LEGACY=1` puts the midfielder's on-ball tree back exactly as
/// it was before roles existed: no strike-range gate, no role term on
/// the arriving runner's licence, no through ball, no square ball, the
/// flat `lane.openness > 0.55` carry, no head-up release and no distance
/// gate on the switch. Same binary, same seed stream, one environment
/// variable — the pattern `OF_EVASION_LEGACY` and `OF_SHOT_BAR` already
/// use, and the only honest way to price a supply change on an unseeded
/// harness.
///
/// It exists because the first attempt to price this pass read single
/// runs. `dev_match stats 140 14 14` repeats to **±0.2 goals a match**
/// (three runs of one build: 3.51 / 3.81 / 3.61), which is the same size
/// as the effects being fitted — so anything claimed from one run of
/// each arm is noise wearing a conclusion's clothes.
pub struct MidfieldPlay;

impl MidfieldPlay {
    /// True when the legacy (pre-role) behaviour is selected.
    #[inline]
    pub fn legacy() -> bool {
        use std::sync::OnceLock;
        static LEGACY: OnceLock<bool> = OnceLock::new();
        *LEGACY.get_or_init(|| {
            std::env::var("OF_MID_LEGACY")
                .map(|v| v != "0" && !v.is_empty())
                .unwrap_or(false)
        })
    }
}

/// A readable name for the licence mix, derived rather than declared.
///
/// Nothing in the engine is allowed to *gate* on this — it exists so a
/// census row says "regista" instead of printing five floats, and so the
/// handful of comments that talk about "the deep playmaker" have a
/// referent. Every actual decision reads the continuous licences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archetype {
    /// Screens the back four. Recycles, does not go.
    Anchor,
    /// Sits deep and passes through the lines from there.
    DeepPlaymaker,
    /// Runs the length of the pitch and arrives in the box.
    BoxToBox,
    /// Plays between the lines and looks for the last ball.
    AdvancedPlaymaker,
    /// Holds a touchline and delivers.
    Wide,
}

impl Archetype {
    /// For diagnostics.
    pub fn label(self) -> &'static str {
        match self {
            Archetype::Anchor => "anchor",
            Archetype::DeepPlaymaker => "deep-playmaker",
            Archetype::BoxToBox => "box-to-box",
            Archetype::AdvancedPlaymaker => "advanced-playmaker",
            Archetype::Wide => "wide",
        }
    }
}

/// What this midfielder is for.
#[derive(Debug, Clone, Copy)]
pub struct MidfieldRole {
    pub archetype: Archetype,

    /// 0..1 — how far up the pitch his own formation slot is, measured
    /// across the band a midfielder can occupy rather than across the
    /// whole pitch, so the number spreads over the midfield instead of
    /// clustering at 0.5. 0 is a holder in front of his back four, 1 is
    /// a ten on the last line.
    pub station: f32,
    /// 0..1 — how far off the middle of the pitch his slot is. 0 is the
    /// centre circle, 1 is the touchline.
    pub width: f32,

    /// Licence to look for the ball that beats a line — the through
    /// ball, the ball into the pocket, the one that costs possession
    /// when it fails. Vision and passing are most of it; the station
    /// scales it, because the same pass is worth more played from
    /// between the lines than from in front of your own centre-backs.
    pub creation: f32,
    /// Licence to run with it rather than release it. A regista's is
    /// low however good his feet are — carrying is not his job — and a
    /// mezzala's is high.
    pub carry: f32,
    /// Licence to hit it. Falls away sharply with depth: a deep
    /// midfielder's shot from 20 m is a bad decision even when he can
    /// strike a ball, and it was the engine's single largest surplus.
    pub shooting: f32,
    /// Licence to leave the midfield line and arrive in the area.
    ///
    /// ⚠ **Declared and not yet read.** The box run is still elected by
    /// `MidfielderAttackSupportingState::attacking_drive`, which is a
    /// raw-attribute blend with no station in it — so a holding
    /// midfielder with a good off-the-ball score can outrank the eight
    /// whose run it is. That election is the site this belongs at, and
    /// moving it is a behaviour change that needs its own paired run
    /// against `MidfieldPlay::legacy`; it is not folded into this pass.
    pub arrival: f32,
    /// Licence to hold the ball and set the tempo — the deep man's
    /// counterpart to `creation`. Highest for the player the side plays
    /// through.
    ///
    /// ⚠ Declared and not yet read, for the same reason: the natural
    /// site is the patient-possession branch of the on-ball tree, which
    /// currently asks `ctx.team().build_up_patience()` and so gives the
    /// whole side one temperament instead of giving the regista his.
    pub tempo: f32,
}

impl MidfieldRole {
    /// Where the midfield band starts and ends as a fraction of the
    /// pitch, measured from the player's own goal line. A 4-4-2's flat
    /// four sit at ~0.44, a 4-2-3-1's pivot at ~0.36 and its three at
    /// ~0.60, a 4-3-3's eights at ~0.50. Anchoring `station` to this
    /// band instead of to the whole pitch is what makes the number mean
    /// "where in the midfield", which is the only question being asked.
    const BAND_DEEP: f32 = 0.32;
    const BAND_HIGH: f32 = 0.66;

    /// Read the role of the midfielder this context is about.
    ///
    /// Pure in the frozen tick snapshot — no rolls, no memory — so two
    /// call sites in the same tick cannot disagree about who he is.
    pub fn read(ctx: &StateProcessingContext, profile: &MidfieldSkillView) -> Self {
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let slot = ctx.player.tactical_position.current_position;

        // ── Station ──────────────────────────────────────────────────
        // His FORMATION slot, not where he is standing: a holder who has
        // followed the ball into the final third is still a holder, and
        // reading his live position would make the role oscillate with
        // the play — which is the opposite of what a role is for.
        let slot_progress = ctx
            .player
            .side
            .map(|s| s.attacking_progress_x(ctx.player.start_position.x, field_width))
            .unwrap_or(0.5);
        let banded = ((slot_progress - Self::BAND_DEEP) / (Self::BAND_HIGH - Self::BAND_DEEP))
            .clamp(0.0, 1.0);
        // The slot LABEL is a second, coarser opinion about the same
        // thing, and the two disagree in the cases that matter — a
        // 4-2-3-1's AMC and a 4-4-2's MC can start within a few metres
        // of each other while being asked for completely different
        // games. Blended rather than switched on, so neither can produce
        // a cliff between two players in adjacent slots.
        let label_station = match slot {
            PlayerPositionType::AttackingMidfielderCenter
            | PlayerPositionType::AttackingMidfielderLeft
            | PlayerPositionType::AttackingMidfielderRight => 0.90,
            PlayerPositionType::MidfielderCenterLeft
            | PlayerPositionType::MidfielderCenter
            | PlayerPositionType::MidfielderCenterRight
            | PlayerPositionType::MidfielderLeft
            | PlayerPositionType::MidfielderRight => 0.45,
            PlayerPositionType::DefensiveMidfielder => 0.05,
            _ => banded,
        };
        let station = (banded * 0.45 + label_station * 0.55).clamp(0.0, 1.0);

        // ── Width ────────────────────────────────────────────────────
        let centre_y = field_height * 0.5;
        let slot_width = ((ctx.player.start_position.y - centre_y).abs() / (field_height * 0.42))
            .clamp(0.0, 1.0);
        let label_wide = matches!(
            slot,
            PlayerPositionType::MidfielderLeft
                | PlayerPositionType::MidfielderRight
                | PlayerPositionType::AttackingMidfielderLeft
                | PlayerPositionType::AttackingMidfielderRight
        );
        let width = if label_wide {
            slot_width.max(0.62)
        } else {
            slot_width
        };

        // ── Licences ─────────────────────────────────────────────────
        // Each is `who he is` × `where he was put`, both continuous.
        // The positional term never falls to zero: a deep midfielder
        // with a hammer of a pass still plays the occasional ball that
        // splits a back four, he just does not go looking for one every
        // time he touches it.
        let creation = (profile.progressive * (0.55 + station * 0.55)).clamp(0.0, 1.0);

        // Carrying is a job as much as a skill. A holding midfielder's
        // is deliberately capped low — the deepest man running with the
        // ball is how a side gets countered — and the wide man's is
        // lifted, because beating his full-back is most of what he is
        // out there to do.
        let carry_station = 0.45 + station * 0.45 + width * 0.25;
        let carry = (profile.carry * carry_station).clamp(0.0, 1.0);

        // Shooting falls off with depth harder than anything else here,
        // because it is where the engine's surplus was: 49.4% of every
        // midfield shot came from 16.5-22 m, and MID scored 61.4% of
        // all goals against a target of 32%. A deep midfielder needs a
        // genuinely exceptional strike to justify the attempt; an
        // arriving ten needs no justification at all.
        let shooting = (profile.strike * (0.22 + station * 0.95)).clamp(0.0, 1.0);

        // Arriving is off-the-ball and an engine, gated by station: the
        // holder does not arrive, whatever his stamina.
        let arrival = (profile.running * (0.20 + station * 1.00)).clamp(0.0, 1.0);

        // Tempo is the deep man's counterpart — composure and range,
        // most valuable where there is time to use them.
        let tempo = (profile.tempo * (1.05 - station * 0.45)).clamp(0.0, 1.0);

        let archetype = Self::name(station, width, creation, carry, arrival, tempo);

        MidfieldRole {
            archetype,
            station,
            width,
            creation,
            carry,
            shooting,
            arrival,
            tempo,
        }
    }

    /// Convenience for the many call sites that already hold a
    /// [`MidfielderSkillProfile`].
    pub fn of(ctx: &StateProcessingContext, profile: &MidfielderSkillProfile) -> Self {
        Self::read(ctx, &MidfieldSkillView::from_profile(ctx, profile))
    }

    /// Put a name to the mix. Ordered so the strongest claim wins, with
    /// the positional facts (a touchline slot, a slot in front of the
    /// back four) taking precedence over the attribute blend — those are
    /// instructions, and the rest is temperament.
    fn name(
        station: f32,
        width: f32,
        creation: f32,
        carry: f32,
        arrival: f32,
        tempo: f32,
    ) -> Archetype {
        if width > 0.55 {
            return Archetype::Wide;
        }
        if station < 0.25 {
            // Deep. Whether he is an anchor or a regista is decided by
            // whether the side plays through him.
            return if creation.max(tempo) > 0.45 {
                Archetype::DeepPlaymaker
            } else {
                Archetype::Anchor
            };
        }
        if station > 0.70 && creation > 0.40 {
            return Archetype::AdvancedPlaymaker;
        }
        if arrival >= creation && arrival >= carry * 0.9 {
            Archetype::BoxToBox
        } else if creation > 0.42 {
            Archetype::AdvancedPlaymaker
        } else {
            Archetype::BoxToBox
        }
    }

    /// Is this the man his side plays through? True for the deep
    /// playmaker and the advanced one — the two archetypes whose whole
    /// job is the ball, as against the two whose job is ground covered.
    #[inline]
    pub fn is_playmaker(&self) -> bool {
        matches!(
            self.archetype,
            Archetype::DeepPlaymaker | Archetype::AdvancedPlaymaker
        )
    }
}

/// The five attribute blends [`MidfieldRole`] needs, separated from the
/// context so the role can be built in a test without a live match.
///
/// Every read is already peer-shifted by
/// `MidfielderSkillProfile`,
/// except the two raw ones below, which are shifted here.
#[derive(Debug, Clone, Copy)]
pub struct MidfieldSkillView {
    /// Sees and plays the progressive ball.
    pub progressive: f32,
    /// Runs with it.
    pub carry: f32,
    /// Strikes it.
    pub strike: f32,
    /// Covers ground and times a run.
    pub running: f32,
    /// Holds it and sets the pace.
    pub tempo: f32,
}

impl MidfieldSkillView {
    pub fn from_profile(ctx: &StateProcessingContext, profile: &MidfielderSkillProfile) -> Self {
        let shift = MatchStandard::shift(ctx.context);
        let peer = |raw: f32| ((raw / 20.0).clamp(0.0, 1.0) - shift).clamp(0.0, 1.0);
        let s = &ctx.player.skills;

        // `progressive_selection` already blends vision / decisions /
        // passing on the profile's own curve; flair is what turns a
        // good passer into one who tries the ball nobody else sees, and
        // it is not in that composite.
        let progressive =
            (profile.progressive_selection * 0.78 + peer(s.mental.flair) * 0.22).clamp(0.0, 1.0);

        // Off-the-ball is the timing of the run, work rate is the engine
        // that repeats it, stamina is whether he still can at 70
        // minutes. `support_profile` carries the profile's own view.
        let running = (profile.support_profile * 0.60
            + peer(s.mental.off_the_ball) * 0.25
            + peer(s.physical.stamina) * 0.15)
            .clamp(0.0, 1.0);

        // Holding it under pressure and picking the pace: composure and
        // decisions, with press resistance for whether he keeps it when
        // somebody arrives.
        let tempo = (peer(s.mental.composure) * 0.38
            + peer(s.mental.decisions) * 0.32
            + profile.press_resistance * 0.30)
            .clamp(0.0, 1.0);

        MidfieldSkillView {
            progressive,
            carry: profile.carry_selection,
            strike: profile.mid_shot_selection,
            running,
            tempo,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(
        progressive: f32,
        carry: f32,
        strike: f32,
        running: f32,
        tempo: f32,
    ) -> MidfieldSkillView {
        MidfieldSkillView {
            progressive,
            carry,
            strike,
            running,
            tempo,
        }
    }

    /// The whole point of the module: two midfielders with IDENTICAL
    /// attributes, stationed differently, must be asked for different
    /// games. Before this existed they were asked for the same one.
    #[test]
    fn the_same_player_in_two_slots_is_two_different_footballers() {
        let v = view(0.60, 0.55, 0.55, 0.55, 0.55);
        let deep = licences(0.05, 0.0, &v);
        let high = licences(0.90, 0.0, &v);
        assert!(
            high.2 > deep.2 * 2.0,
            "the deep man must not shoot like the advanced one: {} vs {}",
            deep.2,
            high.2
        );
        assert!(high.0 > deep.0, "creation should rise up the pitch");
        assert!(high.3 > deep.3, "arrival should rise up the pitch");
        assert!(deep.4 > high.4, "tempo should fall up the pitch");
    }

    /// …and a role is never a licence of ZERO. A holding midfielder who
    /// can pass still plays the ball that beats a line occasionally;
    /// that is football, and a hard zero is the cliff this module exists
    /// to remove.
    #[test]
    fn no_licence_is_ever_switched_off() {
        let v = view(0.70, 0.70, 0.70, 0.70, 0.70);
        let deep = licences(0.0, 0.0, &v);
        assert!(deep.0 > 0.15, "creation floored out: {}", deep.0);
        assert!(deep.2 > 0.05, "shooting floored out: {}", deep.2);
        assert!(deep.3 > 0.05, "arrival floored out: {}", deep.3);
    }

    /// A poor player is a poor player wherever he stands — the station
    /// scales the licence, it does not manufacture one.
    #[test]
    fn station_scales_ability_it_does_not_replace_it() {
        let poor = view(0.05, 0.05, 0.05, 0.05, 0.05);
        let good = view(0.80, 0.80, 0.80, 0.80, 0.80);
        let poor_high = licences(1.0, 0.0, &poor);
        let good_deep = licences(0.0, 0.0, &good);
        assert!(poor_high.0 < good_deep.0);
        assert!(poor_high.2 < good_deep.2);
    }

    /// Naming is derived, and the touchline outranks the attribute mix:
    /// a wide slot is an instruction, not a temperament.
    #[test]
    fn a_touchline_slot_names_a_wide_player() {
        assert_eq!(
            MidfieldRole::name(0.9, 0.8, 0.9, 0.5, 0.5, 0.5),
            Archetype::Wide
        );
        assert_eq!(
            MidfieldRole::name(0.05, 0.1, 0.1, 0.2, 0.2, 0.2),
            Archetype::Anchor
        );
        assert_eq!(
            MidfieldRole::name(0.05, 0.1, 0.7, 0.2, 0.2, 0.7),
            Archetype::DeepPlaymaker
        );
        assert_eq!(
            MidfieldRole::name(0.85, 0.1, 0.7, 0.3, 0.4, 0.3),
            Archetype::AdvancedPlaymaker
        );
    }

    /// Reproduces the licence arithmetic of [`MidfieldRole::read`]
    /// without a `StateProcessingContext`, which cannot be fixtured
    /// here. Kept beside the real one deliberately: if the live form
    /// changes and this does not, the tests above stop describing the
    /// engine.
    fn licences(station: f32, width: f32, v: &MidfieldSkillView) -> (f32, f32, f32, f32, f32) {
        (
            (v.progressive * (0.55 + station * 0.55)).clamp(0.0, 1.0),
            (v.carry * (0.45 + station * 0.45 + width * 0.25)).clamp(0.0, 1.0),
            (v.strike * (0.22 + station * 0.95)).clamp(0.0, 1.0),
            (v.running * (0.20 + station * 1.00)).clamp(0.0, 1.0),
            (v.tempo * (1.05 - station * 0.45)).clamp(0.0, 1.0),
        )
    }
}
