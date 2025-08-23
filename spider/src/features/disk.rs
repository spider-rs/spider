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
            crawl_id: match crawl_id {
                Some(id) => {
                    let sanitized_id = AC.replace_all(&id, &*AC_REPLACE);

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

    /// Generate a sqlite pool.
    pub async fn generate_pool(&self) -> SqlitePool {
        let db_path = get_db_path(&self.crawl_id);
        let direct = db_path.starts_with("sqlite://");

        // not a shared sqlite db.
        if direct {
            create_file_and_directory(&db_path[9..]).await;
        } else {
            create_file_and_directory(&db_path).await;
        }

        let db_url = if direct {
            db_path
        } else {
            format!("sqlite://{}", db_path)
        };

        let pool = SqlitePool::connect_lazy(&db_url).expect("Failed to connect to the database");

        let create_resources_table = sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS resources (
                            id INTEGER PRIMARY KEY,
                            url TEXT NOT NULL COLLATE NOCASE
                        );
                        CREATE INDEX IF NOT EXISTS idx_url ON resources (url COLLATE NOCASE);"#,
        )
        .execute(&pool);

        let create_signatures_table = sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS signatures (
                            id INTEGER PRIMARY KEY,
                            url INTEGER NOT NULL
                        );
                        CREATE INDEX IF NOT EXISTS idx_url ON signatures (url);"#,
        )
        .execute(&pool);

        // Run the queries concurrently
        let (resources_result, signatures_result) =
            tokio::join!(create_resources_table, create_signatures_table);

        // Handle possible errors
        if let Err(e) = resources_result {
            log::warn!("SQLite error creating resources table: {:?}", e);
        }

        if let Err(e) = signatures_result {
            log::warn!("SQLite error creating signatures table: {:?}", e);
        }

        pool
    }

    /// Get or initialize the database pool
    pub async fn initlaize_pool(&self) {
        if !self.pool_inited() {
            let _ = self.pool.set(self.generate_pool().await);
        }
    }

    /// Set the pool directly.
    pub async fn set_pool(&self, pool: SqlitePool) {
        let _ = self.pool.set(pool);
    }

    /// Get or initialize the database pool
    pub async fn get_db_pool(&self) -> &SqlitePool {
        self.pool.get_or_init(|| self.generate_pool()).await
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

/// Get the db path.
pub fn get_db_path(crawl_id: &Option<String>) -> String {
    // Get the base database URL or default to a temporary directory
    let base_url = std::env::var("SQLITE_DATABASE_URL").unwrap_or_else(|_| {
        let temp_dir = std::env::temp_dir();
        temp_dir.to_string_lossy().into_owned()
    });

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

    db_path
}

/// Create the file and directory if locally.
#[cfg(feature = "disk")]
async fn create_file_and_directory(file_path: &str) {
    let path = std::path::Path::new(file_path);

    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    if let Ok(exist) = tokio::fs::try_exists(path).await {
        if !exist {
            let _ = tokio::fs::File::create(path).await;
        }
    }
}

#[cfg(test)]
#[cfg(feature = "disk")]
mod tests {
    use super::*;
    use tokio;

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
