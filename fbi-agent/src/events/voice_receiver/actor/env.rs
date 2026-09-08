//! The slice of the Discord client the recorder actor actually needs.
//!
//! `RecorderActor` used to hold a whole `Arc<Context>`, which made its run loop
//! impossible to drive in a test: a serenity `Context` cannot be built without a
//! live shard and client. Narrowing the dependency to the three fields the actor
//! reads lets tests spawn a real actor against an empty cache and a dummy HTTP
//! client, so the run loop's own behavior can be asserted.

use std::sync::Arc;

use serenity::{
    cache::Cache,
    client::Context,
    http::Http,
    prelude::{RwLock, TypeMap},
};

#[derive(Clone)]
pub(in crate::events::voice_receiver) struct RecorderEnv {
    pub data: Arc<RwLock<TypeMap>>,
    pub cache: Arc<Cache>,
    pub http: Arc<Http>,
}

impl RecorderEnv {
    pub(in crate::events::voice_receiver) fn from_ctx(ctx: &Context) -> Self {
        Self {
            data: Arc::clone(&ctx.data),
            cache: Arc::clone(&ctx.cache),
            http: Arc::clone(&ctx.http),
        }
    }

    /// An environment for tests: caller-supplied data, empty cache, and an HTTP
    /// client that is never used because tests do not reach the network.
    #[cfg(test)]
    pub(in crate::events::voice_receiver) fn for_test(data: Arc<RwLock<TypeMap>>) -> Self {
        Self {
            data,
            cache: Arc::new(Cache::new()),
            http: Arc::new(Http::new("Bot test")),
        }
    }
}
