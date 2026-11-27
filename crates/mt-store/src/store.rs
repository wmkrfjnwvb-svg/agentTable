//! The installation's storage: a control database listing the tenants, and one database per
//! tenant (a file on SQLite, a database on MySQL and PostgreSQL).

use crate::dialect::{check_ident, Dialect};
use crate::error::{StoreError, StoreResult};
use crate::schema::{migrate, Kind, TENANT_VERSION};
use mt_core::id::TenantId;
use sqlx::any::AnyPoolOptions;
use sqlx::AnyPool;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Where the databases live.
#[derive(Debug, Clone)]
enum Location {
    /// A directory of SQLite files.
    Files(PathBuf),
    /// A MySQL or PostgreSQL server, reached with this URL (its own database is used for
    /// creating and dropping the others).
    Server(url::Url),
}

/// How to open the storage.
#[derive(Debug, Clone)]
pub struct StoreConfig {
    /// `sqlite://<directory>`, `mysql://user:pw@host:port/db` or
    /// `postgres://user:pw@host:port/db`.
    pub url: String,
    /// Prefix of the databases this installation creates (`mt_` → `mt_control`, `mt_t_…`).
    pub prefix: String,
    /// Most connections per tenant database (MySQL and PostgreSQL; SQLite uses one writer).
    pub max_connections: u32,
    /// How long to wait for a connection before giving up (ms).
    pub connect_timeout_ms: u64,
}

impl StoreConfig {
    /// A configuration with the default prefix and pool size.
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        StoreConfig {
            url: url.into(),
            prefix: "mt_".to_owned(),
            max_connections: 5,
            connect_timeout_ms: 10_000,
        }
    }
}

/// A tenant as the control database lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantInfo {
    /// The tenant.
    pub id: TenantId,
    /// Its name.
    pub name: String,
    /// `active` (other states come with suspension and deletion).
    pub status: String,
    /// Its database.
    pub db_name: String,
    /// The schema version of its database.
    pub schema_version: i64,
    /// When it was created (ms).
    pub created_at: i64,
}

/// One tenant's database.
#[derive(Debug, Clone)]
pub struct Tenant {
    /// The tenant.
    pub id: TenantId,
    pub(crate) dialect: Dialect,
    pub(crate) pool: AnyPool,
}

impl Tenant {
    /// The engine.
    #[must_use]
    pub const fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// The connection pool, for maintenance tools and tests. It bypasses every check of the
    /// store: use the methods instead.
    #[must_use]
    pub const fn pool(&self) -> &AnyPool {
        &self.pool
    }
}

/// The storage of an installation.
#[derive(Debug)]
pub struct Store {
    dialect: Dialect,
    location: Location,
    prefix: String,
    max_connections: u32,
    connect_timeout_ms: u64,
    admin: Option<AnyPool>,
    control: AnyPool,
    open: Mutex<BTreeMap<TenantId, AnyPool>>,
}

/// A `u64` as a database integer.
///
/// # Errors
/// [`StoreError::Invalid`] above `i64::MAX`.
pub fn int(value: u64) -> StoreResult<i64> {
    i64::try_from(value).map_err(|_| StoreError::Invalid(format!("{value} is too large to store")))
}

/// A database integer as a `u64`.
///
/// # Errors
/// [`StoreError::Corrupt`] for a negative number.
pub fn uint(value: i64) -> StoreResult<u64> {
    u64::try_from(value).map_err(|_| StoreError::Corrupt(format!("{value} is negative")))
}

async fn connect(dialect: Dialect, url: &str, max: u32, timeout_ms: u64) -> StoreResult<AnyPool> {
    sqlx::any::install_default_drivers();
    let options = AnyPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(timeout_ms.max(1)))
        .max_connections(match dialect {
            // One connection is the single writer of the file; no transaction can then find it
            // locked by another.
            Dialect::Sqlite => 1,
            Dialect::MySql | Dialect::Postgres => max.max(1),
        });
    let options = match dialect {
        Dialect::Sqlite => options.after_connect(|conn, _| {
            Box::pin(async move {
                sqlx::query("PRAGMA journal_mode = WAL")
                    .execute(&mut *conn)
                    .await?;
                sqlx::query("PRAGMA synchronous = NORMAL")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        }),
        Dialect::MySql | Dialect::Postgres => options,
    };
    Ok(options.connect(url).await?)
}

impl Store {
    /// Opens (and creates, if needed) the storage and its control database.
    ///
    /// # Errors
    /// [`StoreError::Invalid`] for a bad URL or prefix; the database's errors.
    pub async fn open(config: &StoreConfig, now_ms: i64) -> StoreResult<Store> {
        let dialect = Dialect::of_url(&config.url)?;
        check_ident(&format!("{}control", config.prefix))?;
        let (location, admin) = match dialect {
            Dialect::Sqlite => {
                let dir = config
                    .url
                    .strip_prefix("sqlite://")
                    .or_else(|| config.url.strip_prefix("sqlite:"))
                    .filter(|d| !d.is_empty())
                    .ok_or_else(|| {
                        StoreError::Invalid("give SQLite a directory: sqlite://<dir>".to_owned())
                    })?;
                let dir = PathBuf::from(dir);
                std::fs::create_dir_all(&dir)
                    .map_err(|e| StoreError::Invalid(format!("{}: {e}", dir.display())))?;
                (Location::Files(dir), None)
            }
            Dialect::MySql | Dialect::Postgres => {
                let url = url::Url::parse(&config.url)
                    .map_err(|e| StoreError::Invalid(format!("the database URL: {e}")))?;
                let admin = connect(dialect, url.as_str(), 2, config.connect_timeout_ms).await?;
                (Location::Server(url), Some(admin))
            }
        };
        let control_db = format!("{}control", config.prefix);
        create_database(dialect, admin.as_ref(), &control_db).await?;
        let control = connect(
            dialect,
            &url_of(&location, &control_db)?,
            config.max_connections,
            config.connect_timeout_ms,
        )
        .await?;
        migrate(&control, dialect, Kind::Control, now_ms).await?;
        Ok(Store {
            dialect,
            location,
            prefix: config.prefix.clone(),
            max_connections: config.max_connections,
            connect_timeout_ms: config.connect_timeout_ms,
            admin,
            control,
            open: Mutex::new(BTreeMap::new()),
        })
    }

    /// The engine.
    #[must_use]
    pub const fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// The control database's pool, for maintenance tools and tests (bypasses the store).
    #[must_use]
    pub const fn control(&self) -> &AnyPool {
        &self.control
    }

    fn url_of(&self, db: &str) -> StoreResult<String> {
        url_of(&self.location, db)
    }

    async fn create_database(&self, db: &str) -> StoreResult<()> {
        create_database(self.dialect, self.admin.as_ref(), db).await
    }

    fn admin(&self) -> StoreResult<&AnyPool> {
        self.admin
            .as_ref()
            .ok_or_else(|| StoreError::Invalid("no server connection".to_owned()))
    }

    async fn drop_database(&self, db: &str) -> StoreResult<()> {
        let quoted = self.dialect.quote(db)?;
        match &self.location {
            Location::Files(dir) => {
                for suffix in ["db", "db-wal", "db-shm"] {
                    let path = dir.join(format!("{db}.{suffix}"));
                    if path.exists() {
                        std::fs::remove_file(&path)
                            .map_err(|e| StoreError::Db(format!("{}: {e}", path.display())))?;
                    }
                }
                Ok(())
            }
            Location::Server(_) => {
                let sql = match self.dialect {
                    Dialect::Postgres => format!("DROP DATABASE IF EXISTS {quoted} WITH (FORCE)"),
                    Dialect::MySql | Dialect::Sqlite => format!("DROP DATABASE IF EXISTS {quoted}"),
                };
                sqlx::query(&sql).execute(self.admin()?).await?;
                Ok(())
            }
        }
    }

    fn tenant_db(&self, id: TenantId) -> String {
        format!("{}t_{}", self.prefix, id)
    }

    /// Creates a tenant and its database, at the latest schema.
    ///
    /// # Errors
    /// [`StoreError::Exists`] for a tenant that exists; [`StoreError::Invalid`] for an empty
    /// name; the database's errors.
    pub async fn create_tenant(
        &self,
        id: TenantId,
        name: &str,
        now_ms: i64,
    ) -> StoreResult<Tenant> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StoreError::Invalid("a tenant needs a name".to_owned()));
        }
        if self.info(id).await?.is_some() {
            return Err(StoreError::Exists(format!("tenant {id}")));
        }
        let db = self.tenant_db(id);
        self.create_database(&db).await?;
        let pool = connect(
            self.dialect,
            &self.url_of(&db)?,
            self.max_connections,
            self.connect_timeout_ms,
        )
        .await?;
        let version = migrate(&pool, self.dialect, Kind::Tenant, now_ms).await?;
        let d = self.dialect;
        sqlx::query(&format!(
            "INSERT INTO tenants (id, name, status, db_name, schema_version, created_at) VALUES ({})",
            d.params(1, 6)
        ))
        .bind(id.to_string())
        .bind(name.to_owned())
        .bind("active".to_owned())
        .bind(db)
        .bind(version)
        .bind(now_ms)
        .execute(&self.control)
        .await?;
        self.open.lock().await.insert(id, pool.clone());
        Ok(Tenant {
            id,
            dialect: d,
            pool,
        })
    }

    /// A tenant's database, opened (and brought to the latest schema) on first use.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] for an unknown tenant; the database's errors.
    pub async fn tenant(&self, id: TenantId, now_ms: i64) -> StoreResult<Tenant> {
        let mut open = self.open.lock().await;
        if let Some(pool) = open.get(&id) {
            return Ok(Tenant {
                id,
                dialect: self.dialect,
                pool: pool.clone(),
            });
        }
        let info = self
            .info(id)
            .await?
            .ok_or_else(|| StoreError::NotFound(format!("tenant {id}")))?;
        let pool = connect(
            self.dialect,
            &self.url_of(&info.db_name)?,
            self.max_connections,
            self.connect_timeout_ms,
        )
        .await?;
        if info.schema_version < TENANT_VERSION {
            let version = migrate(&pool, self.dialect, Kind::Tenant, now_ms).await?;
            sqlx::query(&format!(
                "UPDATE tenants SET schema_version = {} WHERE id = {}",
                self.dialect.param(1),
                self.dialect.param(2)
            ))
            .bind(version)
            .bind(id.to_string())
            .execute(&self.control)
            .await?;
        }
        open.insert(id, pool.clone());
        Ok(Tenant {
            id,
            dialect: self.dialect,
            pool,
        })
    }

    /// A tenant as listed.
    ///
    /// # Errors
    /// The database's errors; [`StoreError::Corrupt`] for an unreadable row.
    pub async fn info(&self, id: TenantId) -> StoreResult<Option<TenantInfo>> {
        let row = sqlx::query(&format!(
            "SELECT id, name, status, db_name, schema_version, created_at FROM tenants WHERE id = {}",
            self.dialect.param(1)
        ))
        .bind(id.to_string())
        .fetch_optional(&self.control)
        .await?;
        row.map(|r| info_of(&r)).transpose()
    }

    /// Every tenant, by id.
    ///
    /// # Errors
    /// The database's errors.
    pub async fn tenants(&self) -> StoreResult<Vec<TenantInfo>> {
        let rows = sqlx::query(
            "SELECT id, name, status, db_name, schema_version, created_at FROM tenants ORDER BY id",
        )
        .fetch_all(&self.control)
        .await?;
        rows.iter().map(info_of).collect()
    }

    /// Deletes a tenant and its database, for good (erasure).
    ///
    /// # Errors
    /// [`StoreError::NotFound`]; the database's errors.
    pub async fn drop_tenant(&self, id: TenantId) -> StoreResult<()> {
        let info = self
            .info(id)
            .await?
            .ok_or_else(|| StoreError::NotFound(format!("tenant {id}")))?;
        if let Some(pool) = self.open.lock().await.remove(&id) {
            pool.close().await;
        }
        self.drop_database(&info.db_name).await?;
        sqlx::query(&format!(
            "DELETE FROM tenants WHERE id = {}",
            self.dialect.param(1)
        ))
        .bind(id.to_string())
        .execute(&self.control)
        .await?;
        Ok(())
    }

    /// Closes every connection and removes the installation's databases (tests, and
    /// uninstalling).
    ///
    /// # Errors
    /// The database's errors.
    pub async fn destroy(self) -> StoreResult<()> {
        for tenant in self.tenants().await? {
            self.drop_tenant(tenant.id).await?;
        }
        self.control.close().await;
        self.drop_database(&format!("{}control", self.prefix))
            .await?;
        if let Some(admin) = &self.admin {
            admin.close().await;
        }
        Ok(())
    }

    /// Closes every connection.
    pub async fn close(&self) {
        for pool in self.open.lock().await.values() {
            pool.close().await;
        }
        self.control.close().await;
        if let Some(admin) = &self.admin {
            admin.close().await;
        }
    }
}

fn url_of(location: &Location, db: &str) -> StoreResult<String> {
    check_ident(db)?;
    match location {
        Location::Files(dir) => Ok(format!(
            "sqlite://{}?mode=rwc",
            dir.join(format!("{db}.db")).display()
        )),
        Location::Server(url) => {
            let mut url = url.clone();
            url.set_path(&format!("/{db}"));
            Ok(url.to_string())
        }
    }
}

async fn create_database(dialect: Dialect, admin: Option<&AnyPool>, db: &str) -> StoreResult<()> {
    let quoted = dialect.quote(db)?;
    let admin = || admin.ok_or_else(|| StoreError::Invalid("no server connection".to_owned()));
    match dialect {
        Dialect::Sqlite => Ok(()),
        Dialect::MySql => {
            sqlx::query(&format!(
                "CREATE DATABASE IF NOT EXISTS {quoted} CHARACTER SET utf8mb4"
            ))
            .execute(admin()?)
            .await?;
            Ok(())
        }
        Dialect::Postgres => {
            let exists: Option<i32> =
                sqlx::query_scalar("SELECT 1 FROM pg_database WHERE datname = $1")
                    .bind(db.to_owned())
                    .fetch_optional(admin()?)
                    .await?;
            if exists.is_none() {
                sqlx::query(&format!("CREATE DATABASE {quoted} ENCODING 'UTF8'"))
                    .execute(admin()?)
                    .await?;
            }
            Ok(())
        }
    }
}

fn info_of(row: &sqlx::any::AnyRow) -> StoreResult<TenantInfo> {
    let id = crate::row::text(row, "id")?;
    Ok(TenantInfo {
        id: id
            .parse()
            .map_err(|_| StoreError::Corrupt(format!("tenant id `{id}`")))?,
        name: crate::row::text(row, "name")?,
        status: crate::row::text(row, "status")?,
        db_name: crate::row::text(row, "db_name")?,
        schema_version: crate::row::int(row, "schema_version")?,
        created_at: crate::row::int(row, "created_at")?,
    })
}

/// Shared handle.
pub type SharedStore = Arc<Store>;
