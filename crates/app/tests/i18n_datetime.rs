mod common;
use scraper::{Html, Selector};

/// Sign up a fresh account. Returns the server after signing in.
async fn signup_and_create_todo(server: &common::TestServer, email: &str, title: &str) {
    let signup = server
        .client
        .post(format!("{}/signup", server.base_url))
        .form(&[("email", email), ("password", "twelve-chars!")])
        .send()
        .await
        .unwrap();
    assert!(
        signup.status().is_redirection() || signup.status().is_success(),
        "signup failed: {}",
        signup.status()
    );
    let created = server
        .client
        .post(format!("{}/todos", server.base_url))
        .form(&[("title", title)])
        .send()
        .await
        .unwrap();
    let status = created.status();
    let body = created.text().await.unwrap();
    assert_eq!(status, 201, "todo creation failed; body: {}", &body[..body.len().min(500)]);
}

#[tokio::test]
async fn time_element_has_utc_datetime_attribute() {
    let server = common::spawn().await;
    signup_and_create_todo(&server, "datetime@example.com", "test entry").await;

    // GET /todos returns the todo list partial (no redirect, auth'd via cookie).
    let res = server
        .client
        .get(format!("{}/todos", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "GET /todos failed");
    let body = res.text().await.unwrap();
    let doc = Html::parse_document(&body);
    let sel = Selector::parse("time[datetime]").unwrap();
    let times: Vec<_> = doc.select(&sel).collect();
    assert!(!times.is_empty(), "no <time> elements found in: {}", &body[..body.len().min(500)]);

    for el in times {
        let dt = el.value().attr("datetime").unwrap();
        // RFC3339 UTC: ends with Z
        assert!(dt.ends_with('Z'), "datetime attr not UTC: {dt}");
    }
}

#[tokio::test]
async fn server_text_format_changes_with_locale() {
    let server = common::spawn().await;
    signup_and_create_todo(&server, "esdt@example.com", "fecha").await;

    // Use the todo partial responses (POST /todos returns a locale-rendered partial).
    // We also test via GET /todos with Accept-Language — the list is re-rendered per request.
    let en = server
        .client
        .get(format!("{}/todos", server.base_url))
        .header("Accept-Language", "en")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let es = server
        .client
        .get(format!("{}/todos", server.base_url))
        .header("Accept-Language", "es")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    let extract_time_text = |html: &str| -> Option<String> {
        let doc = Html::parse_document(html);
        let sel = Selector::parse("time").unwrap();
        doc.select(&sel)
            .next()
            .map(|el| el.text().collect::<String>())
    };

    let en_text = extract_time_text(&en).expect("no <time> in en response");
    let es_text = extract_time_text(&es).expect("no <time> in es response");
    assert_ne!(
        en_text.trim(),
        es_text.trim(),
        "en and es datetime text should differ; en={en_text}, es={es_text}"
    );
}
