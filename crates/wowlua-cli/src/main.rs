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
    /// Select a workspace member
    #[arg(short = 'p', long, global = true, value_name = "NAME")]
    pub package: Option<String>,
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
    /// Worker threads for per-file work (1: none)
    #[arg(short = 'j', long, global = true, value_name = "N")]
    pub jobs: Option<usize>,
}

impl GlobalArgs {
    /// `--jobs`, else `WOWLUA_JOBS`, else the config, else the default.
    pub fn jobs(&self) -> wowlua_driver::parallel::Jobs {
        wowlua_driver::parallel::Jobs::resolve(self.jobs)
    }
}

#[derive(Args, Debug, Clone)]
pub struct PublishArgs {
    /// Only these publish targets
    #[arg(long = "target", value_name = "NAME")]
    pub targets: Vec<String>,
    /// Package and print what each target would receive, without uploading
    #[arg(long)]
    pub dry_run: bool,
    /// Tag HEAD (locally) and publish it as that tag
    #[arg(long, value_name = "TAG")]
    pub tag: Option<String>,
    /// Release type instead of the manifest's
    #[arg(long, value_enum)]
    pub release_type: Option<ReleaseTypeArg>,
    /// Publish with uncommitted changes (an alpha build)
    #[arg(long)]
    pub allow_dirty: bool,
    /// Skip the tests
    #[arg(long)]
    pub no_test: bool,
    /// Restrict to these flavors
    #[arg(long, value_delimiter = ',')]
    pub flavor: Vec<String>,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Site {
    Curseforge,
    Wago,
    Wowinterface,
    Github,
    Registry,
}

impl Site {
    pub fn id(self) -> &'static str {
        match self {
            Site::Curseforge => "curseforge",
            Site::Wago => "wago",
            Site::Wowinterface => "wowinterface",
            Site::Github => "github",
            Site::Registry => "registry",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Site::Curseforge => "CurseForge",
            Site::Wago => "Wago",
            Site::Wowinterface => "WoWInterface",
            Site::Github => "GitHub",
            Site::Registry => "wowlua registry",
        }
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseTypeArg {
    Alpha,
    Beta,
    Release,
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
        /// Require and convert .pkgmeta (converted whenever it exists), or
        /// the given packager project file
        #[arg(long, value_name = "FILE", num_args = 0..=1, default_missing_value = "")]
        from_pkgmeta: Option<String>,
    },
    /// Build the AddOn into dist/<profile>/<Name>/
    Build(BuildArgs),
    /// Build every publish target and package its zip(s) into dist/packages/
    Package {
        /// Only these publish targets
        #[arg(long = "target", value_name = "NAME")]
        targets: Vec<String>,
        /// Restrict to these flavors
        #[arg(long, value_delimiter = ',')]
        flavor: Vec<String>,
    },
    /// Package and upload to the publish targets
    Publish(PublishArgs),
    /// Save an API token for a publish site in the OS credential store
    Login {
        /// The site
        #[arg(value_enum)]
        site: Site,
        /// The token (default: read from stdin, or a hidden prompt)
        #[arg(long)]
        token: Option<String>,
        /// Save without checking the token against the site
        #[arg(long)]
        no_verify: bool,
    },
    /// Remove a saved API token
    Logout {
        /// The site
        #[arg(value_enum)]
        site: Site,
    },
    /// Check the sources without writing a build
    Check {
        #[command(flatten)]
        profile: ProfileArgs,
        /// Restrict to these flavors
        #[arg(long, value_delimiter = ',')]
        flavor: Vec<String>,
    },
    /// Compile files to Lua without making an AddOn folder
    Compile(CompileArgs),
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
        /// Stop after this many builds (for measuring)
        #[arg(long, hide = true, value_name = "N")]
        exit_after: Option<usize>,
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
    /// Map error messages and stacks from the game back to source locations
    Trace {
        /// A file with the error text (default: standard input)
        file: Option<PathBuf>,
        /// Read the error text from the clipboard
        #[arg(long, conflicts_with = "file")]
        clipboard: bool,
        /// The build id the error came from (`X-Wowlua-Build`)
        #[arg(long)]
        build: Option<String>,
    },
    /// Add a dependency and update the lockfile
    Add(Box<AddArgs>),
    /// Remove a dependency
    Remove {
        name: String,
        /// Remove it from dev-dependencies
        #[arg(long)]
        dev: bool,
    },
    /// Update locked dependency pins within their declarations
    Update {
        /// Only these dependencies (default: all)
        names: Vec<String>,
        /// Pin one dependency to this git commit or SVN revision
        #[arg(long, value_name = "PIN")]
        precise: Option<String>,
    },
    /// Download all dependencies into the cache
    Fetch,
    /// Show the dependency graph
    Tree {
        /// Only dependencies of this flavor
        #[arg(long)]
        flavor: Option<String>,
        /// Show what depends on this package
        #[arg(long, value_name = "NAME")]
        invert: Option<String>,
    },
    /// List dependencies with newer tags, commits or revisions
    Outdated,
    /// Manage the global package cache
    Cache {
        #[command(subcommand)]
        command: CacheCommand,
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
enum CacheCommand {
    /// Remove cache entries not used for 30 days
    Clean {
        /// Remove everything
        #[arg(long)]
        all: bool,
    },
}

#[derive(Args, Debug, Clone)]
pub struct AddArgs {
    /// The dependency name (`name@<req>` for a registry requirement), or
    /// the AddOn name with --addon
    pub name: String,
    /// Add an addon-dependencies entry instead
    #[arg(long)]
    pub addon: bool,
    #[arg(long, value_name = "URL")]
    pub git: Option<String>,
    #[arg(long, requires = "git", conflicts_with_all = ["branch", "rev"])]
    pub tag: Option<String>,
    #[arg(long, requires = "git", conflicts_with = "rev")]
    pub branch: Option<String>,
    /// A git revision or an SVN revision number
    #[arg(long)]
    pub rev: Option<String>,
    #[arg(long, value_name = "URL", conflicts_with = "git")]
    pub svn: Option<String>,
    #[arg(long, value_name = "PATH", conflicts_with_all = ["git", "svn"])]
    pub path: Option<String>,
    #[arg(long, value_name = "URL", conflicts_with_all = ["git", "svn", "path"], requires = "sha256")]
    pub archive: Option<String>,
    #[arg(long, requires = "archive")]
    pub sha256: Option<String>,
    #[arg(long, requires = "archive")]
    pub strip_prefix: Option<String>,
    #[arg(long, requires = "git")]
    pub subdir: Option<String>,
    /// The package kind (needed for sources without a wowlua.json)
    #[arg(long, value_enum)]
    pub kind: Option<Kind>,
    /// lua-library entry files, in load order
    #[arg(long, value_delimiter = ',')]
    pub files: Vec<String>,
    /// Folder under build.libs-dir
    #[arg(long)]
    pub install_path: Option<String>,
    /// Add to dev-dependencies
    #[arg(long)]
    pub dev: bool,
    /// Only included when a feature enables it (with --addon: OptionalDeps)
    #[arg(long)]
    pub optional: bool,
    /// Only for these flavors
    #[arg(long, value_delimiter = ',')]
    pub flavors: Vec<String>,
    /// Features of the dependency to enable
    #[arg(long, value_delimiter = ',')]
    pub features: Vec<String>,
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
pub struct CompileArgs {
    /// `.wowlua` and `.lua` files, and `.d.wowlua` declaration files
    #[arg(required = true)]
    pub files: Vec<PathBuf>,
    #[command(flatten)]
    pub profile: ProfileArgs,
    /// Restrict to these flavors
    #[arg(long, value_delimiter = ',')]
    pub flavor: Vec<String>,
    /// Bundle into one file (the default of release profiles)
    #[arg(long, conflicts_with = "no_bundle")]
    pub bundle: bool,
    /// Keep one output file per source
    #[arg(long)]
    pub no_bundle: bool,
    /// Minification level (default: the profile's)
    #[arg(long, value_name = "LEVEL")]
    pub minify: Option<String>,
    /// Output file, or directory for several files (default: stdout when
    /// there is one output file)
    #[arg(short, long, value_name = "PATH")]
    pub out: Option<PathBuf>,
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
    /// Report the output's files, sizes and Lua compile time
    #[arg(long)]
    pub stats: bool,
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
        // A job's panic is caught: its region runs again in order, which
        // reports it here as a sequential build would.
        if wowlua_driver::parallel::in_job() {
            wowlua_driver::parallel::record_panic(info.to_string());
            return;
        }
        match info
            .payload()
            .downcast_ref::<wowlua_driver::LuaOutOfMemory>()
        {
            Some(oom) => eprintln!("error: internal compiler error: {oom}"),
            None => eprintln!("error: internal compiler error: {info}"),
        }
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
        Command::Package { targets, flavor } => {
            commands::package(&out, &cli.global, targets, flavor)
        }
        Command::Publish(args) => commands::publish(&out, &cli.global, args),
        Command::Login {
            site,
            token,
            no_verify,
        } => commands::login(&out, site, token, no_verify),
        Command::Logout { site } => commands::logout(&out, site),
        Command::Check { profile, flavor } => {
            commands::check(&out, &cli.global, &profile.name("dev"), flavor)
        }
        Command::Compile(args) => commands::compile(&out, &cli.global, &args),
        Command::Fmt { check, paths } => commands::fmt(&out, &cli.global, check, paths),
        Command::Dev {
            target,
            link,
            exit_after,
        } => commands::dev(&out, &cli.global, &target, link, exit_after),
        Command::Install { profile, target } => {
            commands::install(&out, &cli.global, &profile.name("dev"), &target)
        }
        Command::Uninstall { client, wow } => commands::uninstall(&out, &cli.global, client, wow),
        Command::Clean => commands::clean(&out, &cli.global),
        Command::Toc { profile, flavor } => {
            commands::toc(&out, &cli.global, &profile.name("dev"), flavor)
        }
        Command::Lint { fix } => commands::lint(&out, &cli.global, fix),
        Command::Trace {
            file,
            clipboard,
            build,
        } => commands::trace(
            &out,
            &cli.global,
            file.as_deref(),
            clipboard,
            build.as_deref(),
        ),
        Command::Fix { manifest } => commands::fix(&out, &cli.global, manifest),
        Command::Add(args) => commands::add(&out, &cli.global, &args),
        Command::Remove { name, dev } => commands::remove(&out, &cli.global, &name, dev),
        Command::Update { names, precise } => commands::update(&out, &cli.global, names, precise),
        Command::Fetch => commands::fetch(&out, &cli.global),
        Command::Tree { flavor, invert } => {
            commands::tree(&out, &cli.global, flavor.as_deref(), invert.as_deref())
        }
        Command::Outdated => commands::outdated(&out, &cli.global),
        Command::Cache {
            command: CacheCommand::Clean { all },
        } => commands::cache_clean(&out, all),
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
