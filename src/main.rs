use std::net::SocketAddr;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use invoiceapp::app::{self, AppState};
use invoiceapp::ares::AresClient;
use invoiceapp::auth::google::Google;
use invoiceapp::auth::ratelimit::RateLimiter;
use invoiceapp::cleanup;
use invoiceapp::cnb::CnbClient;
use invoiceapp::config::{Config, DbConfig};
use invoiceapp::email::Mailer;
use invoiceapp::email::sandbox::{self, TemplateSandbox};
use invoiceapp::migration::{Migrator, MigratorTrait};
use invoiceapp::pdf::PdfRoot;
use invoiceapp::pdf::limits::RenderLimits;
use invoiceapp::storage::{Storage, sweep};
use sea_orm::{Database, DatabaseConnection};
use tracing_subscriber::EnvFilter;

#[path = "main_storage.rs"]
mod storage_cli;
use storage_cli::{DesignAction, StorageAction, design_cmd, storage_cmd};

#[derive(Parser)]
#[command(
    name = "invoiceapp",
    version,
    about = "InvoiceApp — invoice management (spaces, Czech VAT)"
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
    /// File storage maintenance (the configured INVOICEAPP__STORAGE_KIND backend).
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
    /// Internal: render one e-mail template job (stdin → stdout) under the
    /// limits the server's sandbox sets.
    #[command(hide = true, name = "render-template")]
    RenderTemplate,
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

fn main() -> Result<()> {
    let command = Cli::parse().command;
    // stdout carries the JSON document: no subscriber, so no log line can mix into it.
    if matches!(command, Command::Openapi) {
        return print_openapi();
    }
    // Before any runtime or logger: one thread, stdout is the protocol.
    if matches!(command, Command::RenderTemplate) {
        return sandbox::child_main();
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("start the async runtime")?
        .block_on(run(command))
}

async fn run(command: Command) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("invoiceapp=info,tower_http=info,sea_orm_migration=info")
        }))
        .init();

    match command {
        Command::Serve => serve().await,
        Command::Migrate { action } => migrate(action).await,
        Command::Storage { action } => storage_cmd(action).await,
        Command::Design { action } => design_cmd(action).await,
        Command::Openapi => print_openapi(),
        Command::RenderTemplate => sandbox::child_main(),
    }
}

fn print_openapi() -> Result<()> {
    use invoiceapp::openapi::ApiDoc;
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
        .context("connect to database (INVOICEAPP__DATABASE_URL)")
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
    let db = Database::connect(cfg.pool_options())
        .await
        .context("connect to database (INVOICEAPP__DATABASE_URL)")?;

    let storage = Storage::new(&cfg.storage)?;
    tokio::spawn(storage.clone().probe());
    tokio::spawn(cleanup::run(db.clone()));
    tokio::spawn(sweep::run(db.clone(), storage.clone()));
    let state = AppState {
        db,
        ares: AresClient::new(&cfg.ares_url)?,
        cnb: CnbClient::new(&cfg.cnb_url)?,
        pdf: PdfRoot::new(
            &cfg.mdcast_url,
            cfg.mdcast_token.as_ref().map(|t| t.expose().as_str()),
            storage,
        )?
        .with_design_cache_mb(cfg.design_cache_mb)
        .with_render_limits(RenderLimits::new(cfg.mdcast_concurrency)),
        email: cfg.smtp.as_ref().map(Mailer::new).transpose()?,
        public: cfg.public.clone(),
        email_disabled: cfg.email_disabled,
        registration: cfg.registration,
        max_spaces_per_user: cfg.max_spaces_per_user,
        trust_forwarded: cfg.trust_forwarded,
        secret_key: cfg.secret_key.clone(),
        limiter: RateLimiter::default(),
        templates: TemplateSandbox::current()?,
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
