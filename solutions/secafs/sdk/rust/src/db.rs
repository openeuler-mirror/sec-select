use crate::error::{Error, Result};
use crate::DatabaseBackend;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio_postgres::types::ToSql;

#[derive(Debug, Clone)]
pub enum DbValue {
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
    Null,
}

impl DbValue {
    pub fn as_integer(&self) -> Option<&i64> {
        match self {
            DbValue::Integer(i) => Some(i),
            _ => None,
        }
    }

    pub fn as_real(&self) -> Option<f64> {
        match self {
            DbValue::Real(f) => Some(*f),
            DbValue::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }
}

impl From<i64> for DbValue {
    fn from(value: i64) -> Self {
        DbValue::Integer(value)
    }
}

impl From<i32> for DbValue {
    fn from(value: i32) -> Self {
        DbValue::Integer(value as i64)
    }
}

impl From<u32> for DbValue {
    fn from(value: u32) -> Self {
        DbValue::Integer(value as i64)
    }
}

impl From<u64> for DbValue {
    fn from(value: u64) -> Self {
        DbValue::Integer(value as i64)
    }
}

impl From<usize> for DbValue {
    fn from(value: usize) -> Self {
        DbValue::Integer(value as i64)
    }
}

impl From<&str> for DbValue {
    fn from(value: &str) -> Self {
        DbValue::Text(value.to_string())
    }
}

impl From<String> for DbValue {
    fn from(value: String) -> Self {
        DbValue::Text(value)
    }
}

impl From<Vec<u8>> for DbValue {
    fn from(value: Vec<u8>) -> Self {
        DbValue::Blob(value)
    }
}

impl From<&[u8]> for DbValue {
    fn from(value: &[u8]) -> Self {
        DbValue::Blob(value.to_vec())
    }
}

pub trait IntoDbArgs {
    fn into_db_args(self) -> Vec<DbValue>;
}

impl IntoDbArgs for () {
    fn into_db_args(self) -> Vec<DbValue> {
        Vec::new()
    }
}

impl IntoDbArgs for Vec<DbValue> {
    fn into_db_args(self) -> Vec<DbValue> {
        self
    }
}

macro_rules! impl_into_db_args_tuple {
    ($($name:ident),+) => {
        impl<$($name),+> IntoDbArgs for ($($name,)+)
        where
            $($name: Into<DbValue>,)+
        {
            fn into_db_args(self) -> Vec<DbValue> {
                let ($($name,)+) = self;
                vec![$($name.into(),)+]
            }
        }
    };
}

impl_into_db_args_tuple!(A);
impl_into_db_args_tuple!(A, B);
impl_into_db_args_tuple!(A, B, C);
impl_into_db_args_tuple!(A, B, C, D);
impl_into_db_args_tuple!(A, B, C, D, E);
impl_into_db_args_tuple!(A, B, C, D, E, F);
impl_into_db_args_tuple!(A, B, C, D, E, F, G);
impl_into_db_args_tuple!(A, B, C, D, E, F, G, H);
impl_into_db_args_tuple!(A, B, C, D, E, F, G, H, I);
impl_into_db_args_tuple!(A, B, C, D, E, F, G, H, I, J);
impl_into_db_args_tuple!(A, B, C, D, E, F, G, H, I, J, K);
impl_into_db_args_tuple!(A, B, C, D, E, F, G, H, I, J, K, L);

#[derive(Debug, Clone)]
pub struct DbRow {
    values: Arc<Vec<DbValue>>,
}

impl DbRow {
    pub fn get_value(&self, index: usize) -> Result<DbValue> {
        self.values
            .get(index)
            .cloned()
            .ok_or_else(|| Error::Internal("row index out of bounds".to_string()))
    }
}

pub struct DbRows {
    rows: Vec<DbRow>,
    index: usize,
}

impl DbRows {
    pub async fn next(&mut self) -> Result<Option<DbRow>> {
        if self.index >= self.rows.len() {
            Ok(None)
        } else {
            let row = self.rows[self.index].clone();
            self.index += 1;
            Ok(Some(row))
        }
    }
}

pub struct DbConn {
    client: Arc<tokio_postgres::Client>,
    pub(crate) backend: DatabaseBackend,
}

impl DbConn {
    /// True when the underlying connection's driver task has terminated
    /// (server closed the socket, idle timeout, network drop). A closed
    /// client fails every subsequent query — callers should discard the
    /// pool and reconnect. Local check, no server round-trip.
    pub fn is_closed(&self) -> bool {
        self.client.is_closed()
    }

    pub async fn execute(&self, sql: &str, params: impl IntoDbArgs) -> Result<u64> {
        let args = params.into_db_args();
        let (mut sql, bindings) = rebind_sql(sql, &args);
        if self.backend == DatabaseBackend::OpenGauss {
            sql = rewrite_for_opengauss(&sql);
        }
        let boxed: Vec<Box<dyn ToSql + Sync + Send>> =
            bindings.into_iter().map(to_pg_value).collect();
        let params: Vec<&(dyn ToSql + Sync)> = boxed.iter().map(|v| v.as_ref() as _).collect();
        let count = self.client.execute(sql.as_str(), &params).await?;
        Ok(count)
    }

    pub async fn query(&self, sql: &str, params: impl IntoDbArgs) -> Result<DbRows> {
        let args = params.into_db_args();
        let (mut sql, bindings) = rebind_sql(sql, &args);
        if self.backend == DatabaseBackend::OpenGauss {
            sql = rewrite_for_opengauss(&sql);
        }
        let boxed: Vec<Box<dyn ToSql + Sync + Send>> =
            bindings.into_iter().map(to_pg_value).collect();
        let params: Vec<&(dyn ToSql + Sync)> = boxed.iter().map(|v| v.as_ref() as _).collect();
        let rows = self.client.query(sql.as_str(), &params).await?;
        let mut out = Vec::new();
        for row in rows {
            let mut values = Vec::new();
            for idx in 0..row.len() {
                let value = if let Ok(v) = row.try_get::<usize, i64>(idx) {
                    DbValue::Integer(v)
                } else if let Ok(v) = row.try_get::<usize, f64>(idx) {
                    DbValue::Real(v)
                } else if let Ok(v) = row.try_get::<usize, String>(idx) {
                    DbValue::Text(v)
                } else if let Ok(v) = row.try_get::<usize, Vec<u8>>(idx) {
                    DbValue::Blob(v)
                } else {
                    DbValue::Null
                };
                values.push(value);
            }
            out.push(DbRow {
                values: Arc::new(values),
            });
        }
        Ok(DbRows { rows: out, index: 0 })
    }

    /// Execute one or more SQL statements without parameter binding.
    /// Suitable for DDL that may contain multiple semicolon-separated commands.
    pub async fn batch_execute(&self, sql: &str) -> Result<()> {
        let rewritten;
        let effective = if self.backend == DatabaseBackend::OpenGauss {
            rewritten = rewrite_for_opengauss(sql);
            rewritten.as_str()
        } else {
            sql
        };
        self.client.batch_execute(effective).await?;
        Ok(())
    }

    pub async fn prepare(&self, sql: &str) -> Result<DbStatement<'_>> {
        Ok(DbStatement {
            conn: self,
            sql: sql.to_string(),
        })
    }

    pub async fn prepare_cached(&self, sql: &str) -> Result<DbStatement<'_>> {
        Ok(DbStatement {
            conn: self,
            sql: sql.to_string(),
        })
    }
}

pub struct DbStatement<'a> {
    conn: &'a DbConn,
    sql: String,
}

impl<'a> DbStatement<'a> {
    pub async fn query(&self, params: impl IntoDbArgs) -> Result<DbRows> {
        self.conn.query(self.sql.as_str(), params).await
    }

    pub async fn execute(&self, params: impl IntoDbArgs) -> Result<u64> {
        self.conn.execute(self.sql.as_str(), params).await
    }

    pub async fn query_row(&self, params: impl IntoDbArgs) -> Result<DbRow> {
        let mut rows = self.conn.query(self.sql.as_str(), params).await?;
        rows.next()
            .await?
            .ok_or_else(|| Error::Internal("no rows returned".to_string()))
    }

    pub fn reset(&mut self) -> Result<()> {
        Ok(())
    }
}

pub struct DbTransaction<'a> {
    conn: &'a DbConn,
}

#[derive(Debug, Clone, Copy)]
pub enum TransactionBehavior {
    Deferred,
    Immediate,
}

impl<'a> DbTransaction<'a> {
    pub async fn new_unchecked(
        conn: &'a DbConn,
        _behavior: TransactionBehavior,
    ) -> Result<DbTransaction<'a>> {
        conn.execute("BEGIN", ()).await?;
        Ok(DbTransaction { conn })
    }

    pub async fn commit(&self) -> Result<()> {
        self.conn.execute("COMMIT", ()).await?;
        Ok(())
    }

    pub async fn rollback(&self) -> Result<()> {
        self.conn.execute("ROLLBACK", ()).await?;
        Ok(())
    }
}

pub struct DbPool {
    clients: Vec<Arc<tokio_postgres::Client>>,
    rr: AtomicUsize,
    backend: DatabaseBackend,
}

impl DbPool {
    pub fn new(clients: Vec<Arc<tokio_postgres::Client>>) -> Self {
        Self {
            clients,
            rr: AtomicUsize::new(0),
            backend: DatabaseBackend::Postgres,
        }
    }

    pub fn with_backend(
        clients: Vec<Arc<tokio_postgres::Client>>,
        backend: DatabaseBackend,
    ) -> Self {
        Self {
            clients,
            rr: AtomicUsize::new(0),
            backend,
        }
    }

    pub async fn get_connection(&self) -> Result<DbConn> {
        if self.clients.is_empty() {
            return Err(Error::Internal(
                "postgres client pool is empty".to_string(),
            ));
        }
        let idx = self.rr.fetch_add(1, Ordering::Relaxed) % self.clients.len();
        let client = Arc::clone(&self.clients[idx]);
        Ok(DbConn {
            client,
            backend: self.backend,
        })
    }

    /// Return the number of connections in the pool.
    pub fn len(&self) -> usize {
        self.clients.len()
    }

    /// Return a `DbConn` wrapping the connection at slot `index` (no
    /// round-robin increment). Used by `ConnectionPool::set_volume_id_guc` to
    /// reach every underlying connection exactly once.
    pub async fn get_connection_at(&self, index: usize) -> Result<DbConn> {
        if self.clients.is_empty() {
            return Err(Error::Internal(
                "postgres client pool is empty".to_string(),
            ));
        }
        let idx = index % self.clients.len();
        let client = Arc::clone(&self.clients[idx]);
        Ok(DbConn {
            client,
            backend: self.backend,
        })
    }
}

fn to_pg_value(value: DbValue) -> Box<dyn ToSql + Sync + Send> {
    match value {
        DbValue::Integer(i) => Box::new(i),
        DbValue::Real(f) => Box::new(f),
        DbValue::Text(s) => Box::new(s),
        DbValue::Blob(b) => Box::new(b),
        DbValue::Null => Box::new(Option::<i64>::None),
    }
}

fn rebind_sql(sql: &str, args: &[DbValue]) -> (String, Vec<DbValue>) {
    let mut out = String::with_capacity(sql.len());
    let mut index = 1;
    let mut chars = sql.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '?' {
            while let Some(next) = chars.peek() {
                if next.is_ascii_digit() {
                    chars.next();
                } else {
                    break;
                }
            }
            out.push('$');
            out.push_str(&index.to_string());
            index += 1;
        } else {
            out.push(ch);
        }
    }
    (out, args.to_vec())
}

// ---------------------------------------------------------------------------
// OpenGauss SQL compatibility
// ---------------------------------------------------------------------------
// OpenGauss 5.0/6.0 (PG 9.2 kernel) does NOT support
// ``ON CONFLICT ... DO UPDATE SET col = EXCLUDED.col`` (PG 9.5+).
// It supports MySQL-compatible ``ON DUPLICATE KEY UPDATE col = VALUES(col)``.

/// Rewrite PG-flavored DDL/DML to be accepted by OpenGauss 6.0 (PG 9.2 kernel).
/// Composes the ON CONFLICT rewrite with three more compatibility passes:
/// * `EXECUTE FUNCTION` → `EXECUTE PROCEDURE` (PG ≤10 / OpenGauss legacy
///   trigger-function syntax)
/// * `ADD COLUMN IF NOT EXISTS` → `ADD COLUMN` (OpenGauss does not support
///   the PG 9.6+ guard; callers wrap each ALTER in `.ok()` so the
///   "column already exists" error on a subsequent run is harmless).
/// * `current_setting('NAME', true)` → `current_setting('NAME')` (the 2-arg
///   `missing_ok` form is PG 9.6+; OpenGauss does not have it. Callers
///   ensure the GUC is SET on every session that fires the trigger, so the
///   1-arg form succeeds at runtime — see `ConnectionPool::set_volume_id_guc`.)
fn rewrite_for_opengauss(sql: &str) -> String {
    let s = rewrite_on_conflict_for_opengauss(sql);
    let s = s.replace("EXECUTE FUNCTION", "EXECUTE PROCEDURE");
    let s = s.replace("ADD COLUMN IF NOT EXISTS", "ADD COLUMN");
    rewrite_current_setting_missing_ok(&s)
}

/// Strip the second `, true` argument from `current_setting('NAME', true)`.
/// Whitespace between the name and `true` is tolerated.
fn rewrite_current_setting_missing_ok(sql: &str) -> String {
    let needle = "current_setting(";
    let mut out = String::with_capacity(sql.len());
    let mut rest = sql;
    while let Some(pos) = rest.find(needle) {
        out.push_str(&rest[..pos]);
        out.push_str(needle);
        let after = &rest[pos + needle.len()..];
        let trimmed = after.trim_start();
        if let Some(quoted_end) = trimmed.strip_prefix('\'').and_then(|s| s.find('\'')) {
            let name = &trimmed[..quoted_end + 2]; // includes both quotes
            let after_quote = &trimmed[quoted_end + 2..];
            let after_quote_trim = after_quote.trim_start();
            if let Some(after_comma) = after_quote_trim.strip_prefix(',') {
                let after_comma_trim = after_comma.trim_start();
                if let Some(after_true) = after_comma_trim.strip_prefix("true") {
                    let after_true_trim = after_true.trim_start();
                    if let Some(tail) = after_true_trim.strip_prefix(')') {
                        out.push_str(name);
                        out.push(')');
                        rest = tail;
                        continue;
                    }
                }
            }
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Find the first column in the `INSERT INTO <table> (col1, col2, ...)` list
/// that is *not* part of the ON CONFLICT key set. The result is suitable for
/// emitting `ON DUPLICATE KEY UPDATE <col> = <col>` on OpenGauss, which
/// rejects updates on primary/unique keys in that clause. Returns `None` if
/// the INSERT column list cannot be located or every column is in the
/// conflict set (rare and only for fully-keyed tables).
fn pick_non_conflict_insert_column(insert_prefix: &str, conflict_cols: &str) -> Option<String> {
    let upper = insert_prefix.to_uppercase();
    let insert_pos = upper.rfind("INSERT INTO")?;
    let after_insert = &insert_prefix[insert_pos + "INSERT INTO".len()..];
    let paren_open_rel = after_insert.find('(')?;
    let paren_close_rel = after_insert.find(')')?;
    if paren_open_rel >= paren_close_rel {
        return None;
    }
    let cols_str = &after_insert[paren_open_rel + 1..paren_close_rel];
    let conflict_set: Vec<String> = conflict_cols
        .split(',')
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty())
        .collect();
    cols_str
        .split(',')
        .map(|s| s.trim())
        .find(|c| !c.is_empty() && !conflict_set.contains(&c.to_uppercase()))
        .map(|c| c.to_string())
}

/// Rewrite PostgreSQL ``ON CONFLICT`` syntax to OpenGauss-compatible syntax.
///
/// * ``ON CONFLICT (...) DO UPDATE SET col = EXCLUDED.col``
///   → ``ON DUPLICATE KEY UPDATE col = VALUES(col)``
/// * ``ON CONFLICT (col) DO NOTHING``
///   → ``ON DUPLICATE KEY UPDATE <non-key-col> = <non-key-col>`` (no-op
///     upsert; assignment must target a non-key column).
fn rewrite_on_conflict_for_opengauss(sql: &str) -> String {
    let upper = sql.to_uppercase();

    // Find "ON CONFLICT" (case-insensitive)
    let Some(oc_start) = find_keyword(&upper, "ON CONFLICT") else {
        return sql.to_string();
    };

    // Find what comes after ON CONFLICT + optional (...). When parens are
    // present, also remember the *original-case* column list inside them so
    // we can emit a MySQL-style no-op UPDATE for `DO NOTHING`.
    let after_oc = &upper[oc_start + "ON CONFLICT".len()..];
    let mut conflict_cols_orig: Option<String> = None;
    let rest_start = if let Some(paren_start) = after_oc.find('(') {
        if after_oc[..paren_start].trim().is_empty() {
            if let Some(paren_end) = after_oc.find(')') {
                let abs_paren_open = oc_start + "ON CONFLICT".len() + paren_start + 1;
                let abs_paren_close = oc_start + "ON CONFLICT".len() + paren_end;
                conflict_cols_orig = Some(sql[abs_paren_open..abs_paren_close].trim().to_string());
                oc_start + "ON CONFLICT".len() + paren_end + 1
            } else {
                return sql.to_string();
            }
        } else {
            oc_start + "ON CONFLICT".len()
        }
    } else {
        oc_start + "ON CONFLICT".len()
    };

    let rest_upper = upper[rest_start..].trim_start();

    if rest_upper.starts_with("DO NOTHING") {
        // OpenGauss has no `INSERT ... ON CONFLICT DO NOTHING` and stripping
        // the clause leaves a plain INSERT that fails on a duplicate key.
        // Rewrite to MySQL-compat `ON DUPLICATE KEY UPDATE <col> = <col>`
        // — but the assignment target must be a *non-key* column (OpenGauss
        // rejects updating PK/unique columns in this form). Parse the
        // INSERT's column list and pick the first column that is not in
        // the `ON CONFLICT (...)` set.
        let do_nothing_end = rest_start
            + upper[rest_start..].find("DO NOTHING").unwrap()
            + "DO NOTHING".len();
        let mut result = sql[..oc_start].to_string();
        let no_op_col = conflict_cols_orig
            .as_deref()
            .and_then(|conflict| pick_non_conflict_insert_column(&sql[..oc_start], conflict));
        if let Some(col) = no_op_col {
            result.push_str(&format!("ON DUPLICATE KEY UPDATE {col} = {col}"));
        }
        result.push_str(&sql[do_nothing_end..]);
        return result;
    }

    if rest_upper.starts_with("DO UPDATE SET") {
        // Replace ON CONFLICT (...) DO UPDATE SET → ON DUPLICATE KEY UPDATE
        let do_update_set_end = rest_start
            + upper[rest_start..].find("DO UPDATE SET").unwrap()
            + "DO UPDATE SET".len();
        let mut result = sql[..oc_start].to_string();
        result.push_str("ON DUPLICATE KEY UPDATE");
        let tail = &sql[do_update_set_end..];
        // Replace EXCLUDED.col → VALUES(col) in the tail
        result.push_str(&replace_excluded_refs(tail));
        return result;
    }

    sql.to_string()
}

/// Case-insensitive search for a keyword preceded by a word boundary.
fn find_keyword(upper: &str, keyword: &str) -> Option<usize> {
    let mut search_from = 0;
    loop {
        if let Some(pos) = upper[search_from..].find(keyword) {
            let abs_pos = search_from + pos;
            // Ensure word boundary: char before must not be alphanumeric
            if abs_pos == 0
                || !upper.as_bytes()[abs_pos - 1].is_ascii_alphanumeric()
            {
                return Some(abs_pos);
            }
            search_from = abs_pos + 1;
        } else {
            return None;
        }
    }
}

/// Replace ``EXCLUDED.colname`` with ``VALUES(colname)`` (case-insensitive).
fn replace_excluded_refs(sql: &str) -> String {
    let mut result = String::with_capacity(sql.len());
    let bytes = sql.as_bytes();
    let upper = sql.to_uppercase();
    let mut i = 0;
    while i < bytes.len() {
        if upper[i..].starts_with("EXCLUDED.") {
            let dot_pos = i + "EXCLUDED".len();
            let col_start = dot_pos + 1;
            let mut col_end = col_start;
            while col_end < bytes.len()
                && (bytes[col_end].is_ascii_alphanumeric() || bytes[col_end] == b'_')
            {
                col_end += 1;
            }
            if col_end > col_start {
                // Check word boundary before EXCLUDED
                let is_boundary = i == 0
                    || !bytes[i - 1].is_ascii_alphanumeric()
                        && bytes[i - 1] != b'_';
                if is_boundary {
                    let col = &sql[col_start..col_end];
                    result.push_str("VALUES(");
                    result.push_str(col);
                    result.push(')');
                    i = col_end;
                    continue;
                }
            }
        }
        result.push(sql[i..].chars().next().unwrap());
        i += 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rewrite_on_conflict_do_update() {
        let sql = "INSERT INTO t (k, v) VALUES ($1, $2) ON CONFLICT (k) DO UPDATE SET v = EXCLUDED.v";
        let result = rewrite_on_conflict_for_opengauss(sql);
        assert!(result.contains("ON DUPLICATE KEY UPDATE"), "got: {result}");
        assert!(result.contains("VALUES(v)"), "got: {result}");
        assert!(!result.to_uppercase().contains("EXCLUDED"), "got: {result}");
    }

    #[test]
    fn test_rewrite_on_conflict_do_nothing() {
        let sql = "INSERT INTO t (id, v) VALUES ($1, $2) ON CONFLICT (id) DO NOTHING";
        let result = rewrite_on_conflict_for_opengauss(sql);
        assert!(!result.to_uppercase().contains("ON CONFLICT"), "got: {result}");
    }

    #[test]
    fn test_no_rewrite_for_regular_sql() {
        let sql = "SELECT * FROM t WHERE id = $1";
        let result = rewrite_on_conflict_for_opengauss(sql);
        assert_eq!(result, sql);
    }

    #[test]
    fn test_rewrite_multi_column_excluded() {
        let sql = "INSERT INTO t (a, b) VALUES ($1, $2) ON CONFLICT (a) DO UPDATE SET a = EXCLUDED.a, b = EXCLUDED.b";
        let result = rewrite_on_conflict_for_opengauss(sql);
        assert!(result.contains("VALUES(a)"), "got: {result}");
        assert!(result.contains("VALUES(b)"), "got: {result}");
    }
}
