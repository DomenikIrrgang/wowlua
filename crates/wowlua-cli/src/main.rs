//! `wowlua`: compiler, package manager and tooling for World of Warcraft
//! AddOns. See the specification in `spec/` (chapter 12 for the CLI).

mod commands;
mod output;

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(
    name = "wowlua",
    version,
    about = "Compiler, package manager and tooling for World of Warcraft AddOns"
)]
struct Cli {
    #[command(flatten)]
    global: GlobalArgs,
    #[command(subcommand)]
    command: Command,
}

#[derive(Args, Debug, Clone)]
pub struct GlobalArgs {
    /// Use a specific wowlua.json
    #[arg(long, global = true, value_name = "PATH")]
    pub manifest: Option<PathBuf>,
    /// Fail if wowlua.lock would change
    #[arg(long, global = true)]
    pub locked: bool,
    /// Never access the network
    #[arg(long, global = true)]
    pub offline: bool,
    /// --locked plus --offline
    #[arg(long, global = true)]
    pub frozen: bool,
    /// Diagnostic output format
    #[arg(long, global = true, value_enum, default_value_t = Format::Human)]
    pub message_format: Format,
    /// Terminal colors
    #[arg(long, global = true, value_enum, default_value_t = Color::Auto)]
    pub color: Color,
    /// Print less output
    #[arg(short, long, global = true)]
    pub quiet: bool,
    /// Treat warnings as errors
    #[arg(long, global = true)]
    pub deny_warnings: bool,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Human,
    Short,
    Json,
    Github,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Auto,
    Always,
    Never,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Addon,
    Library,
    LuaLibrary,
    Types,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ci {
    Github,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Create a new package from a template
    New {
        /// Directory to create
        path: PathBuf,
        #[arg(long, value_enum, default_value_t = Kind::Addon)]
        kind: Kind,
        /// AddOn or library name (default: derived from the directory)
        #[arg(long)]
        name: Option<String>,
        /// Flavors to target, comma-separated (default: standard)
        #[arg(long, value_delimiter = ',')]
        flavors: Vec<String>,
        /// Add a CI workflow
        #[arg(long, value_enum)]
        ci: Option<Ci>,
    },
    /// Create wowlua.json for the existing addon in the current directory
    Init {
        /// The TOC file to adopt
        #[arg(long, value_name = "FILE")]
        from_toc: Option<PathBuf>,
        /// Convert .pkgmeta (arrives with the package manager)
        #[arg(long)]
        from_pkgmeta: bool,
    },
    /// Build the AddOn into dist/<profile>/<Name>/
    Build(BuildArgs),
    /// Check the sources without writing a build
    Check {
        #[command(flatten)]
        profile: ProfileArgs,
        /// Restrict to these flavors
        #[arg(long, value_delimiter = ',')]
        flavor: Vec<String>,
    },
    /// Format .wowlua files (and .lua files with `fmt.lua`)
    Fmt {
        /// Exit with status 1 if any file would change, without writing
        #[arg(long)]
        check: bool,
        /// Files or directories (default: the whole package)
        paths: Vec<PathBuf>,
    },
    /// Watch, build with the dev profile and sync the AddOn into WoW
    Dev {
        #[command(flatten)]
        target: InstallTarget,
        /// Link the AddOn folder to the build instead of copying
        #[arg(long)]
        link: bool,
    },
    /// Build once and install into WoW
    Install {
        #[command(flatten)]
        profile: ProfileArgs,
        #[command(flatten)]
        target: InstallTarget,
    },
    /// Remove installed builds of this addon from WoW
    Uninstall {
        /// Restrict to these client folders
        #[arg(long, value_delimiter = ',')]
        client: Vec<String>,
        /// The WoW installation directory
        #[arg(long, value_name = "PATH")]
        wow: Option<PathBuf>,
    },
    /// Remove the build output
    Clean,
    /// Print the TOC file(s) that would be generated
    Toc {
        #[command(flatten)]
        profile: ProfileArgs,
        /// Restrict to these flavors
        #[arg(long, value_delimiter = ',')]
        flavor: Vec<String>,
    },
    /// Run the lints only (W1003 unused, W1004 shadowing, W1005 unreachable)
    Lint {
        /// Apply machine-applicable fixes
        #[arg(long)]
        fix: bool,
    },
    /// Apply all machine-applicable fixes from diagnostics
    Fix {
        /// Migrate wowlua.json to the current schema
        #[arg(long)]
        manifest: bool,
    },
    /// Type declaration tools
    Types {
        #[command(subcommand)]
        command: TypesCommand,
    },
    /// Explain a diagnostic code
    Explain {
        /// For example E4002
        code: String,
    },
}

#[derive(Subcommand, Debug)]
enum TypesCommand {
    /// Convert LuaLS `---@` annotations into a .d.wowlua declaration file
    Convert {
        /// An annotated .lua file, or a directory of them
        path: PathBuf,
        /// Directory to write <name>.d.wowlua into (default: current directory)
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
    },
}

#[derive(Args, Debug, Clone)]
pub struct ProfileArgs {
    /// Build profile
    #[arg(long, conflicts_with = "release")]
    pub profile: Option<String>,
    /// Use the release profile
    #[arg(long)]
    pub release: bool,
}

impl ProfileArgs {
    pub fn name(&self, default: &str) -> String {
        match (&self.profile, self.release) {
            (Some(p), _) => p.clone(),
            (None, true) => "release".into(),
            (None, false) => default.into(),
        }
    }
}

#[derive(Args, Debug, Clone)]
pub struct BuildArgs {
    #[command(flatten)]
    pub profile: ProfileArgs,
    /// Restrict to these flavors
    #[arg(long, value_delimiter = ',')]
    pub flavor: Vec<String>,
    /// Write the AddOn folder here
    #[arg(long, value_name = "DIR")]
    pub out: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub struct InstallTarget {
    /// Restrict to these flavors
    #[arg(long, value_delimiter = ',')]
    pub flavor: Vec<String>,
    /// Restrict to these client folders (e.g. _retail_)
    #[arg(long, value_delimiter = ',')]
    pub client: Vec<String>,
    /// The WoW installation directory
    #[arg(long, value_name = "PATH")]
    pub wow: Option<PathBuf>,
    /// Replace AddOn folders that were not installed by wowlua
    #[arg(long)]
    pub force: bool,
}

fn main() -> ExitCode {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("error: internal compiler error: {info}");
        eprintln!("note: please report this at https://github.com/DomenikIrrgang/wowlua/issues");
        std::process::exit(101);
    }));
    let cli = Cli::parse();
    let out = output::Output::new(&cli.global);
    let ok = match cli.command {
        Command::New {
            path,
            kind,
            name,
            flavors,
            ci,
        } => commands::new(&out, path, kind, name, flavors, ci.is_some()),
        Command::Init {
            from_toc,
            from_pkgmeta,
        } => commands::init(&out, from_toc, from_pkgmeta),
        Command::Build(args) => commands::build(&out, &cli.global, &args),
        Command::Check { profile, flavor } => {
            commands::check(&out, &cli.global, &profile.name("dev"), flavor)
        }
        Command::Fmt { check, paths } => commands::fmt(&out, &cli.global, check, paths),
        Command::Dev { target, link } => commands::dev(&out, &cli.global, &target, link),
        Command::Install { profile, target } => {
            commands::install(&out, &cli.global, &profile.name("dev"), &target)
        }
        Command::Uninstall { client, wow } => commands::uninstall(&out, &cli.global, client, wow),
        Command::Clean => commands::clean(&out, &cli.global),
        Command::Toc { profile, flavor } => {
            commands::toc(&out, &cli.global, &profile.name("dev"), flavor)
        }
        Command::Lint { fix } => commands::lint(&out, &cli.global, fix),
        Command::Fix { manifest } => commands::fix(&out, &cli.global, manifest),
        Command::Types {
            command: TypesCommand::Convert { path, out: dir },
        } => commands::types_convert(&out, &path, dir.as_deref()),
        Command::Explain { code } => commands::explain(&out, &code),
    };
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
