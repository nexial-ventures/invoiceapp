//! `invoice storage …` and `invoice design …`.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use invoice::config::DbConfig;
use invoice::pdf::design;
use invoice::space;
use invoice::storage::sweep;
use invoice::storage::transfer::{self, Report};
use invoice::storage::{DESIGN_PREFIX, Storage, StorageConfig};
use sea_orm::DatabaseConnection;

use super::connect;

#[derive(Subcommand)]
pub enum StorageAction {
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
    /// Delete document objects no document references, older than an hour
    /// (also done daily by `serve`). Needs the database.
    Sweep {
        /// Only list what would be deleted.
        #[arg(long)]
        dry_run: bool,
        /// Delete even when the guard refuses (no references at all, or
        /// more than max(100, 10 %) orphans).
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
pub enum DesignAction {
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

pub async fn storage_cmd(action: StorageAction) -> Result<()> {
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
        StorageAction::Sweep { dry_run, force } => {
            let cfg = DbConfig::from_env()?;
            let db = connect(cfg.database_url.expose()).await?;
            let opts = sweep::Options {
                cutoff: sweep::cutoff_now()?,
                dry_run,
                force,
            };
            let swept = sweep::sweep(&db, &storage, opts).await?;
            report_sweep(&swept, dry_run)
        }
    }
}

fn report_sweep(swept: &sweep::Swept, dry_run: bool) -> Result<()> {
    if dry_run || swept.refused.is_some() && swept.deleted.is_empty() {
        for key in &swept.orphans {
            println!("orphan     {key}");
        }
    }
    for key in &swept.deleted {
        println!("deleted    {key}");
    }
    for key in &swept.failed {
        eprintln!("failed     {key}");
    }
    println!(
        "{} document object(s), {} orphan(s), {} deleted, {} failed, {} too young",
        swept.listed,
        swept.orphans.len(),
        swept.deleted.len(),
        swept.failed.len(),
        swept.young
    );
    if let Some(why) = swept
        .refused
        .as_ref()
        .filter(|_| swept.deleted.is_empty() && !dry_run)
    {
        bail!("nothing deleted: {why} (check, then re-run with --force)");
    }
    if !swept.failed.is_empty() {
        bail!("{} orphan(s) not deleted", swept.failed.len());
    }
    Ok(())
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

pub async fn design_cmd(action: DesignAction) -> Result<()> {
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
