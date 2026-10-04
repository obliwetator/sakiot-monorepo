//! How synthetic ids are laid out inside the reserved range
//! (`crate::synthetic`). Each fixture guild owns one slot of
//! [`GUILD_STRIDE`] ids, and every channel, role and user it creates lies in
//! that slot, so a guild's rows can be found and deleted by range.

use crate::synthetic::SYNTHETIC_ID_FLOOR;

const GUILD_STRIDE: i64 = 1_000_000_000_000;
pub(super) const MAX_GUILD: u32 = 999;
const CHANNEL_BASE: i64 = 1_000;
const ROLE_BASE: i64 = 2_000;
const USER_BASE: i64 = 1_000_000;

pub(super) fn guild_id(guild: u32) -> i64 {
    SYNTHETIC_ID_FLOOR + i64::from(guild) * GUILD_STRIDE
}

/// The fixture guild number whose slot holds `id`, for any synthetic id.
pub(super) fn guild_of(id: i64) -> Option<u32> {
    if id < SYNTHETIC_ID_FLOOR {
        return None;
    }
    u32::try_from((id - SYNTHETIC_ID_FLOOR) / GUILD_STRIDE)
        .ok()
        .filter(|guild| (1..=MAX_GUILD).contains(guild))
}

pub(super) fn channel_id(guild_id: i64, index: u32) -> i64 {
    guild_id + CHANNEL_BASE + i64::from(index)
}

/// A role other than `@everyone`, whose id is the guild id itself.
pub(super) fn role_id(guild_id: i64, index: u32) -> i64 {
    guild_id + ROLE_BASE + i64::from(index)
}

pub(super) fn user_id(guild_id: i64, index: u32) -> i64 {
    guild_id + USER_BASE + i64::from(index)
}

/// The half-open id range a guild's rows live in.
pub(super) fn slot(guild_id: i64) -> (i64, i64) {
    (guild_id, guild_id + GUILD_STRIDE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_id_maps_back_to_its_guild() {
        for guild in [1, 7, MAX_GUILD] {
            let id = guild_id(guild);
            assert_eq!(guild_of(id), Some(guild));
            assert_eq!(guild_of(channel_id(id, 3)), Some(guild));
            assert_eq!(guild_of(role_id(id, 3)), Some(guild));
            assert_eq!(guild_of(user_id(id, 500_000)), Some(guild));
        }
        assert_eq!(guild_of(SYNTHETIC_ID_FLOOR - 1), None);
        assert_eq!(guild_of(SYNTHETIC_ID_FLOOR), None);
        assert!(guild_id(MAX_GUILD) + GUILD_STRIDE > 0);
    }
}
