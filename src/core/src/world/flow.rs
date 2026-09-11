/// Monthly free-agent market flow counters. Distinguishes the routes a
/// player leaves or enters the pool by, so a long run's diagnostics log
/// can tell apart "saved by a pre-contract" from "signed off the open
/// pool" from "still leaking into long-term free agency".
#[derive(Debug, Default, Clone, Copy)]
pub struct FreeAgentFlowCounters {
    /// Signed out of the cross-country global pool (`data.free_agents`).
    pub signed_from_global_pool: u32,
    /// Signed in-country off a just-expired domestic contract by the
    /// emergency / request / market-clearing matcher.
    pub signed_same_country_expired: u32,
    /// Moved on a staged pre-contract (Bosman) free transfer at expiry.
    pub signed_pre_contract: u32,
    /// Swept into the pool after a release / contract expiry this period.
    pub released_to_pool: u32,
    /// Retired straight out of the pool this period.
    pub retired_from_pool: u32,
}

impl FreeAgentFlowCounters {
    /// Zero every counter — called after the monthly diagnostics log reads
    /// them so the next month measures only its own flow.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}
