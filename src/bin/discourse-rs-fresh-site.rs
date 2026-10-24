//! Prints the SQL that secures a database just loaded from seed/*.sql
//! (src/fresh_site.rs), for running in the same transaction as the load.
//! The NixOS module's discourse-rs-db-init runs it.
//!
//!   INITIAL_ADMIN_PASSWORD=... [INITIAL_ADMIN_EMAIL=...] \
//!     discourse-rs-fresh-site <base-url> > secure.sql
//!
//! The password comes from the environment, not the command line, so it
//! never shows in the process list.

use std::process::ExitCode;

use discourse_rs::fresh_site::{FreshSite, secure_seed_sql};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (Some(base_url), None) = (args.next(), args.next()) else {
        eprintln!("usage: discourse-rs-fresh-site <base-url>");
        return ExitCode::from(64);
    };
    let Some(password) = std::env::var("INITIAL_ADMIN_PASSWORD")
        .ok()
        .filter(|p| !p.is_empty())
    else {
        eprintln!("discourse-rs-fresh-site: INITIAL_ADMIN_PASSWORD is not set");
        return ExitCode::FAILURE;
    };
    let email = std::env::var("INITIAL_ADMIN_EMAIL")
        .ok()
        .filter(|e| !e.trim().is_empty());
    match secure_seed_sql(&FreshSite {
        admin_password: &password,
        admin_email: email.as_deref(),
        base_url: &base_url,
    }) {
        Ok(sql) => {
            print!("{sql}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("discourse-rs-fresh-site: {e}");
            ExitCode::FAILURE
        }
    }
}
