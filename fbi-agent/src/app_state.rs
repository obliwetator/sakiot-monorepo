use std::sync::Arc;

use serenity::{client::Cache, prelude::*};

#[derive(Clone)]
pub struct Custom {
    pub(crate) cache: Arc<Cache>,
    pub(crate) data: Arc<RwLock<TypeMap>>,
    pub(crate) pool: sqlx::Pool<sqlx::Postgres>,
    pub(crate) jam_cooldown: crate::cooldown::JamCooldown,
    pub(crate) runtime: Arc<crate::runtime::RuntimeState>,
    pub(crate) media_archive: crate::media_archive::MediaArchive,
    pub(crate) projections: Arc<crate::projections::Projections>,
}

impl Custom {
    pub(crate) fn new(
        cache: Arc<Cache>,
        data: Arc<RwLock<TypeMap>>,
        pool: sqlx::Pool<sqlx::Postgres>,
        jam_cooldown: crate::cooldown::JamCooldown,
        runtime: Arc<crate::runtime::RuntimeState>,
        media_archive: crate::media_archive::MediaArchive,
        projections: Arc<crate::projections::Projections>,
    ) -> Self {
        Self {
            cache,
            data,
            pool,
            jam_cooldown,
            runtime,
            media_archive,
            projections,
        }
    }
}
