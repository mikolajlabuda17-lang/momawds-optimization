//! Lossless Experience Orb clumping and zero-cooldown multi-orb pickup resolution.
//!
//! Vanilla saves orb `Value` as a signed 16-bit integer (`-32_768..=32_767`) and `Count` as a
//! positive 32-bit signed integer (`1..=i32::MAX`). Every merge computed here preserves the exact
//! `u128` XP sum within those NBT bounds, refusing the merge if no exact factorization exists.

const MAX_ORB_VALUE: u128 = i16::MAX as u128; // 32_767
const MAX_ORB_COUNT: u128 = i32::MAX as u128; // 2_147_483_647

/// Total non-negative XP points held by an orb (`negative` values pay 0 in vanilla).
#[inline]
pub fn orb_total_xp(value: i32, count: u32) -> u128 {
    (value.max(0) as u128) * (count as u128)
}

/// Finds a `(value, count)` pair in valid saved NBT bounds whose product equals `total` and
/// differs from `(into_value, into_count)` so Horizon records an explicit `set_experience_orb`
/// override rather than adding `from_count` a second time.
fn find_exact_factorization(total: u128, into_value: i32, into_count: u32) -> Option<(i32, u32)> {
    if total == 0 || total > MAX_ORB_VALUE * MAX_ORB_COUNT {
        return None;
    }
    let total_u64 = total as u64;
    let min_val = total_u64.div_ceil(MAX_ORB_COUNT as u64);
    let max_val = total_u64.min(MAX_ORB_VALUE as u64);
    for value in (min_val..=max_val).rev() {
        if total_u64.is_multiple_of(value) {
            let count = (total_u64 / value) as u32;
            let val_i32 = value as i32;
            if (val_i32, count) != (into_value, into_count) {
                return Some((val_i32, count));
            }
        }
    }
    None
}

/// Action to take inside `experience_orb_merge`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrbMergePlan {
    /// Leave `into` untouched so the engine adds `from_count` directly.
    LetEngineAddCounts,
    /// Rewrite `into` via `set_experience_orb(value, count)` so `from` is absorbed cleanly.
    RewriteInto { value: i32, count: u32 },
    /// Cancel the merge so neither orb loses points.
    Cancel,
}

/// Plans an exact, lossless merge of `from` (`from_value`, `from_count`) into `into`
/// (`into_value`, `into_count`).
pub fn plan_orb_merge(
    into_value: i32,
    into_count: u32,
    from_value: i32,
    from_count: u32,
) -> OrbMergePlan {
    let total = orb_total_xp(into_value, into_count) + orb_total_xp(from_value, from_count);
    if total == 0 {
        if into_value == 0 {
            return match into_count.checked_add(from_count) {
                Some(sum) if sum >= 1 && sum <= MAX_ORB_COUNT as u32 => {
                    OrbMergePlan::LetEngineAddCounts
                }
                _ => OrbMergePlan::Cancel,
            };
        }
        return OrbMergePlan::RewriteInto { value: 0, count: 1 };
    }
    match find_exact_factorization(total, into_value, into_count) {
        Some((value, count)) => OrbMergePlan::RewriteInto { value, count },
        None => OrbMergePlan::Cancel,
    }
}

/// Collapses multi-count clumped orbs into a single XP payout at pickup time whenever the total
/// fits in `i32`, so Mending and player XP absorb the entire clump in one pickup event.
#[inline]
pub fn resolve_orb_pickup(value: i32, count: u32) -> (i32, u32) {
    if count <= 1 {
        return (value, count);
    }
    let total = orb_total_xp(value, count);
    if total == 0 {
        (0, 1)
    } else if total <= i32::MAX as u128 {
        (total as i32, 1)
    } else {
        (value, count)
    }
}
