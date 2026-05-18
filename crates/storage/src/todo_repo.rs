use std::sync::Arc;

use sqlx::Row;
use time::OffsetDateTime;
use todo_domain::{NewTodo, Todo, TodoId, TodoUpdate, UserId};
use uuid::Uuid;

use crate::{DbPool, StorageError};

#[derive(Clone)]
pub struct TodoRepository {
    pool: Arc<DbPool>,
}

impl TodoRepository {
    #[must_use]
    pub fn new(pool: Arc<DbPool>) -> Self {
        Self { pool }
    }

    #[tracing::instrument(skip(self), fields(user_id = %user_id))]
    pub async fn list_for_user(&self, user_id: UserId) -> Result<Vec<Todo>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, owner_id, title, completed, created_at, updated_at \
             FROM todos \
             WHERE owner_id = $1 \
             ORDER BY completed ASC, created_at DESC",
        )
        .bind(user_id.0)
        .fetch_all(&*self.pool)
        .await?;
        Ok(rows.iter().map(row_to_todo).collect())
    }

    #[tracing::instrument(skip(self, new), fields(user_id = %user_id))]
    pub async fn create(&self, user_id: UserId, new: NewTodo) -> Result<Todo, StorageError> {
        let id = TodoId::new();
        let title = new.title.trim().to_owned();
        let row = sqlx::query(
            "INSERT INTO todos (id, owner_id, title) \
             VALUES ($1, $2, $3) \
             RETURNING id, owner_id, title, completed, created_at, updated_at",
        )
        .bind(id.0)
        .bind(user_id.0)
        .bind(&title)
        .fetch_one(&*self.pool)
        .await?;
        Ok(row_to_todo(&row))
    }

    #[tracing::instrument(skip(self), fields(user_id = %user_id, todo_id = %id))]
    pub async fn get(&self, user_id: UserId, id: TodoId) -> Result<Todo, StorageError> {
        let row = sqlx::query(
            "SELECT id, owner_id, title, completed, created_at, updated_at \
             FROM todos \
             WHERE id = $1 AND owner_id = $2",
        )
        .bind(id.0)
        .bind(user_id.0)
        .fetch_optional(&*self.pool)
        .await?;
        row.as_ref().map(row_to_todo).ok_or(StorageError::NotFound)
    }

    #[tracing::instrument(skip(self, update), fields(user_id = %user_id, todo_id = %id))]
    pub async fn update(
        &self,
        user_id: UserId,
        id: TodoId,
        update: TodoUpdate,
    ) -> Result<Todo, StorageError> {
        let title = update.title.map(|t| t.trim().to_owned());
        let completed = update.completed;
        let row = sqlx::query(
            "UPDATE todos \
             SET title     = COALESCE($3, title), \
                 completed = COALESCE($4, completed) \
             WHERE id = $1 AND owner_id = $2 \
             RETURNING id, owner_id, title, completed, created_at, updated_at",
        )
        .bind(id.0)
        .bind(user_id.0)
        .bind(title)
        .bind(completed)
        .fetch_optional(&*self.pool)
        .await?;
        row.as_ref().map(row_to_todo).ok_or(StorageError::NotFound)
    }

    #[tracing::instrument(skip(self), fields(user_id = %user_id, todo_id = %id))]
    pub async fn toggle(&self, user_id: UserId, id: TodoId) -> Result<Todo, StorageError> {
        let row = sqlx::query(
            "UPDATE todos SET completed = NOT completed \
             WHERE id = $1 AND owner_id = $2 \
             RETURNING id, owner_id, title, completed, created_at, updated_at",
        )
        .bind(id.0)
        .bind(user_id.0)
        .fetch_optional(&*self.pool)
        .await?;
        row.as_ref().map(row_to_todo).ok_or(StorageError::NotFound)
    }

    #[tracing::instrument(skip(self), fields(user_id = %user_id, todo_id = %id))]
    pub async fn delete(&self, user_id: UserId, id: TodoId) -> Result<(), StorageError> {
        let result = sqlx::query("DELETE FROM todos WHERE id = $1 AND owner_id = $2")
            .bind(id.0)
            .bind(user_id.0)
            .execute(&*self.pool)
            .await?;
        if result.rows_affected() == 0 {
            Err(StorageError::NotFound)
        } else {
            Ok(())
        }
    }
}

fn row_to_todo(row: &sqlx::postgres::PgRow) -> Todo {
    let id: Uuid = row.get("id");
    let owner_id: Uuid = row.get("owner_id");
    let title: String = row.get("title");
    let completed: bool = row.get("completed");
    let created_at: OffsetDateTime = row.get("created_at");
    let updated_at: OffsetDateTime = row.get("updated_at");
    Todo {
        id: TodoId(id),
        owner_id: UserId(owner_id),
        title,
        completed,
        created_at,
        updated_at,
    }
}
