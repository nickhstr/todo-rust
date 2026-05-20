use std::{sync::Arc, time::Duration};

use fred::prelude::RedisPool;
use todo_domain::{Todo, UserId};
use todo_i18n::{Assets, Locales};
use todo_storage::{DbPool, StorageError, TodoRepository, UserRepository};

use crate::{cache::Cache, templates::Templates, Config};

/// Shared, cloneable per-request state. All fields are cheap to clone (Arc'd).
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: Arc<DbPool>,
    pub users: UserRepository,
    pub todos: TodoRepository,
    pub templates: Templates,
    pub cache: Cache,
    pub redis: Option<RedisPool>,
    pub locales: Locales,
    pub assets: Arc<Assets>,
}

impl AppState {
    pub fn new(
        config: Arc<Config>,
        db: Arc<DbPool>,
        templates: Templates,
        cache: Cache,
        redis: Option<RedisPool>,
        locales: Locales,
        assets: Arc<Assets>,
    ) -> Self {
        let users = UserRepository::new(db.clone());
        let todos = TodoRepository::new(db.clone());
        Self {
            config,
            db,
            users,
            todos,
            templates,
            cache,
            redis,
            locales,
            assets,
        }
    }

    fn todos_cache_key(user_id: UserId) -> String {
        format!("todos:user:{}", user_id.0)
    }

    /// Cached list of a user's todos. Cache miss falls through to the DB
    /// and back-fills with the configured default TTL (capped to 60s for
    /// the todos list since it changes often).
    pub async fn list_todos_cached(&self, user_id: UserId) -> Result<Vec<Todo>, StorageError> {
        if !self.cache.is_enabled() {
            return self.todos.list_for_user(user_id).await;
        }
        let key = Self::todos_cache_key(user_id);
        let ttl = Duration::from_secs(60);
        let todos = self.todos.clone();
        self.cache
            .get_or_compute::<Vec<Todo>, _, _, StorageError>(&key, Some(ttl), move || async move {
                todos.list_for_user(user_id).await
            })
            .await
    }

    /// Invalidate after writes so the next read repopulates.
    pub async fn invalidate_todos_cache(&self, user_id: UserId) {
        if !self.cache.is_enabled() {
            return;
        }
        let key = Self::todos_cache_key(user_id);
        self.cache.invalidate(&key).await;
    }
}
