//! One owner per background task over the database (`db/schema_extra.surql` `lease`): `lease:<kind>` is taken for
//! `ttl_s` seconds by the first process to ask and renewed by its owner; another process gets it only once it
//! lapsed. One `UPDATE ... WHERE ... RETURN AFTER` statement does the test and the set, on the database's one clock,
//! so two processes asking at once cannot both be told yes: the second write to the row either sees the first
//! (the `WHERE` fails: no rows) or conflicts with it (the engine refuses the commit: no grant either).

use surrealdb::types::{Object, Value};

use crate::error::{Result, StoreError};
use crate::schema::check_ident;
use crate::store::{Store, rows_of, vars};
use crate::value::to_db;

impl Store {
    /// Take or renew `lease:<kind>` for `me`: `true` when this process holds it now. "Already exists" (the loser of two
    /// first-ever creates) is an immediate refusal: that asker holds it. A write conflict is retried after a short
    /// wait of this asker's own length ([`RETRY_WAITS_MS`]), the statement asked again over the rows as they are now:
    /// ours -> renewed (`true`), someone else's and live -> no rows (`false`), still absent -> created again. The create
    /// and the update are two statements, not one transaction, so a create that landed beside eight conflicting
    /// updates used to leave nobody holding it (every asker answered `false`); only a conflict that outlasts the
    /// retries is a `false` for this pass, which the next pass takes up.
    pub async fn lease_acquire(&self, kind: &str, me: &str, ttl_s: u32) -> Result<bool> {
        check_ident(kind, "lease kind")?;
        let sql = format!(
            "IF !record::exists(lease:{kind}) {{ CREATE lease:{kind} CONTENT {{ kind: $k }} }};\
             UPDATE lease:{kind} SET owner = $me, until = time::now() + {ttl_s}s \
             WHERE owner = $me OR until = NONE OR until < time::now() RETURN AFTER;"
        );
        let bound = vars([
            ("k", Value::String(kind.to_string())),
            ("me", Value::String(me.to_string())),
        ]);
        let jitter = me
            .bytes()
            .fold(7_u64, |h, b| h.wrapping_mul(31).wrapping_add(u64::from(b)))
            % 23;
        for wait in RETRY_WAITS_MS.iter().map(|w| Some(w + jitter)).chain([None]) {
            match self.last(&sql, bound.clone()).await {
                Ok(rows) => return Ok(!rows_of(rows).is_empty()),
                Err(e) if is_exists(&e) => return Ok(false),
                Err(e) if is_conflict(&e) => match wait {
                    Some(ms) => tokio::time::sleep(std::time::Duration::from_millis(ms)).await,
                    None => return Ok(false),
                },
                Err(e) => return Err(e),
            }
        }
        Ok(false)
    }

    /// Give `lease:<kind>` up when `me` holds it (a sweeper that cannot run): the next asker takes it at once.
    pub async fn lease_release(&self, kind: &str, me: &str) -> Result<()> {
        check_ident(kind, "lease kind")?;
        self.query(
            &format!("UPDATE lease:{kind} SET owner = NONE, until = NONE WHERE owner = $me;"),
            vars([("me", to_db(&serde_json::json!(me)))]),
        )
        .await?;
        Ok(())
    }

    /// `lease:<kind>` as it is: `(owner, until)`, both none when free or never taken.
    pub async fn lease_state(&self, kind: &str) -> Result<(Option<String>, Option<String>)> {
        check_ident(kind, "lease kind")?;
        let v = self
            .last(
                &format!("SELECT owner, <string> until AS until FROM ONLY lease:{kind};"),
                Object::new(),
            )
            .await?;
        let s = |o: &Object, k: &str| o.get(k).and_then(crate::value::as_str).map(str::to_string);
        Ok(match v {
            Value::Object(o) => (s(&o, "owner"), s(&o, "until")),
            _ => (None, None),
        })
    }
}

/// Waits before the retries of a conflicted acquire, in milliseconds (each plus the asker's own 0-22 ms, so eight
/// askers do not collide again in the same step).
const RETRY_WAITS_MS: [u64; 2] = [15, 40];

/// A transaction conflict between two writers of the row (the engine's words differ by backend).
fn is_conflict(e: &StoreError) -> bool {
    let text = e.to_string().to_lowercase();
    text.contains("can be retried") || text.contains("conflict")
}

/// The loser of two first-ever `CREATE lease:<kind>`: the winner holds the lease.
fn is_exists(e: &StoreError) -> bool {
    e.to_string().to_lowercase().contains("already exists")
}
