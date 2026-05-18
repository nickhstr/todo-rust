//! Pure domain types for the todo app. No I/O, no web, no SQL.

pub mod todo;
pub mod user;

pub use todo::{NewTodo, Todo, TodoError, TodoId, TodoUpdate};
pub use user::{Credentials, NewUser, User, UserError, UserId};
