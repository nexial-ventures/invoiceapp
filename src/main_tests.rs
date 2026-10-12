use super::*;

#[test]
fn cli_parses_subcommands() {
    assert!(matches!(
        Cli::try_parse_from(["invoice", "serve"]).map(|c| c.command),
        Ok(Command::Serve)
    ));
    assert!(matches!(
        Cli::try_parse_from(["invoice", "migrate", "down", "-n", "3"]).map(|c| c.command),
        Ok(Command::Migrate {
            action: MigrateAction::Down { n: 3 }
        })
    ));
    assert!(matches!(
        Cli::try_parse_from(["invoice", "migrate", "down"]).map(|c| c.command),
        Ok(Command::Migrate {
            action: MigrateAction::Down { n: 1 }
        })
    ));
    assert!(Cli::try_parse_from(["invoice", "bogus"]).is_err());
    assert!(matches!(
        Cli::try_parse_from(["invoice", "openapi"]).map(|c| c.command),
        Ok(Command::Openapi)
    ));
    assert!(matches!(
        Cli::try_parse_from([
            "invoice", "storage", "migrate", "--from-dir", "/data", "--design-dir", "/design"
        ])
        .map(|c| c.command),
        Ok(Command::Storage {
            action: StorageAction::Migrate { from_dir, design_dir: Some(d) }
        }) if from_dir.as_path() == std::path::Path::new("/data") && d.as_path() == std::path::Path::new("/design")
    ));
    assert!(Cli::try_parse_from(["invoice", "storage", "migrate"]).is_err());
    assert!(matches!(
        Cli::try_parse_from([
            "invoice",
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
        Cli::try_parse_from(["invoice", "design", "ls", "--space", "firma"]).map(|c| c.command),
        Ok(Command::Design {
            action: DesignAction::Ls { space }
        }) if space == "firma"
    ));
    assert!(Cli::try_parse_from(["invoice", "design", "ls"]).is_err());
    assert!(Cli::try_parse_from(["invoice", "design", "rm"]).is_err());
}
