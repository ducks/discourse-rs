//! Where a service runs its queries: on the pool, each step on a
//! connection of its own and its transaction committed on its own, or on
//! a caller's connection, whose enclosing transaction (Rails' outer
//! `transaction do`) then holds everything the service writes, its own
//! transaction a savepoint inside it.

use std::ops::{Deref, DerefMut};

use sqlx::pool::PoolConnection;
use sqlx::{Connection, PgConnection, PgPool, Postgres, Transaction};

pub enum Db<'c> {
    Pool(&'c PgPool),
    Conn(&'c mut PgConnection),
}

/// A connection from `Db::acquire`: the pool's, returned when dropped, or
/// the caller's, borrowed.
pub enum Lease<'a> {
    Pooled(PoolConnection<Postgres>),
    Borrowed(&'a mut PgConnection),
}

impl Deref for Lease<'_> {
    type Target = PgConnection;

    fn deref(&self) -> &PgConnection {
        match self {
            Lease::Pooled(c) => c,
            Lease::Borrowed(c) => c,
        }
    }
}

impl DerefMut for Lease<'_> {
    fn deref_mut(&mut self) -> &mut PgConnection {
        match self {
            Lease::Pooled(c) => c,
            Lease::Borrowed(c) => c,
        }
    }
}

impl<'c> Db<'c> {
    /// A connection for the next queries.
    pub async fn acquire(&mut self) -> Result<Lease<'_>, sqlx::Error> {
        Ok(match self {
            Db::Pool(pool) => Lease::Pooled(pool.acquire().await?),
            Db::Conn(conn) => Lease::Borrowed(conn),
        })
    }

    /// A transaction of the service's own: BEGIN on the pool, a savepoint
    /// on the caller's connection when it is in one.
    pub async fn begin(&mut self) -> Result<Transaction<'_, Postgres>, sqlx::Error> {
        match self {
            Db::Pool(pool) => pool.begin().await,
            Db::Conn(conn) => conn.begin().await,
        }
    }

    /// The same place, for a nested service.
    pub fn reborrow(&mut self) -> Db<'_> {
        match self {
            Db::Pool(pool) => Db::Pool(pool),
            Db::Conn(conn) => Db::Conn(conn),
        }
    }

    /// The pool, for the work that stays on connections of its own: this
    /// one's, or `fallback` (the site's) on a caller's connection.
    pub fn pool_or<'p>(&'p self, fallback: &'p PgPool) -> &'p PgPool {
        match self {
            Db::Pool(pool) => pool,
            Db::Conn(_) => fallback,
        }
    }
}
