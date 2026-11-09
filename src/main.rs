mod addon;
mod config;
mod flavor;
mod installer;
mod package;
mod toc;
mod ui;
mod wow;

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};

use crate::config::Config;

#[derive(Parser)]
#[command(
    name = "wowmod",
    about = "Install and manage World of Warcraft addons from a terminal"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Install a zip or folder without opening the UI.
    Install {
        /// Zip file or folder containing the addon.
        source: PathBuf,
        /// Flavor code (wow, wowf, wowt, wow_classic_era...) or `_folder_` name.
        #[arg(short, long)]
        flavor: Option<String>,
        /// Delete replaced folders instead of backing them up.
        #[arg(long)]
        no_backup: bool,
        /// Extra World of Warcraft folder to scan (repeatable).
        #[arg(long)]
        root: Vec<PathBuf>,
    },
    /// List detected WoW installs and their addons.
    List {
        /// Extra World of Warcraft folder to scan (repeatable).
        #[arg(long)]
        root: Vec<PathBuf>,
    },
}

fn roots(config: &Config, extra: &[PathBuf]) -> Vec<PathBuf> {
    config.wow_roots.iter().chain(extra).cloned().collect()
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load();
    match cli.command {
        None => ui::run(config),
        Some(Command::List { root }) => {
            let installs = wow::discover(&roots(&config, &root));
            if installs.is_empty() {
                println!("No World of Warcraft installs found.");
            }
            for i in &installs {
                println!("{} [{}]  {}", i.label(), i.flavor.code, i.path.display());
                for a in addon::scan(i) {
                    let extra = if a.members.is_empty() {
                        String::new()
                    } else {
                        format!(" (+{} folders)", a.members.len())
                    };
                    println!(
                        "    {:<40} {:<12} {}{}",
                        a.title(),
                        a.version(),
                        a.primary.enabled.short(),
                        extra
                    );
                }
            }
            Ok(())
        }
        Some(Command::Install {
            source,
            flavor,
            no_backup,
            root,
        }) => {
            let installs = wow::discover(&roots(&config, &root));
            let target = match flavor {
                Some(f) => {
                    let matches: Vec<_> = installs
                        .iter()
                        .filter(|i| {
                            i.flavor.code == f
                                || i.folder_name() == f
                                || i.path.to_string_lossy() == f
                        })
                        .cloned()
                        .collect();
                    match matches.len() {
                        1 => matches.into_iter().next().unwrap(),
                        0 => bail!("no install matching {f}"),
                        _ => bail!(
                            "{f} matches several installs, pass the full path as --flavor:
  {}",
                            matches
                                .iter()
                                .map(|i| i.path.display().to_string())
                                .collect::<Vec<_>>()
                                .join(
                                    "
  "
                                )
                        ),
                    }
                }
                None if installs.len() == 1 => installs[0].clone(),
                None => bail!(
                    "several installs found, pass --flavor one of: {}",
                    installs
                        .iter()
                        .map(|i| i.flavor.code.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
            let pkg = package::Package::analyze(&source, Some(&target.flavor))?;
            let opts = installer::Options {
                backups: !no_backup && config.backups,
            };
            let report = installer::install(&pkg, &target, &opts)?;
            for r in &report.installed {
                let what = if report.replaced.contains(&r.folder) {
                    "replaced"
                } else {
                    "installed"
                };
                println!(
                    "{what} {} {} -> {}",
                    r.folder,
                    r.version.clone().unwrap_or_default(),
                    target.label()
                );
            }
            Ok(())
        }
    }
}
