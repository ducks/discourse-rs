use crate::common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::body::Body;
use axum::http::Request;
use axum::routing::get;

use common::TestDb;
use sqlx::postgres::PgPoolOptions;
use sqlx::{Acquire, PgPool};
use tokio::sync::Notify;
use tower::ServiceExt;

/// The pool's one connection is in a transaction block it never committed:
/// its transaction started before this statement did.
async fn in_open_transaction(pool: &PgPool) -> bool {
    sqlx::query_scalar(
        "SELECT xact_start IS DISTINCT FROM query_start FROM pg_stat_activity WHERE pid = pg_backend_pid()",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Why requests run to completion: sqlx 0.8 queues BEGIN before it counts
/// the transaction, so a `begin()` dropped while it waits (a handler whose
/// client went away) rolls nothing back, and the release ping sends the
/// BEGIN. The connection goes back to the pool in a transaction block and
/// the next borrower's writes are never committed (a login's token row,
/// unseen by the next request, which then deletes the cookie).
#[tokio::test]
async fn a_dropped_begin_leaves_the_connection_in_a_transaction() {
    let db = TestDb::new().await;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db.url())
        .await
        .unwrap();
    {
        let mut conn = pool.acquire().await.unwrap();
        // One poll: BEGIN queued, the reply not yet read.
        let mut begin = std::pin::pin!(conn.begin());
        assert!(
            futures_util::poll!(&mut begin).is_pending(),
            "begin finished in one poll"
        );
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(in_open_transaction(&pool).await);
}

/// A handler waiting on `gate`, then marking `done`; the client is gone
/// before the gate opens.
async fn drop_the_client_midway(
    app: axum::Router,
    gate: Arc<Notify>,
    done: Arc<AtomicBool>,
) -> bool {
    let request = Request::get("/").body(Body::empty()).unwrap();
    {
        let mut call = std::pin::pin!(app.oneshot(request));
        assert!(futures_util::poll!(&mut call).is_pending());
    }
    gate.notify_one();
    tokio::time::sleep(Duration::from_millis(200)).await;
    done.load(Ordering::SeqCst)
}

fn waiting_app(gate: &Arc<Notify>, done: &Arc<AtomicBool>) -> axum::Router {
    let (gate, done) = (gate.clone(), done.clone());
    axum::Router::new().route(
        "/",
        get(move || async move {
            gate.notified().await;
            done.store(true, Ordering::SeqCst);
        }),
    )
}

#[tokio::test]
async fn a_request_runs_on_after_its_client_goes_away() {
    let (gate, done) = (Arc::new(Notify::new()), Arc::new(AtomicBool::new(false)));
    let app =
        waiting_app(&gate, &done).layer(axum::middleware::from_fn(discourse_rs::run_to_completion));
    assert!(drop_the_client_midway(app, gate, done).await);

    // Without the layer the handler is dropped with the client.
    let (gate, done) = (Arc::new(Notify::new()), Arc::new(AtomicBool::new(false)));
    assert!(!drop_the_client_midway(waiting_app(&gate, &done), gate, done).await);
}
