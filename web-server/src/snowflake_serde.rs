//! Serde adapter for Discord snowflake ids.
//!
//! Discord snowflakes exceed 2^53, so ids must travel as decimal strings in
//! JSON: a JSON number would be rounded by the browser before the value could
//! be sent back. `SnowflakeAsStr` serializes an i64 through its `Display`
//! implementation and parses it back on deserialize, so handlers keep working
//! with plain i64s while the wire format stays string-typed.

use serde_with::{As, DisplayFromStr};

pub type SnowflakeAsStr = As<DisplayFromStr>;
