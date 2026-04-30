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
use crate::{HappinessEventType, PlayerStatusType};

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
            self.happiness
                .add_event_with_cooldown(HappinessEventType::MediaCriticism, mag, 14);
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
        self.happiness.add_event_with_partner_and_cooldown(
            HappinessEventType::CloseFriendSold,
            mag,
            Some(partner_player_id),
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
        self.happiness.add_event_with_partner_and_cooldown(
            HappinessEventType::MentorDeparted,
            mag,
            Some(partner_player_id),
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
        self.happiness.add_event_with_partner_and_cooldown(
            HappinessEventType::CompatriotJoined,
            mag,
            Some(partner_player_id),
            30,
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
        self.happiness
            .add_event_default(HappinessEventType::ContractTerminated);
    }
}
