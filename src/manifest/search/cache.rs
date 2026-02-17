use std::num::NonZeroUsize;
use std::sync::Arc;

use moka::sync::Cache as MokaCache;

use crate::manifest::entity::Entity;

pub struct EntityCache {
    cache: MokaCache<String, Arc<Entity>>,
}

impl EntityCache {
    pub const BACKEND_LABEL: &'static str = "moka";

    pub fn build(size: usize) -> Option<Self> {
        let size = NonZeroUsize::new(size)?;
        Some(Self {
            cache: MokaCache::builder().max_capacity(size.get() as u64).build(),
        })
    }

    pub fn get_arc(&self, key: &str) -> Option<Arc<Entity>> {
        self.cache.get(key)
    }

    pub fn insert_arc(&self, key: String, entity: Arc<Entity>) {
        self.cache.insert(key, entity);
    }

    pub fn len(&self) -> usize {
        usize::try_from(self.cache.entry_count()).unwrap_or(usize::MAX)
    }
}
