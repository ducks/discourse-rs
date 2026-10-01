//! Replays every case in parity/cases that has a golden file against the
//! in-process router, so `cargo test` catches parity regressions without
//! Rails or a running server.

mod common;

use std::path::Path;

use axum::body::Body;
use axum::http::{Request, header};
use discourse_rs::parity;
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn golden_responses_match() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cases = parity::load_cases(&root.join("parity/cases")).unwrap();
    let golden_dir = root.join("parity/golden");
    let db = common::TestDb::new().await;
    // The golden files came from the Discourse whose database is the test
    // template (seed/fresh_install.sql) and whose env is parity/environment.
    let config = common::recorded_config();
    let app = discourse_rs::app(common::state(db.pool.clone(), config));

    let mut failures = Vec::new();
    let mut checked = 0;
    for case in &cases {
        let Some(golden) = parity::read_golden(&golden_dir, case).unwrap() else {
            eprintln!("skip {} (no golden file)", case.label());
            continue;
        };

        // The host Rails saw when the golden was recorded.
        let host = golden
            .source
            .split("://")
            .nth(1)
            .map(|h| h.trim_end_matches('/').to_string())
            .unwrap_or_else(|| "localhost".into());
        let actual = parity::run_case(case, |exchange: parity::Exchange| {
            let app = app.clone();
            let host = host.clone();
            async move {
                let mut request = Request::builder()
                    .method(exchange.method.as_str())
                    .uri(&exchange.path)
                    .header(header::HOST, host);
                for (name, value) in &exchange.headers {
                    request = request.header(name.as_str(), value.as_str());
                }
                let body = exchange.body.map(Body::from).unwrap_or_else(Body::empty);
                let response = app.oneshot(request.body(body).unwrap()).await.unwrap();
                let status = response.status().as_u16();
                let content_type = response
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string);
                let set_cookies = response
                    .headers()
                    .get_all(header::SET_COOKIE)
                    .iter()
                    .filter_map(|v| v.to_str().ok())
                    .map(str::to_string)
                    .collect();
                let bytes = response.into_body().collect().await.unwrap().to_bytes();
                Ok(parity::Reply {
                    status,
                    content_type,
                    body: String::from_utf8_lossy(&bytes).into_owned(),
                    set_cookies,
                })
            }
        })
        .await
        .unwrap();

        checked += 1;
        if let Err(report) = parity::compare(case, &golden.response, &actual, ("golden", "rs")) {
            failures.push(format!("{}\n{report}", case.label()));
        }
    }

    assert!(
        checked > 0,
        "no golden files found in {}",
        golden_dir.display()
    );
    assert!(
        failures.is_empty(),
        "{} of {checked} cases mismatched:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
