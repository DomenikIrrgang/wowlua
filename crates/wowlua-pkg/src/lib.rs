//! The wowlua package manager (spec chapter 10): dependency sources, the
//! global cache, fetching with the system `git` and `svn` clients and over
//! HTTPS, the wowlua registry's protocol, checksums, PubGrub resolution,
//! and the lockfile entries.
//!
//! No code of a dependency runs here: packages are only downloaded,
//! extracted and checked (spec §10.11).

pub mod cache;
pub mod checksum;
pub mod fetch;
pub mod lock;
pub mod registry;
pub mod resolve;
pub mod safety;
pub mod source;
pub mod verbose;

pub use cache::Cache;
pub use fetch::{FetchError, Fetcher};
pub use lock::LockedPackage;
pub use resolve::{Resolution, ResolvedPackage, Root, Update, resolve};
pub use source::{GitRef, Source};
