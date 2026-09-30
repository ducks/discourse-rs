//! Replays every case in parity/cases that has a golden file against the
//! in-process router, so `cargo test` catches parity regressions without
//! Rails or a running server.

mod common;

use std::path::Path;

use axum::body::Body;
use axum::http::{Request, header};
use discourse_rs::parity::{self, Recorded};
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

        let mut request = Request::builder()
            .method(case.method.as_str())
            .uri(&case.path);
        for (name, value) in parity::REQUEST_HEADERS {
            request = request.header(*name, *value);
        }
        // The host Rails saw when the golden was recorded.
        if let Some(host) = golden.source.split("://").nth(1) {
            request = request.header(header::HOST, host.trim_end_matches('/'));
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();

        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let actual = Recorded {
            status,
            content_type,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        };

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
