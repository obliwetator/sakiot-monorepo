//! The id range reserved for synthetic load-test data.
//!
//! The load-test harness (`loadtest` feature, `src/loadtest/`) seeds guilds,
//! channels, roles, users and recordings whose Discord ids all lie at or
//! above [`SYNTHETIC_ID_FLOOR`]. Real Discord snowflakes encode a millisecond
//! timestamp since 2015 in their top 42 bits; Discord reaches this range
//! around 2075, so it does not overlap real data.
//!
//! This module is compiled into every build, production included, because
//! code that must leave synthetic rows alone (the media archive never uploads
//! them) runs everywhere.

/// The lowest synthetic id. Everything at or above it was made by the
/// load-test harness.
pub const SYNTHETIC_ID_FLOOR: i64 = 8_000_000_000_000_000_000;

pub fn is_synthetic(id: i64) -> bool {
    id >= SYNTHETIC_ID_FLOOR
}
