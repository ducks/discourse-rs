//! The setup wizard's page (/wizard, /wizard/steps/:id): the step over
//! the site skeleton without the site header, and the exception page a
//! member's or a disabled wizard's load ends on. The statuses are covered
//! by parity/writes/wizard/page_loads_*.

use crate::common;

use common::chat_client::Client;
use common::{TestDb, config, set_setting, state};
use discourse_rs::config::RailsEnv;

const BOOL: i32 = 5;

#[tokio::test]
async fn the_admin_gets_the_setup_step() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st, "admin").await;
    for path in ["/wizard", "/wizard/steps/setup", "/wizard/steps/nope"] {
        let (status, html) = admin.page(path).await;
        assert_eq!(status, 200, "{path}: {html}");
        assert!(
            html.contains(
                "<div id=\"wizard-main\"><div aria-hidden=\"true\" class=\"site-skeleton\">"
            ),
            "{path}"
        );
        assert!(
            html.contains("<div class=\"wizard-container__step setup\">"),
            "{path}"
        );
        assert!(html.contains("data-step-id=\"setup\""));
        // The wizard route: no site header, sidebar or footer.
        assert!(!html.contains("class=\"d-header\""), "{path}");
        assert!(!html.contains("powered-by-discourse"), "{path}");
        assert!(html.contains(" wizard\""), "{path}: the body's class");
        assert!(html.contains("/wizard.css"));
        assert!(html.contains("/wizard.js"));
    }
    let (_, html) = admin.page("/wizard/steps/setup").await;
    // The title field, required.
    assert!(html.contains("class=\"wizard-container__field text-field text-title\" data-field-id=\"title\" data-field-type=\"text\" data-required>"), "{html}");
    assert!(html.contains("<span class=\"wizard-container__label required\">*</span>"));
    assert!(html.contains(
        "id=\"title\" class=\"ember-text-field ember-view wizard-container__text-input\""
    ));
    // The radios, their values selected.
    assert!(
        html.contains(
            "<div class=\"wizard-container__radio-choice --selected\" data-choice-id=\"public\">"
        ),
        "{html}"
    );
    assert!(html.contains("<input checked class=\"wizard-container__radio\" name=\"login_required\" type=\"radio\" value=\"public\">"));
    assert!(html.contains("data-choice-id=\"private\"><label class=\"wizard-container__label\"><input class=\"wizard-container__radio\" name=\"login_required\""));
    // The locale dropdown lists the site's locales.
    assert!(html.contains("wizard-container__dropdown"), "{html}");
    assert!(html.contains("data-value=\"fr\""), "{html}");
    let footer = &html[html.find("jump-in").unwrap()..][..160];
    assert!(
        footer.contains("Let") && footer.contains("s go!</button>"),
        "{footer}"
    );
    // The JSON stays the wizard's.
    let (status, body) = admin.get("/wizard.json").await;
    assert_eq!(status, 200);
    assert_eq!(body["wizard"]["start"], "setup");
}

#[tokio::test]
async fn a_member_or_a_disabled_wizard_gets_the_exception_page() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut member = Client::logged_in(st.clone(), "user1").await;
    let (status, html) = member.page("/wizard/steps/setup").await;
    assert_eq!(status, 200, "{html}");
    assert!(html.contains("<div class=\"error-page\">"), "{html}");
    assert!(html.contains("href=\"/wizard.json\""));
    assert!(
        html.contains("class=\"d-header\""),
        "the app's header stays"
    );
    assert!(!html.contains("wizard-main"));
    set_setting(&db.pool, "wizard_enabled", BOOL, "f").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, html) = admin.page("/wizard").await;
    assert_eq!(status, 200, "{html}");
    assert!(html.contains("<div class=\"error-page\">"), "{html}");
}
