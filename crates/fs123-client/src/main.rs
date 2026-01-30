//! fs123 FUSE filesystem client.
//!
//! Mount a remote fs123 server as a local filesystem.

use clap::Parser;
use fuser::MountOption;
use log::info;

mod cache;
mod fs;
mod inode;

use cache::CacheConfig;
use fs::Fs123Filesystem;

/// fs123 FUSE filesystem client
#[derive(Parser, Debug)]
#[command(name = "mount.fs123", version, about)]
struct Args {
    /// Server hostname or IP address
    #[arg(short = 'H', long)]
    host: String,

    /// Server port
    #[arg(short, long, default_value = "80")]
    port: u16,

    /// Protocol version (e.g., "7.3")
    #[arg(long, default_value = "7.3")]
    proto: String,

    /// Mount point
    mountpoint: String,

    /// Allow other users to access the filesystem
    #[arg(long)]
    allow_other: bool,

    /// Allow root to access the filesystem
    #[arg(long)]
    allow_root: bool,

    /// Enable auto unmount on process exit
    #[arg(long)]
    auto_unmount: bool,

    /// Read-only mount (always true for fs123, but can be explicit)
    #[arg(short, long)]
    read_only: bool,

    /// Default permissions checking (recommended)
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    default_permissions: bool,

    /// Foreground operation (don't daemonize)
    #[arg(short, long)]
    foreground: bool,

    /// Enable debug logging
    #[arg(short, long)]
    debug: bool,

    /// Kernel cache timeout in seconds (default: 1.0)
    #[arg(long, default_value = "1.0")]
    attr_timeout: f64,

    /// Entry cache timeout in seconds (default: 1.0)
    #[arg(long, default_value = "1.0")]
    entry_timeout: f64,

    /// Enable application-level caching
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    cache: bool,

    /// Maximum attribute cache entries
    #[arg(long, default_value = "10000")]
    cache_max_attrs: usize,

    /// Maximum directory cache entries
    #[arg(long, default_value = "1000")]
    cache_max_dirs: usize,

    /// Maximum symlink cache entries
    #[arg(long, default_value = "5000")]
    cache_max_links: usize,

    /// Default cache TTL in seconds (max-age)
    #[arg(long, default_value = "60")]
    cache_ttl: u64,

    /// Stale-while-revalidate window in seconds
    #[arg(long, default_value = "300")]
    cache_swr: u64,

    /// Enable background cache refresh
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    cache_background_refresh: bool,

    /// Number of background refresh threads
    #[arg(long, default_value = "2")]
    cache_refresh_threads: usize,
}

fn main() {
    let args = Args::parse();

    // Initialize logging
    if args.debug {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();
    } else {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    }

    info!(
        "Mounting fs123 filesystem from {}:{} at {}",
        args.host, args.port, args.mountpoint
    );

    // Build mount options
    let mut options = vec![
        MountOption::RO, // fs123 is read-only
        MountOption::FSName(format!("fs123://{}:{}", args.host, args.port)),
        MountOption::Subtype("fs123".to_string()),
    ];

    if args.allow_other {
        options.push(MountOption::AllowOther);
    }
    if args.allow_root {
        options.push(MountOption::AllowRoot);
    }
    if args.auto_unmount {
        options.push(MountOption::AutoUnmount);
    }
    if args.default_permissions {
        options.push(MountOption::DefaultPermissions);
    }

    // Build cache configuration
    let cache_config = if args.cache {
        Some(CacheConfig {
            max_attr_entries: args.cache_max_attrs,
            max_dir_entries: args.cache_max_dirs,
            max_symlink_entries: args.cache_max_links,
            default_max_age: std::time::Duration::from_secs(args.cache_ttl),
            default_swr: std::time::Duration::from_secs(args.cache_swr),
            enable_background_refresh: args.cache_background_refresh,
            refresh_threads: args.cache_refresh_threads,
        })
    } else {
        None
    };

    // Create filesystem
    let fs = Fs123Filesystem::new(
        &args.host,
        args.port,
        &args.proto,
        std::time::Duration::from_secs_f64(args.attr_timeout),
        std::time::Duration::from_secs_f64(args.entry_timeout),
        cache_config,
    );

    // Mount
    if let Err(e) = fuser::mount2(fs, &args.mountpoint, &options) {
        eprintln!("Error mounting filesystem: {}", e);
        std::process::exit(1);
    }
}
