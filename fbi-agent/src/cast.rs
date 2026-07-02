//! Boundary cast for storing Discord `u64` values in postgres `BIGINT` (`i64`).
//!
//! Postgres has no unsigned 64-bit integer and sqlx only encodes `i64` for
//! `BIGINT`, so every Discord id / ssrc / ms-timestamp / permission-bitset must
//! be reinterpreted as `i64` at the query boundary. The cast is lossless for all
//! values stored here (snowflakes and ms timestamps stay below `2^63`).

/// Reinterpret a `u64`-convertible value as `i64` for a postgres `BIGINT` bind.
///
/// Implemented for every `Into<u64>` source: serenity id types (`GuildId`,
/// `UserId`, `ChannelId`, `RoleId`, …), raw `u64`, and `u32` (ssrc).
pub trait ToI64 {
    fn to_i64(self) -> i64;
}

impl<T: Into<u64>> ToI64 for T {
    fn to_i64(self) -> i64 {
        let v: u64 = self.into(); // explicit annotation avoids Into inference ambiguity
        v as i64
    }
}
