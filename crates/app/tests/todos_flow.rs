//! End-to-end todo flow: create, toggle, delete, isolation between users.

mod common;

use common::spawn;
use scraper::{Html, Selector};

async fn signup(app: &common::TestServer, email: &str, password: &str) {
    let res = app
        .client
        .post(format!("{}/signup", app.base_url))
        .form(&[("email", email), ("password", password)])
        .send()
        .await
        .unwrap();
    assert!(
        res.status().is_redirection() || res.status() == 200,
        "signup failed: {}",
        res.status()
    );
}

#[tokio::test]
async fn create_toggle_delete_roundtrip() {
    let app = spawn().await;
    signup(&app, "hank@todos.test", "verylongsecret123").await;

    // Create
    let create = app
        .client
        .post(format!("{}/todos", app.base_url))
        .form(&[("title", "buy milk")])
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), 201);
    let html = create.text().await.unwrap();
    assert!(html.contains("buy milk"));
    // Extract the inserted id from id="todo-..."
    let id = extract_todo_id(&html);

    // List
    let list = app
        .client
        .get(format!("{}/todos", app.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), 200);

    // Toggle
    let toggle = app
        .client
        .post(format!("{}/todos/{}/toggle", app.base_url, id))
        .send()
        .await
        .unwrap();
    assert_eq!(toggle.status(), 200);
    let toggled_html = toggle.text().await.unwrap();
    assert!(toggled_html.contains("buy milk"));

    // Delete
    let del = app
        .client
        .delete(format!("{}/todos/{}", app.base_url, id))
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 200);
}

#[tokio::test]
async fn one_user_cannot_delete_anothers_todo() {
    let app = spawn().await;
    signup(&app, "ivy@todos.test", "verylongsecret123").await;

    let create = app
        .client
        .post(format!("{}/todos", app.base_url))
        .form(&[("title", "ivy's todo")])
        .send()
        .await
        .unwrap();
    let id = extract_todo_id(&create.text().await.unwrap());

    // Logout, sign up as a different user.
    let _ = app
        .client
        .post(format!("{}/logout", app.base_url))
        .send()
        .await
        .unwrap();
    signup(&app, "jack@todos.test", "verylongsecret123").await;

    let del = app
        .client
        .delete(format!("{}/todos/{}", app.base_url, id))
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 404);
}

fn extract_todo_id(html: &str) -> String {
    let doc = Html::parse_fragment(html);
    let sel = Selector::parse("[id^='todo-']").unwrap();
    let el = doc.select(&sel).next().expect("no todo element");
    el.value()
        .id()
        .unwrap()
        .strip_prefix("todo-")
        .unwrap()
        .to_owned()
}
