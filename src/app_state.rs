use std::{collections::HashMap, sync::Arc};

use serenity::{client::Cache, http::Http, prelude::*};

pub struct HasBossMusic;
impl TypeMapKey for HasBossMusic {
    type Value = HashMap<u64, Option<String>>;
}

#[derive(Clone)]
pub struct Custom {
    pub(crate) cache: Arc<Cache>,
    pub(crate) _http: Arc<Http>,
    pub(crate) data: Arc<RwLock<TypeMap>>,
    pub(crate) pool: sqlx::Pool<sqlx::Postgres>,
    pub(crate) jam_cooldown: crate::cooldown::JamCooldown,
    pub(crate) runtime: Arc<crate::runtime::RuntimeState>,
}

impl Custom {
    pub(crate) fn new(
        cache: Arc<Cache>,
        http: Arc<Http>,
        data: Arc<RwLock<TypeMap>>,
        pool: sqlx::Pool<sqlx::Postgres>,
        jam_cooldown: crate::cooldown::JamCooldown,
        runtime: Arc<crate::runtime::RuntimeState>,
    ) -> Self {
        Self {
            cache,
            _http: http,
            data,
            pool,
            jam_cooldown,
            runtime,
        }
    }
}
