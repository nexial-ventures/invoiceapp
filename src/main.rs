use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use invoice::app::{self, AppState};
use invoice::ares::AresClient;
use invoice::auth::google::Google;
use invoice::auth::ratelimit::RateLimiter;
use invoice::cleanup;
use invoice::cnb::CnbClient;
use invoice::config::{Config, DbConfig};
use invoice::db;
use invoice::email::Mailer;
use invoice::migration::{Migrator, MigratorTrait};
use invoice::pdf::{PdfRoot, design};
use invoice::space;
use invoice::storage::transfer::{self, Report};
use invoice::storage::{DESIGN_PREFIX, Storage, StorageConfig};
use sea_orm::{Database, DatabaseConnection};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "invoice",
    version,
    about = "Invoice management (spaces, Czech VAT)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Apply pending migrations, then serve the API and SPA.
    Serve,
    /// Manage database migrations.
    Migrate {
        #[command(subcommand)]
        action: MigrateAction,
    },
    /// File storage maintenance (the configured INVOICE__STORAGE_KIND backend).
    Storage {
        #[command(subcommand)]
        action: StorageAction,
    },
    /// PDF design overrides of one space (storage `spaces/{id}/design/`).
    Design {
        #[command(subcommand)]
        action: DesignAction,
    },
    /// Print the OpenAPI document (as served at `/api/openapi.json`) to stdout.
    Openapi,
}

#[derive(Subcommand)]
enum StorageAction {
    /// Copy a filesystem storage directory (and an old design directory) into
    /// the configured backend. Verifies sha256, skips identical files, never
    /// overwrites different ones, never deletes the source.
    Migrate {
        /// The old INVOICE__STORAGE_DIR (keys = relative paths).
        #[arg(long)]
        from_dir: PathBuf,
        /// The old INVOICE__DESIGN_DIR (copied under `design/`).
        #[arg(long)]
        design_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum DesignAction {
    /// Upload every file of DIR to the space's `design/…` (changed files replaced).
    Push {
        dir: PathBuf,
        /// The space slug.
        #[arg(long)]
        space: String,
    },
    /// Download every `design/…` file of the space into DIR.
    Pull {
        dir: PathBuf,
        #[arg(long)]
        space: String,
    },
    /// List the effective design of the space: custom files and the built-in defaults.
    Ls {
        #[arg(long)]
        space: String,
    },
    /// Remove custom design files (the built-in default shows through again).
    Rm {
        #[arg(required = true)]
        paths: Vec<String>,
        #[arg(long)]
        space: String,
    },
}

impl DesignAction {
    fn space(&self) -> &str {
        match self {
            Self::Push { space, .. }
            | Self::Pull { space, .. }
            | Self::Ls { space }
            | Self::Rm { space, .. } => space,
        }
    }
}

#[derive(Subcommand)]
enum MigrateAction {
    /// Apply all pending migrations.
    Up,
    /// Show applied and pending migrations.
    Status,
    /// Roll back the last N migrations.
    Down {
        #[arg(short = 'n', long, default_value_t = 1)]
        n: u32,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let command = Cli::parse().command;
    // stdout carries the JSON document: no subscriber, so no log line can mix into it.
    if matches!(command, Command::Openapi) {
        return print_openapi();
    }
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("invoice=info,tower_http=info,sea_orm_migration=info")
        }))
        .init();

    match command {
        Command::Serve => serve().await,
        Command::Migrate { action } => migrate(action).await,
        Command::Storage { action } => storage_cmd(action).await,
        Command::Design { action } => design_cmd(action).await,
        Command::Openapi => print_openapi(),
    }
}

fn print_openapi() -> Result<()> {
    use invoice::openapi::ApiDoc;
    use utoipa::OpenApi;
    let json = ApiDoc::openapi()
        .to_pretty_json()
        .context("serialize the OpenAPI document")?;
    println!("{json}");
    Ok(())
}

async fn connect(url: &str) -> Result<DatabaseConnection> {
    Database::connect(url)
        .await
        .context("connect to database (INVOICE__DATABASE_URL)")
}

async fn serve() -> Result<()> {
    let cfg = Config::from_env()?;
    // Migrations on their own connection: the pool's 30 s statement timeout
    // is for requests, not for a long schema change.
    let migrations = connect(cfg.database_url.expose()).await?;
    Migrator::up(&migrations, None)
        .await
        .context("apply pending migrations")?;
    migrations
        .close()
        .await
        .context("close migration connection")?;
    let db = Database::connect(db::pool_options(
        cfg.database_url.expose(),
        cfg.db_max_connections,
    ))
    .await
    .context("connect to database (INVOICE__DATABASE_URL)")?;

    let storage = Storage::new(&cfg.storage)?;
    tokio::spawn(storage.clone().probe());
    tokio::spawn(cleanup::run(db.clone()));
    let state = AppState {
        db,
        ares: AresClient::new(&cfg.ares_url)?,
        cnb: CnbClient::new(&cfg.cnb_url)?,
        pdf: PdfRoot::new(
            &cfg.mdcast_url,
            cfg.mdcast_token.as_ref().map(|t| t.expose().as_str()),
            storage,
        )?
        .with_design_cache_mb(cfg.design_cache_mb),
        email: cfg.smtp.as_ref().map(Mailer::new).transpose()?,
        public: cfg.public.clone(),
        email_disabled: cfg.email_disabled,
        registration: cfg.registration,
        max_spaces_per_user: cfg.max_spaces_per_user,
        trust_forwarded: cfg.trust_forwarded,
        secret_key: cfg.secret_key.clone(),
        limiter: RateLimiter::default(),
        google: cfg.google.clone().map(Google::new).transpose()?,
    };
    let listener = tokio::net::TcpListener::bind(&cfg.bind)
        .await
        .with_context(|| format!("bind {}", cfg.bind))?;
    tracing::info!(bind = %cfg.bind, base_url = %cfg.public.base_url(), "listening");
    let app = app::router(state).into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("http server")
}

async fn migrate(action: MigrateAction) -> Result<()> {
    let cfg = DbConfig::from_env()?;
    let db = connect(cfg.database_url.expose()).await?;
    match action {
        MigrateAction::Up => Migrator::up(&db, None).await,
        MigrateAction::Status => Migrator::status(&db).await,
        MigrateAction::Down { n } => Migrator::down(&db, Some(n)).await,
    }
    .context("run migrations")
}

fn open_storage() -> Result<Storage> {
    Storage::new(&StorageConfig::from_env()?)
}

fn finish(report: &Report) -> Result<()> {
    for key in &report.copied {
        println!("copied     {key}");
    }
    for problem in &report.problems {
        eprintln!("problem    {problem}");
    }
    println!("{}", report.summary());
    if !report.problems.is_empty() {
        bail!("{} file(s) not copied", report.problems.len());
    }
    Ok(())
}

async fn storage_cmd(action: StorageAction) -> Result<()> {
    let storage = open_storage()?;
    match action {
        StorageAction::Migrate {
            from_dir,
            design_dir,
        } => {
            let result =
                finish(&transfer::migrate(&storage, &from_dir, design_dir.as_deref()).await?);
            refresh_design_usage(&storage).await?;
            result
        }
    }
}

/// The copied keys may hold space files: record every space's design bytes
/// again (needs the database; without it, the next design listing does).
async fn refresh_design_usage(storage: &Storage) -> Result<()> {
    let Ok(cfg) = DbConfig::from_env() else {
        println!("design storage usage not refreshed (no INVOICE__DATABASE_URL)");
        return Ok(());
    };
    let db = connect(cfg.database_url.expose()).await?;
    let n = invoice::pdf::editor::refresh_all(&db, storage)
        .await
        .map_err(|e| anyhow::anyhow!("refresh the design storage usage: {e}"))?;
    println!("design storage usage refreshed for {n} space(s)");
    Ok(())
}

/// The space `slug` (looked up in the database) and its storage.
async fn space_storage(slug: &str) -> Result<(DatabaseConnection, space::SpaceId, Storage)> {
    let cfg = DbConfig::from_env()?;
    let db = connect(cfg.database_url.expose()).await?;
    let space = space::repo::find_by_slug(&db, slug)
        .await
        .map_err(|e| anyhow::anyhow!("look up space {slug}: {e}"))?
        .with_context(|| format!("no space with slug {slug}"))?;
    let space = space::SpaceId::from(&space);
    let storage = open_storage()?.scoped(&space.storage_prefix())?;
    Ok((db, space, storage))
}

async fn design_cmd(action: DesignAction) -> Result<()> {
    let (db, space, storage) = space_storage(action.space()).await?;
    let changes = matches!(action, DesignAction::Push { .. } | DesignAction::Rm { .. });
    let result = design_action(&storage, action).await;
    if changes {
        // Also after a partial failure: some files may have changed.
        invoice::pdf::editor::refresh_usage(&db, space, &storage).await;
    }
    result
}

async fn design_action(storage: &Storage, action: DesignAction) -> Result<()> {
    match action {
        DesignAction::Push { dir, .. } => {
            let mut report = Report::default();
            transfer::copy_tree(storage, &dir, Some(DESIGN_PREFIX), true, &mut report).await?;
            finish(&report)
        }
        DesignAction::Pull { dir, .. } => {
            for rel in transfer::pull(storage, DESIGN_PREFIX, &dir).await? {
                println!("{rel}");
            }
            Ok(())
        }
        DesignAction::Ls { .. } => {
            for f in design::list(storage).await? {
                let source = match f.source {
                    design::Source::Custom => "custom",
                    design::Source::Default => "default",
                };
                println!("{source:<8} {:>10}  {}", f.size, f.path);
            }
            Ok(())
        }
        DesignAction::Rm { paths, .. } => {
            let mut missing = 0;
            for path in paths {
                if design::remove(storage, &path).await? {
                    println!("removed    {path}");
                } else {
                    eprintln!("not found  {path}");
                    missing += 1;
                }
            }
            if missing > 0 {
                bail!("{missing} design file(s) not found");
            }
            Ok(())
        }
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %e, "install Ctrl+C handler");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => {
                tracing::error!(error = %e, "install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutting down");
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
