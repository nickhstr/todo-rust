//! axum-login backend wrapping the user repository.

use async_trait::async_trait;
use axum_login::{AuthUser, AuthnBackend, UserId as LoginUserId};
use serde::{Deserialize, Serialize};
use todo_domain::{Credentials, User, UserId};
use todo_storage::{StorageError, UserRepository};
use uuid::Uuid;

/// `axum-login`'s `AuthUser` requires us to implement on the user record.
/// We can't impl on the foreign `todo_domain::User`, so we wrap it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserRecord(pub User);

impl AuthUser for AuthUserRecord {
    type Id = Uuid;

    fn id(&self) -> Self::Id {
        self.0.id.0
    }

    fn session_auth_hash(&self) -> &[u8] {
        // tower-sessions invalidates the session if this changes (e.g. password reset).
        self.0.password_hash.as_bytes()
    }
}

/// Login credentials carried through axum-login.
#[derive(Debug, Clone)]
pub struct LoginCredentials {
    pub email: String,
    pub password: String,
}

impl From<&Credentials> for LoginCredentials {
    fn from(c: &Credentials) -> Self {
        Self {
            email: c.email.clone(),
            password: c.password.clone(),
        }
    }
}

#[derive(Clone)]
pub struct AuthBackend {
    users: UserRepository,
}

impl AuthBackend {
    pub fn new(users: UserRepository) -> Self {
        Self { users }
    }
}

#[async_trait]
impl AuthnBackend for AuthBackend {
    type User = AuthUserRecord;
    type Credentials = LoginCredentials;
    type Error = StorageError;

    async fn authenticate(
        &self,
        creds: Self::Credentials,
    ) -> Result<Option<Self::User>, Self::Error> {
        let opt = self.users.verify(&creds.email, &creds.password).await?;
        Ok(opt.map(AuthUserRecord))
    }

    async fn get_user(
        &self,
        user_id: &LoginUserId<Self>,
    ) -> Result<Option<Self::User>, Self::Error> {
        let opt = self.users.find_by_id(UserId(*user_id)).await?;
        Ok(opt.map(AuthUserRecord))
    }
}

/// Convenient alias used throughout handlers.
pub type AuthSession = axum_login::AuthSession<AuthBackend>;

/// Pulls the current `UserId` out of an `AuthSession`, or `Unauthorized`.
pub fn require_user(auth: &AuthSession) -> Result<UserId, crate::AppError> {
    auth.user
        .as_ref()
        .map(|u| u.0.id)
        .ok_or(crate::AppError::Unauthorized)
}
