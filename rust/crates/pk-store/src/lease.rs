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
    /// Take or renew `lease:<kind>` for `me`: `true` when this process holds it now. A write conflict with another
    /// asker is a `false` (that asker holds it), never an error.
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
        match self.last(&sql, bound).await {
            Ok(rows) => Ok(!rows_of(rows).is_empty()),
            Err(e) if is_conflict(&e) => Ok(false),
            Err(e) => Err(e),
        }
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

/// A transaction conflict between two writers of the row (the engine's words differ by backend).
fn is_conflict(e: &StoreError) -> bool {
    let text = e.to_string().to_lowercase();
    text.contains("can be retried") || text.contains("conflict")
}
