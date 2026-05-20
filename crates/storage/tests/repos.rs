//! Integration tests for UserRepository and TodoRepository.
//! Requires Docker (testcontainers spins up an ephemeral Postgres per test).

use std::{sync::Arc, time::Instant};

use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use todo_domain::{NewTodo, NewUser, TodoUpdate};
use todo_storage::{
    pool::{build_pool, DbPoolConfig},
    run_migrations, StorageError, TodoRepository, UserRepository,
};

async fn fixture() -> (testcontainers::ContainerAsync<Postgres>, Arc<sqlx::PgPool>) {
    let container = Postgres::default()
        .with_db_name("todo")
        .with_user("todo")
        .with_password("todo")
        .start()
        .await
        .expect("start postgres");
    let host_port = container.get_host_port_ipv4(5432).await.expect("host port");
    let cfg = DbPoolConfig {
        url: format!("postgres://todo:todo@127.0.0.1:{host_port}/todo"),
        max_connections: 4,
        min_connections: 1,
        acquire_timeout_secs: 10,
    };
    let pool = build_pool(&cfg).await.expect("build pool");
    run_migrations(&pool).await.expect("migrate");
    (container, Arc::new(pool))
}

#[tokio::test]
async fn user_signup_duplicate_email_returns_conflict() {
    let (_c, pool) = fixture().await;
    let repo = UserRepository::new(pool);

    let new = NewUser {
        email: "Alice@Example.com".into(),
        password: "correcthorsebattery".into(),
    };
    let user = repo.create(new.clone()).await.expect("create");
    assert_eq!(user.email, "Alice@Example.com");

    let err = repo
        .create(NewUser {
            email: "alice@example.com".into(),
            password: "another-strong-pw".into(),
        })
        .await
        .expect_err("duplicate email should conflict");
    assert!(matches!(err, StorageError::Conflict(_)), "got {err:?}");
}

#[tokio::test]
async fn user_verify_wrong_password_returns_none() {
    let (_c, pool) = fixture().await;
    let repo = UserRepository::new(pool);
    let _ = repo
        .create(NewUser {
            email: "bob@example.com".into(),
            password: "correcthorsebattery".into(),
        })
        .await
        .unwrap();

    let result = repo.verify("bob@example.com", "wrong").await.unwrap();
    assert!(result.is_none());

    let ok = repo
        .verify("bob@example.com", "correcthorsebattery")
        .await
        .unwrap();
    assert!(ok.is_some());
}

#[tokio::test]
async fn user_verify_unknown_email_is_timing_equalized() {
    let (_c, pool) = fixture().await;
    let repo = UserRepository::new(pool);
    let _ = repo
        .create(NewUser {
            email: "carol@example.com".into(),
            password: "the-right-passphrase".into(),
        })
        .await
        .unwrap();

    // Warm-up to pay any first-call cost.
    let _ = repo.verify("carol@example.com", "warmup").await.unwrap();
    let _ = repo.verify("unknown@example.com", "warmup").await.unwrap();

    // Sample N times and compare medians. Both paths should spend the same
    // CPU time on argon2 (the unknown path runs against a dummy hash to
    // equalize timing). We compare the ratio rather than an absolute bound
    // so the test is robust to runner speed — a fast laptop and a slow CI
    // runner both produce ratios near 1.0 when the defense is intact.
    // A dropped dummy hash makes the unknown path skip argon2 entirely,
    // producing a ~50x ratio; the 2x bound below comfortably catches that.
    const SAMPLES: usize = 5;
    let mut known = Vec::with_capacity(SAMPLES);
    let mut unknown = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let t = Instant::now();
        let _ = repo.verify("carol@example.com", "wrong").await.unwrap();
        known.push(t.elapsed());
        let t = Instant::now();
        let _ = repo.verify("ghost@example.com", "wrong").await.unwrap();
        unknown.push(t.elapsed());
    }
    known.sort();
    unknown.sort();
    let med_known = known[SAMPLES / 2];
    let med_unknown = unknown[SAMPLES / 2];

    let (smaller, larger) = if med_known <= med_unknown {
        (med_known, med_unknown)
    } else {
        (med_unknown, med_known)
    };
    let ratio = larger.as_secs_f64() / smaller.as_secs_f64();
    assert!(
        ratio < 2.0,
        "timing diverged: known median={med_known:?}, unknown median={med_unknown:?}, ratio={ratio:.2}"
    );
}

#[tokio::test]
async fn todos_isolated_per_user() {
    let (_c, pool) = fixture().await;
    let users = UserRepository::new(pool.clone());
    let todos = TodoRepository::new(pool);

    let alice = users
        .create(NewUser {
            email: "alice@todos.test".into(),
            password: "verylongpasswordhere".into(),
        })
        .await
        .unwrap();
    let bob = users
        .create(NewUser {
            email: "bob@todos.test".into(),
            password: "anotherlongpassword".into(),
        })
        .await
        .unwrap();

    let t = todos
        .create(
            alice.id,
            NewTodo {
                title: "alice todo".into(),
            },
        )
        .await
        .unwrap();
    let _ = todos
        .create(
            bob.id,
            NewTodo {
                title: "bob todo".into(),
            },
        )
        .await
        .unwrap();

    let alice_list = todos.list_for_user(alice.id).await.unwrap();
    assert_eq!(alice_list.len(), 1);
    assert_eq!(alice_list[0].title, "alice todo");

    // Bob cannot get Alice's todo.
    let err = todos.get(bob.id, t.id).await.expect_err("not allowed");
    assert!(matches!(err, StorageError::NotFound));

    let err = todos.delete(bob.id, t.id).await.expect_err("not allowed");
    assert!(matches!(err, StorageError::NotFound));
}

#[tokio::test]
async fn update_preferences_persists_and_clears() {
    let (_c, pool) = fixture().await;
    let users = UserRepository::new(pool);

    let user = users
        .create(NewUser {
            email: format!("prefs-{}@example.com", uuid::Uuid::new_v4()),
            password: "twelve-chars!".into(),
        })
        .await
        .unwrap();

    // initial: both NULL
    assert_eq!(user.locale, None);
    assert_eq!(user.timezone, None);

    // set both
    users
        .update_preferences(user.id, Some("es"), Some("America/Los_Angeles"))
        .await
        .unwrap();
    let reloaded = users.find_by_id(user.id).await.unwrap().unwrap();
    assert_eq!(reloaded.locale.as_deref(), Some("es"));
    assert_eq!(reloaded.timezone.as_deref(), Some("America/Los_Angeles"));

    // clear locale only (empty string == clear); leave timezone
    users
        .update_preferences(user.id, Some(""), None)
        .await
        .unwrap();
    let reloaded = users.find_by_id(user.id).await.unwrap().unwrap();
    assert_eq!(reloaded.locale, None);
    assert_eq!(reloaded.timezone.as_deref(), Some("America/Los_Angeles"));

    // both None: skip — neither column should change.
    users.update_preferences(user.id, None, None).await.unwrap();
    let reloaded = users.find_by_id(user.id).await.unwrap().unwrap();
    assert_eq!(reloaded.locale, None);
    assert_eq!(reloaded.timezone.as_deref(), Some("America/Los_Angeles"));
}

#[tokio::test]
async fn todo_update_and_toggle() {
    let (_c, pool) = fixture().await;
    let users = UserRepository::new(pool.clone());
    let todos = TodoRepository::new(pool);

    let user = users
        .create(NewUser {
            email: "dave@todos.test".into(),
            password: "verylongpasswordhere".into(),
        })
        .await
        .unwrap();

    let t = todos
        .create(
            user.id,
            NewTodo {
                title: "draft".into(),
            },
        )
        .await
        .unwrap();
    assert!(!t.completed);

    let toggled = todos.toggle(user.id, t.id).await.unwrap();
    assert!(toggled.completed);

    let updated = todos
        .update(
            user.id,
            t.id,
            TodoUpdate {
                title: Some("final".into()),
                completed: Some(false),
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.title, "final");
    assert!(!updated.completed);
}
