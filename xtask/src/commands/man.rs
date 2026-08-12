use std::path::PathBuf;

use clap::Args;

#[derive(Args, Debug)]
pub struct ManArgs {
    /// Output directory (default: dist/share/man/man1)
    #[arg(long = "out-dir", default_value = "dist/share/man/man1")]
    pub out_dir: PathBuf,
}

pub fn cmd_man(args: ManArgs) -> Result<(), String> {
    let out_dir = crate::workspace_root().join(args.out_dir);

    // Renders the same augmented tree `librebar::cli::parse_with` uses, so the
    // pages cover `schema` and `completions` too. Filenames carry the full
    // hyphenated command path, so equally named leaves in different subtrees
    // cannot overwrite each other. Creates the output directory itself, and
    // skips Clap's generated `help` subcommand.
    let paths = librebar::cli::generate_manpages::<colophon::Cli>(&out_dir)
        .map_err(|error| format!("generate manpages in {}: {error}", out_dir.display()))?;

    for path in paths {
        println!("wrote {}", path.display());
    }

    Ok(())
}
