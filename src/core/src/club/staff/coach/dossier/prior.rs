//! How much of an old read a coach still trusts.
//!
//! A manager who worked with a player for three seasons and parted two
//! years ago does not form a first impression when they meet again — but he
//! does not simply resume either, because two years is two years and the
//! man in front of him is not quite the one he remembers.
//!
//! The prior is that fraction. Everything a reunion does is scaled by it:
//! how many matches of credit the coach gives himself, how far each trust
//! axis starts from neutral, how slowly he re-rates what he sees, and
//! whether the old role is a floor under the new plan.
//!
//! Four things move it, and they are the four things that actually decide
//! whether an old opinion is still worth anything:
//!
//! | | |
//! |---|---|
//! | **time** | an exponential, because forgetting is |
//! | **depth** | five matches is an impression, twenty is a view |
//! | **age** | a man who has crossed thirty is a different footballer |
//! | **eye** | a good judge trusts his own old read further, and is more often right to |
//!
//! It is never zero — he does not forget a man he coached — and never one:
//! he always looks again.

use super::record::PlayerDossier;
use super::tuning::DossierTuning;
use crate::club::mind::organs::memory::EpochDay;
use crate::club::staff::perception::CoachProfile;

/// How far a coach's old view of a player still carries.
pub struct ReunionPrior;

impl ReunionPrior {
    /// The weight, 0..1, of what he already knew.
    pub fn compute(
        dossier: &PlayerDossier,
        age_now: u8,
        profile: &CoachProfile,
        today: EpochDay,
    ) -> f32 {
        let raw = Self::time(dossier, today)
            * Self::depth(dossier)
            * Self::age(dossier, age_now)
            * Self::eye(profile);
        raw.clamp(DossierTuning::PRIOR_MIN, DossierTuning::PRIOR_MAX)
    }

    /// Detail fades exponentially. One year → 0.78, three → 0.47, six →
    /// 0.22: he remembers, but he stops being sure it still holds.
    pub fn time(dossier: &PlayerDossier, today: EpochDay) -> f32 {
        (-dossier.years_apart(today) / DossierTuning::TAU_REUNION_YEARS).exp()
    }

    /// Five matches is an impression; twenty is a view.
    pub fn depth(dossier: &PlayerDossier) -> f32 {
        (dossier.matches_together as f32 / DossierTuning::DEPTH_FULL_AT_MATCHES)
            .clamp(DossierTuning::DEPTH_MIN, 1.0)
    }

    /// A player who has crossed thirty since, or who was a boy when the
    /// coach last saw him and is a man now, is not the footballer in the
    /// record — however well the coach remembers that one.
    pub fn age(dossier: &PlayerDossier, age_now: u8) -> f32 {
        let was = dossier.age_at_parting;
        let crossed_the_hill = age_now >= DossierTuning::AGE_BAND_OLD
            && was < DossierTuning::AGE_BAND_OLD;
        let grew_up =
            was <= DossierTuning::AGE_BAND_BOY && age_now >= DossierTuning::AGE_BAND_GROWN;
        if crossed_the_hill || grew_up {
            DossierTuning::AGE_BAND_PENALTY
        } else {
            1.0
        }
    }

    /// A good judge trusts his own old read further. Not the same as being
    /// right to — but on the whole he is, which is why the term points this
    /// way and not the other.
    pub fn eye(profile: &CoachProfile) -> f32 {
        DossierTuning::EYE_PRIOR_BASE
            + profile.judging_accuracy.clamp(0.0, 1.0) * DossierTuning::EYE_PRIOR_SPAN
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::record::SeparationCause;
    use crate::club::staff::StaffStub;
    use crate::Staff;

    const TODAY: EpochDay = 10_000;
    const YEAR: EpochDay = 365;

    /// Fixture builders, grouped so the tests read as sentences.
    struct Fx;

    impl Fx {
        fn parted(matches: u16, age: u8, years_ago: u16) -> PlayerDossier {
            let parted_on = TODAY - years_ago * YEAR;
            let mut record = PlayerDossier::opened(4, 1, parted_on);
            record.add_matches(matches);
            record.close(SeparationCause::IMovedOn, 1, age, parted_on);
            record
        }

        fn coach(judging: u8) -> Staff {
            let mut staff = StaffStub::default();
            staff.staff_attributes.knowledge.judging_player_ability = judging;
            staff
        }

        fn profile(judging: u8) -> CoachProfile {
            CoachProfile::from_staff(&Self::coach(judging))
        }
    }

    #[test]
    fn a_prior_is_never_nothing_and_never_everything() {
        let forgotten = Fx::parted(2, 24, 20);
        let yesterday = Fx::parted(300, 27, 0);
        let profile = Fx::profile(20);

        assert!(ReunionPrior::compute(&forgotten, 44, &profile, TODAY) >= DossierTuning::PRIOR_MIN);
        assert!(ReunionPrior::compute(&yesterday, 27, &profile, TODAY) <= DossierTuning::PRIOR_MAX);
    }

    #[test]
    fn three_seasons_together_two_years_ago_is_most_of_a_view() {
        let record = Fx::parted(90, 26, 2);
        let prior = ReunionPrior::compute(&record, 28, &Fx::profile(13), TODAY);
        assert!(
            prior > 0.45 && prior < 0.75,
            "he remembers him well but looks again: {prior}"
        );
    }

    #[test]
    fn five_years_and_a_thirtieth_birthday_leave_only_the_warmth() {
        let record = Fx::parted(90, 27, 5);
        let prior = ReunionPrior::compute(&record, 32, &Fx::profile(13), TODAY);
        assert!(
            prior < 0.25,
            "the detail is gone even though the man is not: {prior}"
        );
    }

    #[test]
    fn a_handful_of_matches_was_never_a_view_to_begin_with() {
        let glimpsed = Fx::parted(4, 25, 1);
        let known = Fx::parted(40, 25, 1);
        let profile = Fx::profile(13);
        assert!(
            ReunionPrior::compute(&glimpsed, 26, &profile, TODAY)
                < ReunionPrior::compute(&known, 26, &profile, TODAY) * 0.5
        );
    }

    #[test]
    fn a_boy_who_grew_up_elsewhere_is_a_different_player() {
        let boy = Fx::parted(40, 20, 4);
        let man = Fx::parted(40, 26, 4);
        let profile = Fx::profile(13);
        // The man stays inside his band; the boy has become somebody else.
        assert!(
            ReunionPrior::compute(&boy, 24, &profile, TODAY)
                < ReunionPrior::compute(&man, 29, &profile, TODAY)
        );
    }

    #[test]
    fn a_good_judge_trusts_his_own_old_read_further() {
        let record = Fx::parted(40, 26, 3);
        assert!(
            ReunionPrior::compute(&record, 29, &Fx::profile(19), TODAY)
                > ReunionPrior::compute(&record, 29, &Fx::profile(5), TODAY)
        );
    }
}
