//! Social fallout from transfer activity: how *this* player reacts to
//! third-party events around them. Bid rejections, dream-move
//! collapses, a friend or mentor leaving, a compatriot arriving, a
//! mutual contract termination.
//!
//! These methods only mutate happiness/relations/statuses — they do
//! NOT touch contract/club state. Legal-state changes from the same
//! pipeline live in [`super::transfer`].

use chrono::NaiveDate;

use super::scaling;
use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::player::Player;
use crate::{
    ContractEventContext, ContractEventKind, HappinessEventCause, HappinessEventContext,
    HappinessEventEvidence, HappinessEventFollowUp, HappinessEventScope, HappinessEventSeverity,
    HappinessEventType, MediaFanEventContext, MediaFanEventKind, MediaFanSource, PlayerStatusType,
    TransferInterestContext, TransferInterestEvidence, TransferInterestKind,
    TransferInterestReaction, TransferInterestSource, TransferInterestStage, TransferSportingFit,
};

/// Inputs the transfer pipeline supplies when reporting any visible
/// stage of an interest event. The Player owner picks the reaction,
/// magnitude, evidence, and cooldown — the caller just states the
/// fact.
#[derive(Debug, Clone)]
pub struct TransferInterestSignal {
    pub interested_club_id: u32,
    pub interested_league_id: Option<u32>,
    /// Buyer reputation (0..1), as the negotiation pipeline already
    /// carries it. Used to compute the rep gap.
    pub buyer_rep: f32,
    /// Seller reputation (0..1).
    pub seller_rep: f32,
    /// Buying league reputation (raw 0..20000 score). 0 when unknown.
    pub buyer_league_rep: u16,
    /// Selling league reputation. 0 when unknown.
    pub seller_league_rep: u16,
    pub stage: TransferInterestStage,
    pub source: TransferInterestSource,
    /// True when the interested club has been linked repeatedly enough
    /// that the player's representatives have brought it up.
    pub repeated_attention: bool,
    /// True when the buyer is a known sporting rival of the seller.
    pub is_rival: bool,
    /// Domain meta — set when the buying club is in the player's home
    /// country.
    pub is_home_country: bool,
    /// True when the player is currently at a club in his home country
    /// (i.e. the selling club's country matches the player's nationality).
    /// Together with `is_home_country` this distinguishes a real
    /// homecoming from a domestic move where the player was already home.
    pub is_seller_in_home_country: bool,
    /// True when the buying club is a previous club of this player.
    pub is_former_club: bool,
    /// Country id of the buying club. 0 if unknown.
    pub buyer_country_id: u32,
    /// Continent id of the buying club's country (matches the
    /// `transfers::scouting_region` mapping: 1 = Europe, 3 = South
    /// America, …). 0 if unknown.
    pub buyer_continent_id: u32,
    /// Caller-supplied hint that the buying club is on a credible
    /// continental qualification path right now (top of league, large
    /// continental berth count, or active continental cup run). Used by
    /// the classifier to surface `EuropeanCompetitionOpportunity` /
    /// `CopaLibertadoresOpportunity` instead of generic StepUp.
    pub buyer_has_continental_path: bool,
    /// Caller-supplied label for the continental tier the buyer can
    /// realistically offer — distinguishes "Champions League ladder"
    /// from "Conference League marginal." Drives the satisfaction
    /// rules for high-CA stars.
    pub buyer_competition_path: Option<TransferContinentalPath>,
}

/// Coarse continental-path bucket the buyer can offer the player. Used
/// by the classifier to choose between the European / Libertadores
/// opportunity flavours and to gate satisfaction for high-CA stars
/// (a 150 CA player isn't satisfied by a Conference League marginal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferContinentalPath {
    /// Champions League regulars: top-band European clubs (UEFA pot 1-2).
    EliteEurope,
    /// Europa League / mid-band continental football.
    EuropaLeague,
    /// Conference League / qualifying-round level.
    ConferenceLeague,
    /// Copa Libertadores group stage / South American elite.
    Libertadores,
    /// Copa Sudamericana / sub-Libertadores South American.
    Sudamericana,
}

impl TransferInterestSignal {
    pub fn rep_diff(&self) -> f32 {
        self.buyer_rep - self.seller_rep
    }

    pub fn league_rep_diff(&self) -> i32 {
        self.buyer_league_rep as i32 - self.seller_league_rep as i32
    }
}

fn cause_for_interest_kind(kind: TransferInterestKind) -> HappinessEventCause {
    match kind {
        TransferInterestKind::FavoriteClubInterest
        | TransferInterestKind::Homecoming
        | TransferInterestKind::FormerClubReturn => HappinessEventCause::AdaptationIsolation,
        TransferInterestKind::RivalMove => HappinessEventCause::ReputationTension,
        TransferInterestKind::StepUp | TransferInterestKind::BigLeagueOpportunity => {
            HappinessEventCause::ReputationAdmiration
        }
        TransferInterestKind::EuropeanCompetitionOpportunity
        | TransferInterestKind::CopaLibertadoresOpportunity => {
            HappinessEventCause::ReputationAdmiration
        }
        TransferInterestKind::EscapeRoute | TransferInterestKind::StepDownWithMinutes => {
            HappinessEventCause::PoorFormPressure
        }
        _ => HappinessEventCause::MediaPressure,
    }
}

impl Player {
    /// An approach from `buyer_rep` has made it past the selling club's
    /// initial acceptance check, so it counts as real media-reported
    /// interest rather than a rumour. Flattery boost for ambitious
    /// players being chased upward; light destabilisation for the rest
    /// (rumour mill unsettles focus). Noop unless the gap is at least
    /// modest — generic peer-level interest isn't news.
    ///
    /// Cooldown gates re-firing when the same buyer keeps probing — the
    /// player has already heard the rumour mill in the past fortnight.
    pub fn on_transfer_interest_confirmed(&mut self, buyer_rep: f32, seller_rep: f32) {
        let rep_diff = buyer_rep - seller_rep;
        if rep_diff < 0.1 {
            return;
        }
        let ambition = self.attributes.ambition;
        if ambition >= 12.0 {
            // Ambitious player flattered by a bigger club chasing them —
            // proper "wanted by a bigger club" event, not a generic
            // manager talk.
            let mag = 1.0 + (rep_diff - 0.1).clamp(0.0, 0.6) * 4.0;
            self.happiness
                .add_event_with_cooldown(HappinessEventType::WantedByBiggerClub, mag, 14);
        } else {
            // Settled player disrupted by headline-grabbing rumour —
            // tabloid drama, modelled as media noise.
            let mag = -(0.5 + (rep_diff - 0.1).clamp(0.0, 0.4) * 2.0);
            let mfctx = MediaFanEventContext::new(
                MediaFanEventKind::SocialMediaCriticism,
                MediaFanSource::SocialMedia,
            )
            .with_transfer_trigger();
            let happiness_ctx = HappinessEventContext::new(
                HappinessEventCause::MediaPressure,
                HappinessEventSeverity::from_magnitude(mag),
                HappinessEventScope::Media,
            )
            .with_media_fan_context(mfctx);
            self.happiness.add_event_with_context_and_cooldown(
                HappinessEventType::MediaCriticism,
                mag,
                None,
                happiness_ctx,
                14,
            );
        }
    }

    /// Selling club rejected a real bid from a meaningfully bigger
    /// suitor, or from a club the player has flagged as a favorite.
    /// Magnitude grows with ambition and the rep gap, dampened by
    /// professionalism, amplified for favorite-club destinations.
    /// Cooldown 21d so a buying club's repeated bids don't pile this on.
    ///
    /// `buyer_rep` and `seller_rep` are normalised 0–1 reputation scores
    /// (the fields the negotiation already carries). `was_favorite_club`
    /// lifts the gating threshold and amplifies magnitude — a favorite
    /// club's bid being rejected stings even at a lateral move, where a
    /// generic peer-level rejection would otherwise be silent.
    pub fn on_transfer_bid_rejected(
        &mut self,
        buyer_rep: f32,
        seller_rep: f32,
        was_favorite_club: bool,
    ) {
        let rep_diff = buyer_rep - seller_rep;
        let ambition = self.attributes.ambition;
        let listed_or_unhappy = self.statuses.get().contains(&PlayerStatusType::Lst)
            || self.statuses.get().contains(&PlayerStatusType::Req)
            || self.statuses.get().contains(&PlayerStatusType::Unh)
            || self.statuses.get().contains(&PlayerStatusType::Trn);

        // Favorite-club bid: any meaningful approach (rep_diff > -0.05,
        // i.e. roughly peer-level or up) being rejected hurts even an
        // average-ambition player. Otherwise the existing gates apply.
        if was_favorite_club {
            if rep_diff < -0.05 {
                return;
            }
        } else {
            if rep_diff < 0.10 {
                return;
            }
            if ambition < 12.0 && !listed_or_unhappy {
                return;
            }
        }

        let cfg = HappinessConfig::default();
        let base = cfg.catalog.transfer_bid_rejected;
        let ambition_mul = scaling::ambition_amplifier(ambition);
        // Low loyalty stings more — the player wanted out.
        let loyalty = self.attributes.loyalty.clamp(0.0, 20.0) / 20.0;
        let loyalty_mul = 1.0 + (1.0 - loyalty) * 0.25;
        let prof_dampen = scaling::criticism_dampener(self.attributes.professionalism);
        // Bigger gap, sharper hit — but capped so the magnitude band stays sane.
        let gap_mul = 1.0 + (rep_diff.max(0.0) - 0.10).clamp(0.0, 0.40) * 1.5;
        let favorite_mul = if was_favorite_club { 1.25 } else { 1.0 };
        let mag = base * ambition_mul * loyalty_mul * prof_dampen * gap_mul * favorite_mul;
        self.happiness
            .add_event_with_cooldown(HappinessEventType::TransferBidRejected, mag, 21);
    }

    /// A late-stage transfer collapse — clubs agreed, terms agreed, and
    /// then the move fell over (medical, registration mishap). Stronger
    /// than a bid rejection. Only fires for meaningfully upward moves or
    /// known favorite-club destinations — collapse of a sideways move is
    /// merely annoying, not a "dream" gone.
    pub fn on_dream_move_collapsed(
        &mut self,
        buyer_rep: f32,
        seller_rep: f32,
        was_favorite_club: bool,
    ) {
        let rep_diff = buyer_rep - seller_rep;
        if !was_favorite_club && rep_diff < 0.15 {
            return;
        }
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.dream_move_collapsed;
        let ambition_mul = scaling::ambition_amplifier(self.attributes.ambition);
        let favorite_mul = if was_favorite_club { 1.30 } else { 1.0 };
        // Loyal players feel it less — they were less invested in leaving.
        let loyalty = self.attributes.loyalty.clamp(0.0, 20.0) / 20.0;
        let loyalty_dampen = 1.0 - 0.25 * loyalty;
        let mag = base * ambition_mul * favorite_mul * loyalty_dampen;
        // 30-day cooldown so a chain of failed reattempts doesn't stack.
        self.happiness
            .add_event_with_cooldown(HappinessEventType::DreamMoveCollapsed, mag, 30);
    }

    /// React to a teammate leaving the club. Caller has already determined
    /// that this player had a meaningful bond with the departing teammate
    /// and supplies the bond signals so the helper can pick the right
    /// event flavour (close-friend vs mentor) and dial magnitude.
    ///
    /// `bond_friendship` is the 0..100 friendship score and
    /// `same_nationality` / `is_long_term_teammate` modulate magnitude.
    pub fn on_close_friend_sold(
        &mut self,
        partner_player_id: u32,
        bond_friendship: f32,
        same_nationality: bool,
        departing_was_high_reputation: bool,
    ) {
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.close_friend_sold;
        // Bond strength: 65→1.0, 100→1.4
        let bond = ((bond_friendship - 65.0).clamp(0.0, 35.0) / 35.0) * 0.4 + 1.0;
        let nat_mul = if same_nationality { 1.20 } else { 1.0 };
        let rep_mul = if departing_was_high_reputation {
            1.15
        } else {
            1.0
        };
        let mag = base * bond * nat_mul * rep_mul;
        let mut ctx = HappinessEventContext::new(
            HappinessEventCause::FriendDeparture,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::DressingRoom,
        )
        .with_evidence(HappinessEventEvidence::StrongExistingBond)
        .with_follow_up(HappinessEventFollowUp::ManagerInterventionRisk);
        if same_nationality {
            ctx = ctx.with_evidence(HappinessEventEvidence::SharedNationality);
        }
        self.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::CloseFriendSold,
            mag,
            Some(partner_player_id),
            ctx,
            30,
        );
    }

    /// React to a veteran mentor leaving. Same call shape as
    /// `on_close_friend_sold` but tuned for the mentor / mentee dynamic —
    /// larger base hit, longer cooldown.
    pub fn on_mentor_departed(
        &mut self,
        partner_player_id: u32,
        bond_friendship: f32,
        same_nationality: bool,
    ) {
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.mentor_departed;
        let bond = ((bond_friendship.clamp(0.0, 100.0)) / 100.0) * 0.5 + 0.75;
        let nat_mul = if same_nationality { 1.15 } else { 1.0 };
        let mag = base * bond * nat_mul;
        let mut ctx = HappinessEventContext::new(
            HappinessEventCause::MentorDeparture,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::DressingRoom,
        )
        .with_evidence(HappinessEventEvidence::MentorInfluence)
        .with_follow_up(HappinessEventFollowUp::ManagerInterventionRisk);
        if same_nationality {
            ctx = ctx.with_evidence(HappinessEventEvidence::SharedNationality);
        }
        self.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::MentorDeparted,
            mag,
            Some(partner_player_id),
            ctx,
            60,
        );
    }

    /// React to a same-nationality player joining the squad. Strongest
    /// for foreign players who lack the local language; not emitted for
    /// domestic players in their home country (everyone speaks the same
    /// language, no integration boost).
    pub fn on_compatriot_joined(
        &mut self,
        partner_player_id: u32,
        club_country_id: u32,
        lacks_local_language: bool,
    ) {
        if self.country_id == club_country_id {
            return;
        }
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.compatriot_joined;
        // Foreign players lacking the local tongue lean on a compatriot
        // doubly hard — bigger lift than a foreign player who's already
        // settled linguistically.
        let lang_mul = if lacks_local_language { 1.30 } else { 1.0 };
        let mag = base * lang_mul;
        let mut ctx = HappinessEventContext::new(
            HappinessEventCause::NationalityIntegration,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::DressingRoom,
        )
        .with_evidence(HappinessEventEvidence::SharedNationality)
        .with_follow_up(HappinessEventFollowUp::SettlingInProgress);
        if lacks_local_language {
            ctx = ctx.with_evidence(HappinessEventEvidence::LanguageBarrier);
        }
        self.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::CompatriotJoined,
            mag,
            Some(partner_player_id),
            ctx,
            30,
        );
    }

    /// Report a visible transfer-interest moment and let the player react.
    ///
    /// The selector below is the single dispatch point for the rich
    /// interest funnel: scout sightings, rumours, agent leaks, concrete
    /// approaches from bigger / rival / favourite / former / homecoming
    /// clubs, talks-expected, interest-cooled, and contract-leverage
    /// moments. The pipeline states the *fact* (stage, source, club,
    /// rep gaps, repeated-attention) and the player picks the reaction
    /// and magnitude based on personality and current context.
    ///
    /// Returns `true` when the event landed (vs cooldown / threshold
    /// suppression).
    pub fn on_transfer_interest_signal(&mut self, sig: &TransferInterestSignal) -> bool {
        let kind = self.classify_interest_kind(sig);

        // Positive counterpart to a recent WantsReturnHome mood —
        // emitted on top of (not instead of) the regular interest
        // event, so the player feed shows both the standing rumour and
        // the "answers his homesickness" framing. Only fires for
        // concrete or further stages (rumours and scout sightings are
        // too vague to be a "real opportunity").
        self.maybe_emit_home_return_opportunity(sig, kind);

        // Visible-event gate: a stray scout sighting is not surfaced to
        // the player unless it actually means something — repeated
        // attention, agent/media leak, big-club gap, or contract noise.
        if !self.signal_should_surface(sig, kind) {
            return false;
        }

        let (event_type, base_mag, cooldown_days) = self.event_for_signal(sig, kind);
        let (reaction, fit, magnitude) =
            self.reaction_for_signal(sig, kind, base_mag);
        let evidence = self.evidence_for_signal(sig, kind);

        let real_homecoming = sig.is_home_country && !sig.is_seller_in_home_country;
        let mut tic = TransferInterestContext::new(
            sig.stage,
            sig.source,
            kind,
            reaction,
        )
        .with_interested_club(sig.interested_club_id)
        .with_reputation_gap((sig.rep_diff() * 100.0).round() as i32)
        .with_league_reputation_gap(sig.league_rep_diff())
        .with_rival(sig.is_rival)
        .with_home_country(real_homecoming)
        .with_former_club(sig.is_former_club)
        .with_favorite_club(self.favorite_clubs.contains(&sig.interested_club_id));
        if let Some(league_id) = sig.interested_league_id {
            tic = tic.with_interested_league(league_id);
        }
        if let Some(f) = fit {
            tic = tic.with_sporting_fit(f);
        }
        if let Some(c) = self.contract.as_ref() {
            tic = tic.with_current_squad_status(c.squad_status.clone());
        }
        for ev in evidence {
            tic = tic.with_evidence(ev);
        }

        let scope = match sig.source {
            TransferInterestSource::LocalPress
            | TransferInterestSource::NationalPress
            | TransferInterestSource::FanSpeculation => HappinessEventScope::Media,
            TransferInterestSource::ScoutAttendance => HappinessEventScope::MatchDay,
            TransferInterestSource::ContractTalk => HappinessEventScope::Boardroom,
            _ => HappinessEventScope::Personal,
        };

        let follow_up = self.follow_up_for_signal(sig, kind, reaction);
        let mut ctx = HappinessEventContext::new(
            cause_for_interest_kind(kind),
            HappinessEventSeverity::from_magnitude(magnitude),
            scope,
        )
        .with_transfer_interest_context(tic);
        if let Some(fu) = follow_up {
            ctx = ctx.with_follow_up(fu);
        }

        self.happiness.add_event_with_context_and_cooldown(
            event_type,
            magnitude,
            None,
            ctx,
            cooldown_days,
        )
    }

    /// Stage a `HomeReturnOpportunity` event when a concrete approach
    /// from a home / former / favourite club lands in the wake of a
    /// `WantsReturnHome` mood. Idempotent via cooldown — a stalled
    /// negotiation that ticks through multiple stages over a fortnight
    /// won't emit this twice.
    fn maybe_emit_home_return_opportunity(
        &mut self,
        sig: &TransferInterestSignal,
        kind: TransferInterestKind,
    ) {
        let stage_qualifies = matches!(
            sig.stage,
            TransferInterestStage::ConcreteInterest
                | TransferInterestStage::BidExpected
                | TransferInterestStage::BidSubmitted
                | TransferInterestStage::NegotiationsOpened
        );
        if !stage_qualifies {
            return;
        }
        let kind_qualifies = matches!(
            kind,
            TransferInterestKind::Homecoming
                | TransferInterestKind::FormerClubReturn
                | TransferInterestKind::FavoriteClubInterest
        );
        if !kind_qualifies {
            return;
        }
        if !self
            .happiness
            .has_recent_event(&HappinessEventType::WantsReturnHome, 180)
        {
            return;
        }
        let cfg = HappinessConfig::default();
        let mag = cfg.catalog.home_return_opportunity;
        let mut desire_ctx = crate::CareerDesireEventContext::new(
            crate::CareerDesireKind::ReturnHomeAfterPoorAdaptation,
        );
        desire_ctx = desire_ctx.with_evidence(crate::CareerDesireEvidence::HomeOrFavouriteLink);
        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::AdaptationIsolation,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::Personal,
        )
        .with_career_desire_context(desire_ctx)
        .with_follow_up(HappinessEventFollowUp::LikelyToSettle);
        self.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::HomeReturnOpportunity,
            mag,
            None,
            happiness_ctx,
            45,
        );
    }

    fn classify_interest_kind(&self, sig: &TransferInterestSignal) -> TransferInterestKind {
        if self.favorite_clubs.contains(&sig.interested_club_id) {
            return TransferInterestKind::FavoriteClubInterest;
        }
        if sig.is_former_club {
            return TransferInterestKind::FormerClubReturn;
        }
        if sig.is_home_country
            && !sig.is_seller_in_home_country
            && sig.rep_diff().abs() < 0.10
        {
            return TransferInterestKind::Homecoming;
        }
        if sig.is_rival {
            return TransferInterestKind::RivalMove;
        }
        // Continental-path opportunities — must be classified BEFORE
        // generic StepUp so a documented WantsEuropeanCompetition or
        // WantsCopaLibertadores mood routes through the matching
        // narrative even when the rep gap is modest. We don't fire
        // these when the buyer has no credible path or when the
        // satisfaction-tier guard fails for a star.
        if let Some(kind) = self.classify_continental_path_opportunity(sig) {
            return kind;
        }
        if sig.rep_diff() >= 0.20 {
            return TransferInterestKind::StepUp;
        }
        if sig.league_rep_diff() >= 2000 && sig.rep_diff() >= 0.05 {
            return TransferInterestKind::BigLeagueOpportunity;
        }
        if sig.rep_diff() <= -0.15 {
            // Smaller club: only meaningful as an escape route for a
            // player who isn't getting minutes today.
            if self.fringe_at_current_club() {
                return TransferInterestKind::EscapeRoute;
            }
            return TransferInterestKind::StepDownWithMinutes;
        }
        if sig.rep_diff().abs() < 0.10 {
            return TransferInterestKind::LateralMove;
        }
        TransferInterestKind::Speculative
    }

    /// Surface `EuropeanCompetitionOpportunity` /
    /// `CopaLibertadoresOpportunity` when the buyer can credibly offer
    /// the matching continental path AND either (a) the player is
    /// already carrying the desire mood or (b) personality / heritage
    /// makes it a natural fit. Returns `None` when neither flavour
    /// applies — the legacy step-up / lateral classifiers take over.
    ///
    /// Star-tier guard: a high-CA player (≥ 145) is NOT satisfied by a
    /// `ConferenceLeague` / `Sudamericana` path — the move still reads
    /// as a step-up at most, not a continental-ambition narrative.
    fn classify_continental_path_opportunity(
        &self,
        sig: &TransferInterestSignal,
    ) -> Option<TransferInterestKind> {
        if !sig.buyer_has_continental_path {
            return None;
        }
        let path = sig.buyer_competition_path?;
        let ca = self.player_attributes.current_ability;
        let high_ca = ca >= 145;
        match path {
            TransferContinentalPath::EliteEurope => {
                if self
                    .happiness
                    .has_recent_event(&HappinessEventType::WantsEuropeanCompetition, 120)
                    || self.attributes.ambition >= 16.0
                {
                    return Some(TransferInterestKind::EuropeanCompetitionOpportunity);
                }
            }
            TransferContinentalPath::EuropaLeague => {
                if self
                    .happiness
                    .has_recent_event(&HappinessEventType::WantsEuropeanCompetition, 120)
                    || self.attributes.ambition >= 14.0
                {
                    return Some(TransferInterestKind::EuropeanCompetitionOpportunity);
                }
            }
            TransferContinentalPath::ConferenceLeague => {
                if high_ca {
                    // High-CA stars aren't satisfied by Conference-tier
                    // football — let the legacy classifier label it.
                    return None;
                }
                if self
                    .happiness
                    .has_recent_event(&HappinessEventType::WantsEuropeanCompetition, 120)
                {
                    return Some(TransferInterestKind::EuropeanCompetitionOpportunity);
                }
            }
            TransferContinentalPath::Libertadores => {
                if self
                    .happiness
                    .has_recent_event(&HappinessEventType::WantsCopaLibertadores, 120)
                    || self.attributes.ambition >= 14.0
                {
                    return Some(TransferInterestKind::CopaLibertadoresOpportunity);
                }
            }
            TransferContinentalPath::Sudamericana => {
                if high_ca {
                    return None;
                }
                if self
                    .happiness
                    .has_recent_event(&HappinessEventType::WantsCopaLibertadores, 120)
                {
                    return Some(TransferInterestKind::CopaLibertadoresOpportunity);
                }
            }
        }
        None
    }

    fn fringe_at_current_club(&self) -> bool {
        use crate::PlayerSquadStatus as S;
        match self.contract.as_ref().map(|c| c.squad_status.clone()) {
            Some(S::MainBackupPlayer)
            | Some(S::FirstTeamSquadRotation)
            | Some(S::DecentYoungster)
            | Some(S::NotNeeded) => true,
            _ => false,
        }
    }

    fn signal_should_surface(
        &self,
        sig: &TransferInterestSignal,
        kind: TransferInterestKind,
    ) -> bool {
        match sig.stage {
            TransferInterestStage::ScoutWatched => {
                // Scouts at matches are everyday — surface only for
                // repeated attention, when the rep gap is meaningful, or
                // for the player's home-country / favourite / former /
                // rival club (emotional weight).
                if sig.repeated_attention || sig.rep_diff() >= 0.15 {
                    return true;
                }
                matches!(
                    kind,
                    TransferInterestKind::FavoriteClubInterest
                        | TransferInterestKind::FormerClubReturn
                        | TransferInterestKind::Homecoming
                        | TransferInterestKind::RivalMove
                )
            }
            TransferInterestStage::Shortlisted => {
                // Shortlisting is invisible unless it's leaked / agent
                // amplifies it, or a big-club gap exists.
                sig.repeated_attention || sig.rep_diff() >= 0.10
            }
            TransferInterestStage::AgentSoundingOut => true,
            TransferInterestStage::LooseRumour => sig.rep_diff() >= 0.05 || sig.is_rival,
            TransferInterestStage::ConcreteInterest
            | TransferInterestStage::BidExpected
            | TransferInterestStage::BidSubmitted
            | TransferInterestStage::BidRejected
            | TransferInterestStage::NegotiationsOpened
            | TransferInterestStage::NegotiationsStalled
            | TransferInterestStage::MoveCollapsed
            | TransferInterestStage::InterestCooled => true,
        }
    }

    fn event_for_signal(
        &self,
        sig: &TransferInterestSignal,
        kind: TransferInterestKind,
    ) -> (HappinessEventType, f32, u16) {
        let cfg = HappinessConfig::default();
        let cat = &cfg.catalog;
        match (sig.stage, kind) {
            (TransferInterestStage::ScoutWatched, _) => {
                (HappinessEventType::ScoutedByClub, cat.scouted_by_club, 45)
            }
            (TransferInterestStage::Shortlisted, _) => {
                (HappinessEventType::TransferRumour, cat.transfer_rumour, 30)
            }
            (TransferInterestStage::AgentSoundingOut, _) => (
                HappinessEventType::AgentStirsInterest,
                cat.agent_stirs_interest,
                45,
            ),
            (TransferInterestStage::LooseRumour, _) => {
                (HappinessEventType::TransferRumour, cat.transfer_rumour, 30)
            }
            (
                TransferInterestStage::ConcreteInterest
                | TransferInterestStage::BidExpected
                | TransferInterestStage::BidSubmitted,
                kind,
            ) => match kind {
                TransferInterestKind::FavoriteClubInterest => (
                    HappinessEventType::FavoriteClubInterest,
                    cat.favorite_club_interest,
                    14,
                ),
                TransferInterestKind::FormerClubReturn => (
                    HappinessEventType::FormerClubInterest,
                    cat.former_club_interest,
                    14,
                ),
                TransferInterestKind::Homecoming => (
                    HappinessEventType::HomecomingRumour,
                    cat.homecoming_rumour,
                    14,
                ),
                TransferInterestKind::RivalMove => (
                    HappinessEventType::InterestFromRival,
                    cat.interest_from_rival,
                    14,
                ),
                TransferInterestKind::StepUp
                | TransferInterestKind::BigLeagueOpportunity => (
                    HappinessEventType::InterestFromBiggerClub,
                    cat.interest_from_bigger_club,
                    14,
                ),
                _ => (
                    HappinessEventType::WantedByBiggerClub,
                    cat.wanted_by_bigger_club,
                    14,
                ),
            },
            (TransferInterestStage::NegotiationsOpened, _) => (
                HappinessEventType::TransferTalksExpected,
                cat.transfer_talks_expected,
                14,
            ),
            (TransferInterestStage::NegotiationsStalled, _)
            | (TransferInterestStage::InterestCooled, _) => {
                (HappinessEventType::InterestCooled, cat.interest_cooled, 30)
            }
            (TransferInterestStage::BidRejected, _) => (
                HappinessEventType::TransferBidRejected,
                cat.transfer_bid_rejected,
                21,
            ),
            (TransferInterestStage::MoveCollapsed, _) => (
                HappinessEventType::DreamMoveCollapsed,
                cat.dream_move_collapsed,
                30,
            ),
        }
    }

    fn reaction_for_signal(
        &self,
        sig: &TransferInterestSignal,
        kind: TransferInterestKind,
        base_mag: f32,
    ) -> (
        TransferInterestReaction,
        Option<TransferSportingFit>,
        f32,
    ) {
        let ambition = self.attributes.ambition;
        let loyalty = self.attributes.loyalty;
        let professionalism = self.attributes.professionalism;
        let controversy = self.attributes.controversy;
        let fringe = self.fringe_at_current_club();
        let listed_or_unhappy = self.statuses.get().contains(&PlayerStatusType::Lst)
            || self.statuses.get().contains(&PlayerStatusType::Req)
            || self.statuses.get().contains(&PlayerStatusType::Unh)
            || self.statuses.get().contains(&PlayerStatusType::Trn);

        let (reaction, fit) = match kind {
            TransferInterestKind::StepUp | TransferInterestKind::BigLeagueOpportunity => {
                if ambition >= 14.0 {
                    (
                        TransferInterestReaction::Excited,
                        Some(TransferSportingFit::ClearUpgrade),
                    )
                } else if loyalty >= 14.0 {
                    (
                        TransferInterestReaction::PubliclyCalmPrivatelyInterested,
                        Some(TransferSportingFit::ClearUpgrade),
                    )
                } else if fringe {
                    (
                        TransferInterestReaction::WantsTalks,
                        Some(TransferSportingFit::BiggerClubButHarderMinutes),
                    )
                } else {
                    (
                        TransferInterestReaction::Flattered,
                        Some(TransferSportingFit::ClearUpgrade),
                    )
                }
            }
            TransferInterestKind::FavoriteClubInterest => (
                TransferInterestReaction::Excited,
                Some(TransferSportingFit::EmotionalFit),
            ),
            TransferInterestKind::FormerClubReturn => {
                if loyalty >= 13.0 {
                    (
                        TransferInterestReaction::Cautious,
                        Some(TransferSportingFit::EmotionalFit),
                    )
                } else {
                    (
                        TransferInterestReaction::Flattered,
                        Some(TransferSportingFit::EmotionalFit),
                    )
                }
            }
            TransferInterestKind::Homecoming => (
                TransferInterestReaction::Excited,
                Some(TransferSportingFit::EmotionalFit),
            ),
            TransferInterestKind::RivalMove => {
                if loyalty >= 13.0 {
                    (
                        TransferInterestReaction::LoyalToCurrentClub,
                        Some(TransferSportingFit::TacticalFit),
                    )
                } else if professionalism >= 14.0 {
                    (
                        TransferInterestReaction::PubliclyCalmPrivatelyInterested,
                        Some(TransferSportingFit::TacticalFit),
                    )
                } else {
                    (
                        TransferInterestReaction::AnnoyedBySpeculation,
                        Some(TransferSportingFit::TacticalFit),
                    )
                }
            }
            TransferInterestKind::EscapeRoute => (
                TransferInterestReaction::WantsTalks,
                Some(TransferSportingFit::BetterPlayingTime),
            ),
            TransferInterestKind::StepDownWithMinutes => {
                if fringe {
                    (
                        TransferInterestReaction::WantsTalks,
                        Some(TransferSportingFit::BetterPlayingTime),
                    )
                } else {
                    (
                        TransferInterestReaction::Cautious,
                        Some(TransferSportingFit::PoorRoleFit),
                    )
                }
            }
            TransferInterestKind::LateralMove => (
                if professionalism >= 14.0 {
                    TransferInterestReaction::Focused
                } else {
                    TransferInterestReaction::Distracted
                },
                None,
            ),
            TransferInterestKind::LoanDevelopment => (
                TransferInterestReaction::Cautious,
                Some(TransferSportingFit::BetterPlayingTime),
            ),
            TransferInterestKind::Speculative => (
                if controversy >= 14.0 {
                    TransferInterestReaction::AnnoyedBySpeculation
                } else if professionalism >= 14.0 {
                    TransferInterestReaction::Focused
                } else {
                    TransferInterestReaction::Distracted
                },
                None,
            ),
            // Continental-path opportunities: stronger Excited /
            // WantsTalks reactions when the player carries the
            // matching desire mood, otherwise treat like a regular
            // step-up.
            TransferInterestKind::EuropeanCompetitionOpportunity => {
                let has_desire = self.happiness.has_recent_event(
                    &HappinessEventType::WantsEuropeanCompetition,
                    120,
                );
                if has_desire || ambition >= 16.0 {
                    (
                        TransferInterestReaction::WantsTalks,
                        Some(TransferSportingFit::ClearUpgrade),
                    )
                } else if loyalty >= 14.0 {
                    (
                        TransferInterestReaction::PubliclyCalmPrivatelyInterested,
                        Some(TransferSportingFit::ClearUpgrade),
                    )
                } else {
                    (
                        TransferInterestReaction::Excited,
                        Some(TransferSportingFit::ClearUpgrade),
                    )
                }
            }
            TransferInterestKind::CopaLibertadoresOpportunity => {
                let has_desire = self.happiness.has_recent_event(
                    &HappinessEventType::WantsCopaLibertadores,
                    120,
                );
                if has_desire {
                    (
                        TransferInterestReaction::WantsTalks,
                        Some(TransferSportingFit::EmotionalFit),
                    )
                } else {
                    (
                        TransferInterestReaction::Excited,
                        Some(TransferSportingFit::EmotionalFit),
                    )
                }
            }
        };

        // Magnitude — base scaled by personality + listed/unhappy + stage.
        let amb_mul = scaling::ambition_amplifier(ambition);
        let loy_mul = scaling::loyalty_amplifier(loyalty);
        let prof_dampen = scaling::criticism_dampener(professionalism);

        let stage_factor = match sig.stage {
            TransferInterestStage::ScoutWatched => 0.7,
            TransferInterestStage::Shortlisted | TransferInterestStage::LooseRumour => 0.85,
            TransferInterestStage::AgentSoundingOut => 1.0,
            TransferInterestStage::ConcreteInterest => 1.0,
            TransferInterestStage::BidExpected | TransferInterestStage::BidSubmitted => 1.15,
            TransferInterestStage::BidRejected => 1.0,
            TransferInterestStage::NegotiationsOpened => 1.0,
            TransferInterestStage::NegotiationsStalled => 0.8,
            TransferInterestStage::MoveCollapsed => 1.0,
            TransferInterestStage::InterestCooled => 0.8,
        };

        let mut mag = base_mag * stage_factor;
        // Polarity of the catalog entry already encodes positive/negative.
        if mag > 0.0 {
            mag *= amb_mul;
            // Loyal players experience positive interest as more
            // measured — dampen a bit.
            if loyalty >= 14.0 {
                mag *= 0.65;
            }
        } else if mag < 0.0 {
            // Negative effects (rival, cooled) dampened by professionalism.
            mag *= prof_dampen;
        }
        // Listed/unhappy player: positive interest feels like an escape
        // route — amplified.
        if listed_or_unhappy && mag > 0.0 {
            mag *= 1.2;
        }
        // Repeated speculation: pressure-load drag on settled players.
        if sig.repeated_attention && mag.abs() < 0.5 {
            mag = mag.min(-0.5);
        }
        let _ = loy_mul; // reserved for future fan-pressure ramp
        (reaction, fit, mag)
    }

    fn evidence_for_signal(
        &self,
        sig: &TransferInterestSignal,
        kind: TransferInterestKind,
    ) -> Vec<TransferInterestEvidence> {
        let mut out: Vec<TransferInterestEvidence> = Vec::new();
        if sig.rep_diff() >= 0.15 {
            out.push(TransferInterestEvidence::BiggerClub);
        }
        if sig.league_rep_diff() >= 1500 {
            out.push(TransferInterestEvidence::BiggerLeague);
        }
        if sig.is_rival {
            out.push(TransferInterestEvidence::RivalClub);
        }
        if sig.is_former_club {
            out.push(TransferInterestEvidence::FormerClub);
        }
        if self.favorite_clubs.contains(&sig.interested_club_id) {
            out.push(TransferInterestEvidence::FavoriteClub);
        }
        // Home-country evidence should mean a real homecoming: the buyer
        // is in the player's home country and the seller is not. A
        // domestic move while the player is already home is not a
        // "return home" story.
        if sig.is_home_country && !sig.is_seller_in_home_country {
            out.push(TransferInterestEvidence::HomeCountry);
        }
        if sig.repeated_attention {
            out.push(TransferInterestEvidence::RepeatedRumours);
        }
        if matches!(sig.source, TransferInterestSource::ScoutAttendance) {
            out.push(TransferInterestEvidence::ScoutAtMatch);
        }
        if matches!(sig.source, TransferInterestSource::AgentLeak) {
            out.push(TransferInterestEvidence::AgentPushing);
        }
        if matches!(
            sig.source,
            TransferInterestSource::LocalPress | TransferInterestSource::NationalPress
        ) {
            out.push(TransferInterestEvidence::MediaNoise);
        }
        if matches!(sig.source, TransferInterestSource::FanSpeculation) {
            out.push(TransferInterestEvidence::FanPressure);
        }
        if matches!(sig.stage, TransferInterestStage::BidRejected) {
            out.push(TransferInterestEvidence::RejectedBid);
        }
        if self.attributes.ambition >= 15.0 {
            out.push(TransferInterestEvidence::HighAmbition);
        } else if self.attributes.ambition <= 7.0 {
            out.push(TransferInterestEvidence::LowAmbition);
        }
        if self.attributes.loyalty >= 15.0 {
            out.push(TransferInterestEvidence::HighLoyalty);
        } else if self.attributes.loyalty <= 7.0 {
            out.push(TransferInterestEvidence::LowLoyalty);
        }
        if self.attributes.professionalism >= 16.0 {
            out.push(TransferInterestEvidence::HighProfessionalism);
        }
        if self.attributes.controversy >= 15.0 {
            out.push(TransferInterestEvidence::HighControversy);
        }
        if self.fringe_at_current_club()
            && matches!(
                kind,
                TransferInterestKind::EscapeRoute
                    | TransferInterestKind::StepDownWithMinutes
                    | TransferInterestKind::StepUp
            )
        {
            out.push(TransferInterestEvidence::CurrentPlayingTimeFrustration);
            out.push(TransferInterestEvidence::MoreLikelyStarts);
        }
        // Contract context — within 12 months of expiry?
        if let Some(c) = self.contract.as_ref() {
            let expiry = c.expiration;
            // Rough guard: if expiry is close (≤365 days) flag it.
            // We don't have a date here, so the caller can supply via a
            // future sig field; for now we flag when an active loan or
            // listed/unhappy state hints contract pressure.
            let _ = expiry;
        }
        if self.statuses.get().contains(&PlayerStatusType::Unh)
            || self.statuses.get().contains(&PlayerStatusType::Req)
        {
            out.push(TransferInterestEvidence::CurrentClubAmbitionMismatch);
        }
        // Career-desire alignment — when the player is carrying a
        // documented career-desire mood, flag the matching evidence so
        // the renderer can call out "answers his open homesickness"
        // / "matches his European-ambition mood" instead of relying
        // on generic step-up copy.
        if matches!(kind, TransferInterestKind::EuropeanCompetitionOpportunity)
            || self
                .happiness
                .has_recent_event(&HappinessEventType::WantsEuropeanCompetition, 120)
        {
            out.push(TransferInterestEvidence::EuropeanCompetitionOpportunity);
        }
        if matches!(kind, TransferInterestKind::CopaLibertadoresOpportunity)
            || self
                .happiness
                .has_recent_event(&HappinessEventType::WantsCopaLibertadores, 120)
        {
            out.push(TransferInterestEvidence::CopaLibertadoresOpportunity);
        }
        if ((sig.is_home_country && !sig.is_seller_in_home_country)
            || sig.is_former_club
            || self.favorite_clubs.contains(&sig.interested_club_id))
            && self
                .happiness
                .has_recent_event(&HappinessEventType::WantsReturnHome, 120)
        {
            out.push(TransferInterestEvidence::ReturnHomeRelief);
        }
        out
    }

    fn follow_up_for_signal(
        &self,
        sig: &TransferInterestSignal,
        kind: TransferInterestKind,
        reaction: TransferInterestReaction,
    ) -> Option<HappinessEventFollowUp> {
        match (sig.stage, kind, reaction) {
            (TransferInterestStage::BidRejected, _, _) => {
                Some(HappinessEventFollowUp::ContractRequestRisk)
            }
            (TransferInterestStage::MoveCollapsed, _, _) => {
                Some(HappinessEventFollowUp::ManagerInterventionRisk)
            }
            (TransferInterestStage::InterestCooled, _, _) => {
                Some(HappinessEventFollowUp::LikelyToSettle)
            }
            (_, _, TransferInterestReaction::WantsTalks) => {
                Some(HappinessEventFollowUp::ContractRequestRisk)
            }
            (_, _, TransferInterestReaction::Excited)
            | (_, _, TransferInterestReaction::Distracted) => {
                Some(HappinessEventFollowUp::PressureBuilding)
            }
            (_, _, TransferInterestReaction::LoyalToCurrentClub) => {
                Some(HappinessEventFollowUp::LikelyToSettle)
            }
            _ => None,
        }
    }

    /// Player publicly dismissed the speculation. Small positive PR /
    /// dressing-room note. Cooldown so the same player doesn't get a
    /// dismissal headline every week of the same rumour cycle.
    pub fn on_transfer_interest_dismissed(&mut self, interested_club_id: u32) {
        let cfg = HappinessConfig::default();
        let mag = cfg.catalog.transfer_interest_dismissed;
        let tic = TransferInterestContext::new(
            TransferInterestStage::InterestCooled,
            TransferInterestSource::NationalPress,
            TransferInterestKind::Speculative,
            TransferInterestReaction::LoyalToCurrentClub,
        )
        .with_interested_club(interested_club_id);
        let ctx = HappinessEventContext::new(
            HappinessEventCause::Other,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::Media,
        )
        .with_transfer_interest_context(tic)
        .with_follow_up(HappinessEventFollowUp::LikelyToSettle);
        self.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::TransferInterestDismissed,
            mag,
            None,
            ctx,
            21,
        );
    }

    /// Player used external interest as leverage during a contract talk.
    /// Sets `pending_signing`-style follow-up risk via FollowUp tag.
    pub fn on_used_interest_for_contract_leverage(&mut self, interested_club_id: u32) {
        let cfg = HappinessConfig::default();
        let mag = cfg.catalog.used_interest_for_contract_leverage;
        let tic = TransferInterestContext::new(
            TransferInterestStage::ConcreteInterest,
            TransferInterestSource::ContractTalk,
            TransferInterestKind::Speculative,
            TransferInterestReaction::UsesInterestForContractLeverage,
        )
        .with_interested_club(interested_club_id)
        .with_evidence(TransferInterestEvidence::AgentPushing);
        let ctx = HappinessEventContext::new(
            HappinessEventCause::Other,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::Boardroom,
        )
        .with_transfer_interest_context(tic)
        .with_follow_up(HappinessEventFollowUp::ContractRequestRisk);
        self.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::UsedInterestForContractLeverage,
            mag,
            None,
            ctx,
            45,
        );
    }

    /// Count visible transfer-interest events emitted in the last `days`
    /// days. Used by the weekly tick to apply background distraction
    /// pressure for players whose situation hasn't been resolved.
    pub fn count_recent_transfer_interest_events(&self, days: u16) -> u8 {
        let mut n: u8 = 0;
        for e in &self.happiness.recent_events {
            if e.days_ago > days {
                continue;
            }
            let is_interest = matches!(
                e.event_type,
                HappinessEventType::ScoutedByClub
                    | HappinessEventType::TransferRumour
                    | HappinessEventType::AgentStirsInterest
                    | HappinessEventType::InterestFromBiggerClub
                    | HappinessEventType::InterestFromRival
                    | HappinessEventType::HomecomingRumour
                    | HappinessEventType::FormerClubInterest
                    | HappinessEventType::FavoriteClubInterest
                    | HappinessEventType::TransferTalksExpected
                    | HappinessEventType::WantedByBiggerClub
            );
            if is_interest {
                n = n.saturating_add(1);
            }
        }
        n
    }

    /// Weekly background drag from unresolved interest. The pipeline
    /// passes in a count of recent visible interest events; this method
    /// emits a `TransferSpeculationDistracts` event when the count plus
    /// personality justify it.
    pub fn on_unresolved_speculation_pressure(&mut self, recent_visible_events: u8) {
        if recent_visible_events < 2 {
            return;
        }
        let prof = self.attributes.professionalism;
        if prof >= 16.0 {
            return;
        }
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.transfer_speculation_distracts;
        let mut mag = base;
        if recent_visible_events >= 4 {
            mag *= 1.5;
        }
        if self.attributes.controversy >= 14.0 {
            mag *= 1.25;
        }
        let tic = TransferInterestContext::new(
            TransferInterestStage::LooseRumour,
            TransferInterestSource::NationalPress,
            TransferInterestKind::Speculative,
            TransferInterestReaction::Distracted,
        )
        .with_evidence(TransferInterestEvidence::RepeatedRumours)
        .with_evidence(TransferInterestEvidence::MediaNoise);
        let ctx = HappinessEventContext::new(
            HappinessEventCause::MediaPressure,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::Media,
        )
        .with_transfer_interest_context(tic)
        .with_follow_up(HappinessEventFollowUp::PressureBuilding);
        self.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::TransferSpeculationDistracts,
            mag,
            None,
            ctx,
            21,
        );
    }

    /// React to a mutual contract termination. Clears the contract (player
    /// becomes a free agent), drops transfer statuses that no longer apply,
    /// and logs a mild morale event — it's a blow to pride, but freedom
    /// plus a payout softens it considerably.
    pub fn on_contract_terminated(&mut self, _date: NaiveDate) {
        self.contract = None;
        self.contract_loan = None;
        for s in [
            PlayerStatusType::Lst,
            PlayerStatusType::Req,
            PlayerStatusType::Unh,
            PlayerStatusType::Trn,
            PlayerStatusType::Bid,
        ] {
            self.statuses.remove(s);
        }
        let cctx = ContractEventContext::new(ContractEventKind::Terminated);
        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::Other,
            HappinessEventSeverity::Moderate,
            HappinessEventScope::Boardroom,
        )
        .with_contract_context(cctx);
        let mag = HappinessConfig::default()
            .catalog
            .magnitude(HappinessEventType::ContractTerminated);
        self.happiness.add_event_with_context(
            HappinessEventType::ContractTerminated,
            mag,
            None,
            happiness_ctx,
        );
    }
}
