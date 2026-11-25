//! A badge's page (badges/show) past its first page of grants:
//! BadgesShowController#canLoadMore and loadMore.

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn page(st: &AppState, path: &str) -> (StatusCode, String) {
    let response = discourse_rs::app(st.clone())
        .oneshot(
            Request::get(path)
                .header(header::HOST, "test.localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// 100 more grants of Basic User than the seed's: the first page draws 96
/// with the spinner and the next offset, no footer, and badge-page.js; the
/// page at that offset draws the rest and the empty spinner.
#[tokio::test]
async fn a_badge_with_more_grants_than_a_page_loads_the_rest() {
    let db = TestDb::new().await;
    sqlx::query(
        "INSERT INTO user_badges (badge_id, user_id, granted_at, granted_by_id, seq, created_at) \
         SELECT 1, 1, now() - n * interval '1 minute', -1, 100 + n, now() \
         FROM generate_series(1, 100) n",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE badges SET grant_count = (SELECT count(*) FROM user_badges WHERE badge_id = 1) WHERE id = 1")
        .execute(&db.pool)
        .await
        .unwrap();
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM user_badges WHERE badge_id = 1")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;

    let (status, html) = page(&st, "/badges/1/basic-user").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert_eq!(html.matches("user-info medium badge-info").count(), 96);
    assert!(
        html.contains(
            "<div aria-hidden=\"true\" class=\"load-more-sentinel\" data-next-offset=\"96\"></div>"
        ),
        "{html}"
    );
    assert!(html.contains(
        "<div class=\"loading-container visible\" data-loading=\"true\"><div class=\"spinner \"></div></div>"
    ));
    assert!(html.contains("/badge-page.js"));

    let (status, html) = page(&st, "/badges/1/basic-user?offset=96").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert_eq!(
        html.matches("user-info medium badge-info").count() as i64,
        total - 96
    );
    assert!(!html.contains("data-next-offset"), "{html}");
    assert!(html.contains("<div class=\"loading-container\"></div>"));
    assert!(!html.contains("/badge-page.js"));
}
