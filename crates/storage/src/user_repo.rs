use std::sync::Arc;

use password_auth::{generate_hash, verify_password};
use sqlx::Row;
use time::OffsetDateTime;
use todo_domain::{NewUser, User, UserId};
use uuid::Uuid;

use crate::{DbPool, StorageError};

/// A precomputed argon2id hash used to equalize timing for unknown-user lookups.
/// Verifying against this consumes roughly the same CPU as a real verify, so the
/// "no such user" path and "wrong password" path take indistinguishable wall time.
/// Verification will always fail.
const TIMING_DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$\
    c29tZXNhbHRzb21lc2FsdA$\
    UJg9rqFq8mqo5Hckj4cVi1NJV4O8e1V5z9eC7tEjxlw";

#[derive(Clone)]
pub struct UserRepository {
    pool: Arc<DbPool>,
}

impl UserRepository {
    #[must_use]
    pub fn new(pool: Arc<DbPool>) -> Self {
        Self { pool }
    }

    #[tracing::instrument(skip(self, new), fields(email = %new.email))]
    pub async fn create(&self, new: NewUser) -> Result<User, StorageError> {
        let id = UserId::new();
        let email = new.email.trim().to_owned();
        let password = new.password;
        // argon2 is CPU-heavy; keep it off the runtime worker thread.
        let hash = tokio::task::spawn_blocking(move || generate_hash(password.as_bytes()))
            .await
            .map_err(StorageError::Join)?;

        let row = sqlx::query(
            "INSERT INTO users (id, email, password_hash) \
             VALUES ($1, $2, $3) \
             RETURNING id, email, password_hash, created_at",
        )
        .bind(id.0)
        .bind(&email)
        .bind(&hash)
        .fetch_one(&*self.pool)
        .await;

        match row {
            Ok(row) => Ok(row_to_user(&row)),
            Err(sqlx::Error::Database(db_err)) if db_err.is_unique_violation() => {
                Err(StorageError::Conflict("email already in use".into()))
            }
            Err(e) => Err(e.into()),
        }
    }

    #[tracing::instrument(skip(self))]
    pub async fn find_by_id(&self, id: UserId) -> Result<Option<User>, StorageError> {
        let row =
            sqlx::query("SELECT id, email, password_hash, created_at FROM users WHERE id = $1")
                .bind(id.0)
                .fetch_optional(&*self.pool)
                .await?;
        Ok(row.as_ref().map(row_to_user))
    }

    /// Returns `Ok(Some(user))` on a matching email+password, `Ok(None)` for both
    /// "no such user" and "wrong password". Time spent in this function is roughly
    /// constant across all three outcomes by performing a dummy verify when the
    /// user is missing.
    #[tracing::instrument(skip(self, password), fields(email = %email))]
    pub async fn verify(&self, email: &str, password: &str) -> Result<Option<User>, StorageError> {
        let row = sqlx::query(
            "SELECT id, email, password_hash, created_at \
             FROM users \
             WHERE LOWER(email) = LOWER($1) \
             LIMIT 1",
        )
        .bind(email.trim())
        .fetch_optional(&*self.pool)
        .await?;

        let (stored_hash, found_user) = match row.as_ref() {
            Some(row) => {
                let hash: String = row.try_get("password_hash")?;
                (hash, Some(row_to_user(row)))
            }
            None => (TIMING_DUMMY_HASH.to_owned(), None),
        };

        let password = password.to_owned();
        let ok = tokio::task::spawn_blocking(move || {
            verify_password(password.as_bytes(), &stored_hash).is_ok()
        })
        .await
        .map_err(StorageError::Join)?;

        Ok(if ok { found_user } else { None })
    }
}

fn row_to_user(row: &sqlx::postgres::PgRow) -> User {
    let id: Uuid = row.get("id");
    let email: String = row.get("email");
    let password_hash: String = row.get("password_hash");
    let created_at: OffsetDateTime = row.get("created_at");
    User {
        id: UserId(id),
        email,
        password_hash,
        created_at,
    }
}
