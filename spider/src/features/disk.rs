#[cfg(feature = "disk")]
use case_insensitive_string::CaseInsensitiveString;
#[cfg(feature = "disk")]
use hashbrown::HashSet;
#[cfg(feature = "disk")]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(feature = "disk")]
use crate::utils::emit_log;
#[cfg(feature = "disk")]
use sqlx::{sqlite::SqlitePool, Sqlite, Transaction};

#[cfg(feature = "disk")]
lazy_static! {
    static ref AC: aho_corasick::AhoCorasick = {
        let patterns = vec![".", "/", ":", "\\", "?", "*", "\"", "<", ">", "|"];
        aho_corasick::AhoCorasick::new(&patterns).expect("valid replacer")
    };
    static ref AC_REPLACE: [&'static str; 10] = ["_", "_", "_", "_", "_", "_", "_", "_", "_", "_"];
}

#[derive(Default, Debug, Clone)]
#[cfg(feature = "disk")]
/// Manage Sqlite database operations
pub struct DatabaseHandler {
    /// Persist after drop.
    pub persist: bool,
    /// The crawl ID.
    pub crawl_id: Option<String>,
    /// The connection pool.
    pool: tokio::sync::OnceCell<SqlitePool>,
    /// Closed compatibility value, separate from successfully initialized storage.
    unavailable_pool: tokio::sync::OnceCell<SqlitePool>,
    /// Initial seed ran.
    pub seeded: bool,
}

#[derive(Default, Debug, Clone)]
#[cfg(not(feature = "disk"))]
/// Manage Sqlite database operations
pub struct DatabaseHandler {
    /// Persist after drop.
    pub persist: bool,
}

#[cfg(not(feature = "disk"))]
impl DatabaseHandler {
    /// A new DB handler.
    pub fn new(_crawl_id: &Option<String>) -> Self {
        Default::default()
    }
    /// Delete the db by id.
    pub fn delete_db_by_id(&mut self) {}
}

#[cfg(feature = "disk")]
impl DatabaseHandler {
    /// A new DB handler.
    pub fn new(crawl_id: &Option<String>) -> Self {
        Self {
            persist: false,
            pool: tokio::sync::OnceCell::const_new(),
            unavailable_pool: tokio::sync::OnceCell::const_new(),
            crawl_id: match crawl_id {
                Some(id) => {
                    let sanitized_id = AC.replace_all(id, &*AC_REPLACE);

                    Some(format!("{}_{}", sanitized_id, get_id()))
                }
                _ => None,
            },
            seeded: false,
        }
    }

    /// Determine if the pool is initialized.
    pub fn pool_inited(&self) -> bool {
        self.pool.initialized()
    }

    /// Determine if a seed was already done.
    pub fn ready(&self) -> bool {
        self.seeded
    }

    /// Set the seeded state
    pub fn set_seeded(&mut self, seeded: bool) {
        self.seeded = seeded;
    }

    /// Set the persist state
    pub fn set_persisted(&mut self, persist: bool) {
        self.persist = persist;
    }

    /// Generate a sqlite pool. On failure, log the storage error and return a
    /// closed pool so legacy callers receive `PoolClosed` from database operations.
    /// No replacement database is opened. Use `try_generate_pool` for the cause.
    pub async fn generate_pool(&self) -> SqlitePool {
        pool_or_closed(self.try_generate_pool().await).await
    }

    /// Generate a pool, propagating configuration, filesystem and schema errors.
    pub async fn try_generate_pool(&self) -> Result<SqlitePool, sqlx::Error> {
        let path = configured_db_path(std::env::var("SQLITE_DATABASE_URL"), &self.crawl_id)
            .map_err(|e| storage_error("invalid SQLite storage configuration", e))?;
        generate_pool_at(&path).await
    }

    /// Get or initialize the database pool (legacy infallible API).
    pub async fn initlaize_pool(&self) {
        let _ = self.get_db_pool().await;
    }

    /// Get or initialize the database pool, preserving the initialization error.
    /// Failed initialization is not cached, allowing a later retry.
    pub async fn try_get_db_pool(&self) -> Result<&SqlitePool, sqlx::Error> {
        self.pool.get_or_try_init(|| self.try_generate_pool()).await
    }

    /// Set the pool directly.
    pub async fn set_pool(&self, pool: SqlitePool) {
        let _ = self.pool.set(pool);
    }

    /// Get or initialize the database pool. Failures return a closed pool and
    /// leave initialization retryable; use `try_get_db_pool` for the cause.
    pub async fn get_db_pool(&self) -> &SqlitePool {
        match self.try_get_db_pool().await {
            Ok(pool) => pool,
            Err(error) => {
                self.unavailable_pool
                    .get_or_init(|| pool_or_closed(Err(error)))
                    .await
            }
        }
    }

    /// Check if a URL exists (ignore case)
    pub async fn url_exists(&self, pool: &SqlitePool, url_to_check: &str) -> bool {
        match sqlx::query("SELECT 1 FROM resources WHERE url = ? LIMIT 1")
            .bind(url_to_check)
            .fetch_optional(pool)
            .await
        {
            Ok(result) => result.is_some(),
            Err(e) => {
                if let Some(db_err) = e.as_database_error() {
                    emit_log(db_err.message());
                } else {
                    emit_log(&format!("A non-database error occurred: {:?}", e));
                }
                false
            }
        }
    }

    /// Check if a signature exists (ignore case)
    pub async fn signature_exists(&self, pool: &SqlitePool, signature_to_check: u64) -> bool {
        match sqlx::query("SELECT 1 FROM signatures WHERE url = ? LIMIT 1")
            .bind(signature_to_check.to_string())
            .fetch_optional(pool)
            .await
        {
            Ok(result) => result.is_some(),
            Err(e) => {
                if let Some(db_err) = e.as_database_error() {
                    emit_log(db_err.message());
                } else {
                    emit_log(&format!("A non-database error occurred: {:?}", e));
                }
                false
            }
        }
    }

    /// Insert a new URL if it doesn't exist
    pub async fn insert_url(&self, pool: &SqlitePool, new_url: &str) {
        if !self.url_exists(pool, new_url).await {
            if let Err(e) = sqlx::query("INSERT INTO resources (url) VALUES (?)")
                .bind(new_url)
                .execute(pool)
                .await
            {
                if let Some(db_err) = e.as_database_error() {
                    emit_log(db_err.message());
                } else {
                    emit_log(&format!("A non-database error occurred: {:?}", e));
                }
            }
        }
    }

    /// Insert a new signature if it doesn't exist
    pub async fn insert_signature(&self, pool: &SqlitePool, new_signature: u64) {
        if !self.signature_exists(pool, new_signature).await {
            if let Err(e) = sqlx::query("INSERT INTO signatures (url) VALUES (?)")
                .bind(new_signature.to_string())
                .execute(pool)
                .await
            {
                if let Some(db_err) = e.as_database_error() {
                    emit_log(db_err.message());
                } else {
                    emit_log(&format!("A non-database error occurred: {:?}", e));
                }
            }
        }
    }

    /// Seed the database and manage URLs
    pub async fn seed(
        &self,
        pool: &SqlitePool,
        mut urls: HashSet<CaseInsensitiveString>,
    ) -> Result<HashSet<CaseInsensitiveString>, sqlx::Error> {
        const CHUNK_SIZE: usize = 500;
        const KEEP_COUNT: usize = 100;

        let mut tx: Transaction<'_, Sqlite> = pool.begin().await?;
        let mut keep_urls = HashSet::with_capacity(KEEP_COUNT);

        for url in urls.iter().take(KEEP_COUNT) {
            keep_urls.insert(url.clone());
        }

        urls.retain(|url| !keep_urls.contains(url));

        for chunk in keep_urls.iter().collect::<Vec<_>>().chunks(CHUNK_SIZE) {
            let mut query = "INSERT OR IGNORE INTO resources (url) VALUES ".to_string();
            query.push_str(&vec!["(?)"; chunk.len()].join(", "));
            let mut statement = sqlx::query(&query);

            for url in chunk {
                statement = statement.bind(url.to_string());
            }

            statement.execute(&mut *tx).await?;
        }

        for chunk in urls.drain().collect::<Vec<_>>().chunks(CHUNK_SIZE) {
            let mut query = "INSERT OR IGNORE INTO resources (url) VALUES ".to_string();
            query.push_str(&vec!["(?)"; chunk.len()].join(", "));
            let mut statement = sqlx::query(&query);

            for url in chunk {
                statement = statement.bind(url.to_string());
            }

            statement.execute(&mut *tx).await?;
        }

        tx.commit().await?;

        Ok(keep_urls)
    }

    /// Count the records stored.
    pub async fn count_records(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
        let result = sqlx::query_scalar::<_, u64>("SELECT COUNT(*) FROM resources")
            .fetch_one(pool)
            .await?;
        Ok(result)
    }

    /// Get all the resources stored.
    pub async fn get_all_resources(
        pool: &SqlitePool,
    ) -> Result<HashSet<CaseInsensitiveString>, sqlx::Error> {
        use sqlx::Row;
        let rows = sqlx::query("SELECT url FROM resources")
            .fetch_all(pool) // Fetches all rows at once.
            .await?;

        let urls = rows
            .into_iter()
            .map(|row| row.get::<String, _>("url").into())
            .collect();

        Ok(urls)
    }

    /// Clear DB by id
    pub fn delete_db_by_id(&self) {
        let _ = std::fs::remove_file(get_db_path(&self.crawl_id));
    }

    /// Clear the resources table.
    pub async fn clear_table(pool: &SqlitePool) -> Result<(), sqlx::Error> {
        let _ = tokio::join!(
            sqlx::query("DELETE FROM resources").execute(pool),
            sqlx::query("DELETE FROM signatures").execute(pool)
        );
        Ok(())
    }
}

#[cfg(feature = "disk")]
impl Drop for DatabaseHandler {
    fn drop(&mut self) {
        if !self.persist {
            self.delete_db_by_id();
        }
    }
}

/// simple counter to get the next ID.
#[cfg(feature = "disk")]
fn get_id() -> usize {
    static COUNTER: AtomicUsize = AtomicUsize::new(1);

    let mut current = COUNTER.load(Ordering::Relaxed);
    loop {
        let next = if current == usize::MAX {
            1
        } else {
            current + 1
        };
        match COUNTER.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return current,
            Err(updated) => current = updated,
        }
    }
}

/// Get the db path. Invalid environment configuration returns an unusable NUL
/// path rather than redirecting persistence or cleanup to the default database.
pub fn get_db_path(crawl_id: &Option<String>) -> String {
    configured_db_path(std::env::var("SQLITE_DATABASE_URL"), crawl_id).unwrap_or_else(|error| {
        log::error!("Invalid SQLite storage configuration: {error}");
        "\0".to_owned()
    })
}

fn configured_db_path(
    configured: Result<String, std::env::VarError>,
    crawl_id: &Option<String>,
) -> std::io::Result<String> {
    let base_url = match configured {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => std::env::temp_dir()
            .into_os_string()
            .into_string()
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "SQLite temporary directory is not UTF-8",
                )
            })?,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "SQLITE_DATABASE_URL is not UTF-8",
            ))
        }
    };
    if base_url.is_empty() || base_url.contains('\0') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "SQLITE_DATABASE_URL is empty or contains NUL",
        ));
    }

    let delim = if base_url.starts_with("sqlite://memory:") {
        ":"
    } else {
        "/"
    };

    // Determine the db_path
    let db_path = match crawl_id {
        Some(crawl_id) => {
            format!(
                "{}{delim}spider_{}.db",
                base_url.trim_end_matches('/'),
                crawl_id.replace(".", "_")
            )
        }
        None => format!("{}{delim}spider.db", base_url.trim_end_matches('/')),
    };

    Ok(db_path)
}

/// Attach the operation and destination without changing the selected database.
#[cfg(feature = "disk")]
fn storage_error(context: &str, error: impl std::fmt::Display) -> sqlx::Error {
    sqlx::Error::Configuration(format!("{context}: {error}").into())
}

#[cfg(feature = "disk")]
async fn generate_pool_at(db_path: &str) -> Result<SqlitePool, sqlx::Error> {
    use sqlx::sqlite::SqliteConnectOptions;

    let db_url = if db_path.starts_with("sqlite:") {
        db_path.to_owned()
    } else {
        format!("sqlite://{db_path}")
    };
    // Parse before touching the filesystem. In particular, do not turn an
    // invalid URL into a filename or silently substitute an in-memory database.
    let options: SqliteConnectOptions = db_url
        .parse()
        .map_err(|e| storage_error("invalid SQLite storage URL", e))?;
    if let Some(parent) = options
        .get_filename()
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            storage_error(
                &format!("creating SQLite directory {}", parent.display()),
                e,
            )
        })?;
    }
    // SQLite creates the file atomically; an exists/write pair could truncate a
    // database created by another initializer between those two operations.
    let pool = SqlitePool::connect_with(options.create_if_missing(true))
        .await
        .map_err(|e| storage_error(&format!("opening SQLite storage {db_path}"), e))?;
    let schema = sqlx::raw_sql(
        "CREATE TABLE IF NOT EXISTS resources (
            id INTEGER PRIMARY KEY, url TEXT NOT NULL COLLATE NOCASE
        );
        CREATE INDEX IF NOT EXISTS idx_url ON resources (url COLLATE NOCASE);
        CREATE TABLE IF NOT EXISTS signatures (
            id INTEGER PRIMARY KEY, url INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_url ON signatures (url);",
    )
    .execute(&pool)
    .await;
    if let Err(e) = schema {
        pool.close().await;
        return Err(storage_error(
            &format!("initializing SQLite schema {db_path}"),
            e,
        ));
    }
    Ok(pool)
}

#[cfg(feature = "disk")]
async fn pool_or_closed(result: Result<SqlitePool, sqlx::Error>) -> SqlitePool {
    match result {
        Ok(pool) => pool,
        Err(error) => {
            log::error!("SQLite storage initialization failed: {error}");
            // An inert compatibility value, never a fallback database. There are
            // no initial connections, the filename is invalid on every OS, and
            // the pool is closed before it can be returned or acquired.
            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .min_connections(0)
                .connect_lazy_with(sqlx::sqlite::SqliteConnectOptions::new().filename("\0"));
            pool.close().await;
            pool
        }
    }
}

#[cfg(test)]
#[cfg(feature = "disk")]
mod tests {
    use super::*;
    use tokio;

    #[test]
    fn runtime_safety_disk_invalid_environment_is_not_redirected() {
        for value in [
            Ok(String::new()),
            Ok("bad\0path".into()),
            Err(std::env::VarError::NotUnicode("invalid".into())),
        ] {
            assert_eq!(
                configured_db_path(value, &None).unwrap_err().kind(),
                std::io::ErrorKind::InvalidInput
            );
        }
        let root = tempfile::tempdir().unwrap();
        let path = configured_db_path(
            Ok(root.path().to_str().unwrap().into()),
            &Some("test".into()),
        )
        .unwrap();
        assert_eq!(
            std::path::Path::new(&path),
            root.path().join("spider_test.db")
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn runtime_safety_disk_invalid_url_does_not_create_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("invalid.db");
        let url = format!("sqlite://{}?mode=invalid", path.display());
        let error = generate_pool_at(&url).await.unwrap_err();
        assert!(error.to_string().contains("invalid SQLite storage URL"));
        assert!(!path.exists());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn runtime_safety_disk_filesystem_error_is_contextual() {
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().join("not-a-directory");
        std::fs::write(&parent, b"keep").unwrap();
        let path = parent.join("nested/db.sqlite");
        let error = generate_pool_at(path.to_str().unwrap()).await.unwrap_err();
        assert!(error.to_string().contains("creating SQLite directory"));
        assert_eq!(std::fs::read(parent).unwrap(), b"keep");
    }

    #[tokio::test]
    async fn runtime_safety_disk_legacy_failure_returns_closed_pool() {
        let root = tempfile::tempdir().unwrap();
        let url = format!(
            "sqlite://{}?unknown=value",
            root.path().join("bad.db").display()
        );
        let pool = pool_or_closed(generate_pool_at(&url).await).await;
        assert!(pool.is_closed());
        assert!(matches!(pool.acquire().await, Err(sqlx::Error::PoolClosed)));
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn runtime_safety_disk_preserves_persistent_data_and_schema_errors() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("nested/storage.db");
        let pool = generate_pool_at(path.to_str().unwrap()).await.unwrap();
        sqlx::query("INSERT INTO resources (url) VALUES ('https://offline.invalid')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO signatures (url) VALUES (42)")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        let pool = generate_pool_at(path.to_str().unwrap()).await.unwrap();
        assert_eq!(DatabaseHandler::count_records(&pool).await.unwrap(), 1);
        let signatures: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM signatures")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(signatures, 1);
        pool.close().await;
        // A separate malformed test schema must report initialization failure.
        let malformed = root.path().join("malformed.db");
        let pool = SqlitePool::connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&malformed)
                .create_if_missing(true),
        )
        .await
        .unwrap();
        sqlx::query("CREATE TABLE resources (wrong TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        let error = generate_pool_at(malformed.to_str().unwrap())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("initializing SQLite schema"));
        assert!(path.exists());
    }

    #[tokio::test]
    async fn test_connect_db() {
        let handler = DatabaseHandler::new(&Some("example.com".into()));
        let test_url = CaseInsensitiveString::new("http://example.com");
        let pool = handler.get_db_pool().await;

        if handler.url_exists(pool, &test_url).await {
            println!("URL '{}' already exists in the database.", test_url);
        } else {
            handler.insert_url(pool, &test_url).await;
            println!("URL '{}' was inserted into the database.", test_url);
        }

        assert!(
            handler.url_exists(pool, &test_url).await,
            "URL should exist after insertion."
        );
    }

    #[tokio::test]
    async fn test_url_insert_and_exists() {
        let handler = DatabaseHandler::new(&Some("example.com".into()));
        let new_url = CaseInsensitiveString::new("http://new-example.com");
        let pool = handler.get_db_pool().await;

        assert!(
            !handler.url_exists(pool, &new_url).await,
            "URL should not exist initially."
        );

        handler.insert_url(pool, &new_url).await;
        assert!(
            handler.url_exists(pool, &new_url).await,
            "URL should exist after insertion."
        );
    }

    #[tokio::test]
    async fn test_url_case_insensitivity() {
        let handler = DatabaseHandler::new(&Some("case-test.com".into()));
        let url1 = CaseInsensitiveString::new("http://case-test.com");
        let url2 = CaseInsensitiveString::new("http://CASE-TEST.com");
        let pool = handler.get_db_pool().await;

        handler.insert_url(pool, &url1).await;
        assert!(
            handler.url_exists(pool, &url2).await,
            "URL check should be case-insensitive."
        );
    }

    #[tokio::test]
    async fn test_seed_urls() {
        let handler = DatabaseHandler::new(&Some("example.com".into()));
        let mut urls = HashSet::new();
        urls.insert(CaseInsensitiveString::new("http://foo.com"));
        urls.insert(CaseInsensitiveString::new("http://bar.com"));
        let pool = handler.get_db_pool().await;

        handler
            .seed(pool, urls.clone())
            .await
            .expect("Seeding failed");

        for url in urls {
            assert!(
                handler.url_exists(pool, &url).await,
                "Seeded URL should exist after seeding."
            );
        }
    }
}
