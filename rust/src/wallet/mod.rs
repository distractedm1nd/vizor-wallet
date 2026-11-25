use std::num::NonZeroU32;

use zcash_client_backend::data_api::wallet::{ConfirmationTier, ConfirmationsPolicy};
use zcash_protocol::value::Zatoshis;

pub(crate) mod addresses;
pub(crate) mod balance_cache;
pub(crate) mod block_times;
pub(crate) mod bounded_runtime;
pub(crate) mod db;
pub(crate) mod endpoint_selection;
pub mod failure;
pub(crate) mod fees;
pub(crate) mod gift_card_tracking;
pub mod keys;
pub mod keystone;
pub mod ledger;
pub mod network;
pub(crate) mod network_upgrade;
pub(crate) mod one_time_ivk;
pub(crate) mod pir_config;
pub mod secret_payload;
pub mod secret_store;
pub mod sync;
pub mod sync_engine;
pub(crate) mod transaction_data;
pub(crate) mod transparent_receive_cache;
pub(crate) mod tree_states;
pub mod voting;
pub(crate) mod wallet_summary_cache;

// Confirmations before the wallet spends its own change. Anchoring one block
// behind the tip also allows externally received transfers under 10 ZEC to
// spend at 2 confirmations.
const TRUSTED_CONFIRMATIONS: u32 = 2;
// Vizor's product policy scales confirmations for externally received funds by
// transfer amount: < 10 ZEC => 2, < 1000 ZEC => 4, >= 1000 ZEC => 6.
const SMALL_TRANSFER_BELOW_ZAT: u64 = 1_000_000_000; // 10 ZEC
const SMALL_TRANSFER_CONFIRMATIONS: u32 = 2;
const MEDIUM_TRANSFER_BELOW_ZAT: u64 = 100_000_000_000; // 1000 ZEC
const MEDIUM_TRANSFER_CONFIRMATIONS: u32 = 4;
const UNTRUSTED_CONFIRMATIONS: u32 = 6;
const ALLOW_ZERO_CONFIRMATION_SHIELDING: bool = true;
// Gift Card claims spend a bearer card into the recipient's own wallet. Two
// confirmations anchor one block below the tip, so a one-block reorg cannot
// invalidate the claim.
const PAYMENT_LINK_CLAIM_CONFIRMATIONS: u32 = 2;

/// The SDK's trusted depth for funds that need `confirmations`. It anchors
/// that many blocks below the tip, so one fewer puts the anchor on the block
/// that completes them; a note above the anchor cannot be spent.
fn anchored_at(confirmations: u32) -> NonZeroU32 {
    NonZeroU32::new(confirmations - 1).expect("anchored confirmations exceed one")
}

fn confirmations_policy() -> ConfirmationsPolicy {
    ConfirmationsPolicy::new(
        anchored_at(TRUSTED_CONFIRMATIONS),
        NonZeroU32::new(UNTRUSTED_CONFIRMATIONS).expect("untrusted confirmations are nonzero"),
        ALLOW_ZERO_CONFIRMATION_SHIELDING,
    )
    .expect("trusted confirmations do not exceed untrusted confirmations")
    .with_untrusted_tiers(&[
        ConfirmationTier::new(
            Zatoshis::const_from_u64(SMALL_TRANSFER_BELOW_ZAT),
            NonZeroU32::new(SMALL_TRANSFER_CONFIRMATIONS)
                .expect("small transfer confirmations are nonzero"),
        ),
        ConfirmationTier::new(
            Zatoshis::const_from_u64(MEDIUM_TRANSFER_BELOW_ZAT),
            NonZeroU32::new(MEDIUM_TRANSFER_CONFIRMATIONS)
                .expect("medium transfer confirmations are nonzero"),
        ),
    ])
    .expect("amount confirmation tiers are valid")
}

fn payment_link_claim_confirmations_policy() -> ConfirmationsPolicy {
    let confirmations = anchored_at(PAYMENT_LINK_CLAIM_CONFIRMATIONS);
    ConfirmationsPolicy::new(confirmations, confirmations, false)
        .expect("claim confirmations are symmetric")
}

#[cfg(test)]
mod tests {
    use super::*;
    use zcash_client_backend::data_api::wallet::TargetHeight;
    use zcash_protocol::{consensus::BlockHeight, PoolType, ShieldedPool};
    use zip32::Scope;

    #[test]
    fn confirmation_policy_uses_six_confirmations_as_maximum_for_external_funds() {
        let policy = confirmations_policy();

        assert_eq!(u32::from(policy.untrusted()), UNTRUSTED_CONFIRMATIONS);
    }

    #[test]
    fn change_is_anchored_on_its_second_confirmation() {
        // The SDK anchors `trusted` blocks below the tip. Change mined at 100
        // has its second confirmation at tip 101.
        let tip = 100 + TRUSTED_CONFIRMATIONS - 1;
        assert_eq!(tip - u32::from(confirmations_policy().trusted()), 100);
    }

    #[test]
    fn external_funds_scale_confirmations_with_transfer_amount() {
        let policy = confirmations_policy();
        let confirmations_remaining = |target_height, amount_zat| {
            policy.confirmations_until_spendable_for_value(
                TargetHeight::from(target_height),
                PoolType::Shielded(ShieldedPool::Orchard),
                Some(Scope::External),
                Some(BlockHeight::from_u32(100)),
                false,
                None,
                false,
                Zatoshis::const_from_u64(amount_zat),
            )
        };

        // < 10 ZEC => 2 confirmations (spendable at target 102, i.e. tip 101).
        assert_eq!(
            confirmations_remaining(101, SMALL_TRANSFER_BELOW_ZAT - 1),
            1
        );
        assert_eq!(
            confirmations_remaining(102, SMALL_TRANSFER_BELOW_ZAT - 1),
            0
        );

        // 10 ZEC .. < 1000 ZEC => 4 confirmations (spendable at target 104, tip 103).
        assert_eq!(confirmations_remaining(103, SMALL_TRANSFER_BELOW_ZAT), 1);
        assert_eq!(confirmations_remaining(104, SMALL_TRANSFER_BELOW_ZAT), 0);
        assert_eq!(
            confirmations_remaining(103, MEDIUM_TRANSFER_BELOW_ZAT - 1),
            1
        );
        assert_eq!(
            confirmations_remaining(104, MEDIUM_TRANSFER_BELOW_ZAT - 1),
            0
        );

        // >= 1000 ZEC => 6 confirmations (spendable at target 106, tip 105).
        assert_eq!(confirmations_remaining(105, MEDIUM_TRANSFER_BELOW_ZAT), 1);
        assert_eq!(confirmations_remaining(106, MEDIUM_TRANSFER_BELOW_ZAT), 0);
    }

    #[test]
    fn gift_card_claim_policy_leaves_the_two_block_wait_to_the_anchor() {
        let policy = payment_link_claim_confirmations_policy();
        let confirmations_remaining = |target_height| {
            policy.confirmations_until_spendable(
                TargetHeight::from(target_height),
                PoolType::Shielded(ShieldedPool::Orchard),
                Some(Scope::External),
                Some(BlockHeight::from_u32(100)),
                false,
                None,
                false,
            )
        };

        assert_eq!(policy.trusted(), policy.untrusted());
        assert_eq!(
            u32::from(policy.trusted()),
            PAYMENT_LINK_CLAIM_CONFIRMATIONS - 1
        );
        // The policy alone is met in the funding block; the anchor one block
        // below the tip is what holds the claim until the next block.
        assert_eq!(confirmations_remaining(100), 1);
        assert_eq!(confirmations_remaining(101), 0);
        assert_eq!(
            sync_engine::gift_card_claim::max_claim_anchor_height(101),
            Some(100),
            "the funding block is first anchorable one block later"
        );
    }
}
