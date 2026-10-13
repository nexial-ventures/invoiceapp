//! Unit tests of `main.rs` (CLI parsing).

use super::*;

#[test]
fn cli_parses_subcommands() {
    assert!(matches!(
        Cli::try_parse_from(["invoiceapp", "serve"]).map(|c| c.command),
        Ok(Command::Serve)
    ));
    assert!(matches!(
        Cli::try_parse_from(["invoiceapp", "migrate", "down", "-n", "3"]).map(|c| c.command),
        Ok(Command::Migrate {
            action: MigrateAction::Down { n: 3 }
        })
    ));
    assert!(matches!(
        Cli::try_parse_from(["invoiceapp", "migrate", "down"]).map(|c| c.command),
        Ok(Command::Migrate {
            action: MigrateAction::Down { n: 1 }
        })
    ));
    assert!(Cli::try_parse_from(["invoiceapp", "bogus"]).is_err());
    assert!(matches!(
        Cli::try_parse_from(["invoiceapp", "openapi"]).map(|c| c.command),
        Ok(Command::Openapi)
    ));
    assert!(matches!(
        Cli::try_parse_from([
            "invoiceapp", "storage", "migrate", "--from-dir", "/data", "--design-dir", "/design"
        ])
        .map(|c| c.command),
        Ok(Command::Storage {
            action: StorageAction::Migrate { from_dir, design_dir: Some(d) }
        }) if from_dir.as_path() == std::path::Path::new("/data") && d.as_path() == std::path::Path::new("/design")
    ));
    assert!(Cli::try_parse_from(["invoiceapp", "storage", "migrate"]).is_err());
    assert!(matches!(
        Cli::try_parse_from(["invoiceapp", "storage", "sweep", "--dry-run"]).map(|c| c.command),
        Ok(Command::Storage {
            action: StorageAction::Sweep {
                dry_run: true,
                force: false
            }
        })
    ));
    assert!(matches!(
        Cli::try_parse_from(["invoiceapp", "storage", "sweep", "--force"]).map(|c| c.command),
        Ok(Command::Storage {
            action: StorageAction::Sweep {
                dry_run: false,
                force: true
            }
        })
    ));
    assert!(matches!(
        Cli::try_parse_from([
            "invoiceapp",
            "design",
            "push",
            "./my-design",
            "--space",
            "firma"
        ])
        .map(|c| c.command),
        Ok(Command::Design {
            action: DesignAction::Push { .. }
        })
    ));
    assert!(matches!(
        Cli::try_parse_from(["invoiceapp", "design", "ls", "--space", "firma"]).map(|c| c.command),
        Ok(Command::Design {
            action: DesignAction::Ls { space }
        }) if space == "firma"
    ));
    assert!(Cli::try_parse_from(["invoiceapp", "design", "ls"]).is_err());
    assert!(Cli::try_parse_from(["invoiceapp", "design", "rm"]).is_err());
    assert!(matches!(
        Cli::try_parse_from(["invoiceapp", "render-template"]).map(|c| c.command),
        Ok(Command::RenderTemplate)
    ));
}
