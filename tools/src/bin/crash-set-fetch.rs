//! Fetches a crash set from crash-stats into a crash sets' store (see
//! tools::crash_sets), for development and tests' fixtures; the web server's
//! ingestion (to come; see asuth-gecko-notes' crash-sets-plan.md) does the
//! same with a user's token.  Prints the set's ID.
//!
//! An unprivileged crash-stats API token (checked) can be in the file
//! `$SOCORRO_API_TOKEN_PATH` (as socorro-cli's), for its rate limits.
//!
//! With `--manifest FILE`, it stores that manifest (see
//! `tools::crash_sets::CrashSetManifest`) instead, for tests' fixtures, whose
//! crashes are already in the store.

use clap::Parser;
use tools::crash_sets::{CRASH_STATS_API, CrashSearch, CrashSetStore, fetch_set};

#[derive(Debug, Parser)]
struct Args {
    /// The store's directory.
    #[clap(long)]
    store: String,
    /// The signature, exactly.
    #[clap(long)]
    signature: Option<String>,
    /// A piece of the proto signatures, which contain it.
    #[clap(long)]
    proto_signature: Option<String>,
    #[clap(long, default_value = "Firefox")]
    product: String,
    #[clap(long)]
    release_channel: Vec<String>,
    #[clap(long)]
    version: Vec<String>,
    /// Ex: "Windows NT", "Mac OS X", "Linux".
    #[clap(long)]
    platform: Vec<String>,
    /// Ex: "parent", "content".
    #[clap(long)]
    process_type: Vec<String>,
    /// The first day (YYYY-MM-DD), else `--days` before the last.
    #[clap(long)]
    from: Option<String>,
    /// The day after the last (YYYY-MM-DD), else tomorrow.
    #[clap(long)]
    to: Option<String>,
    #[clap(long, default_value = "7")]
    days: i64,
    /// How many crashes (the latest).
    #[clap(long, default_value = "100")]
    limit: usize,
    /// crash-stats' API.
    #[clap(long, default_value = CRASH_STATS_API)]
    api: String,
    /// A manifest to store, rather than a search to fetch.
    #[clap(long)]
    manifest: Option<String>,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if let Some(path) = &args.manifest {
        let manifest = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|text| serde_json::from_str(&text).map_err(|e| e.to_string()))
            .and_then(|manifest| CrashSetStore::new(&args.store).store_manifest(&manifest));
        match manifest {
            Ok(id) => println!("{}", id),
            Err(message) => {
                eprintln!("{}", message);
                std::process::exit(1);
            }
        }
        return;
    }
    let to = args.to.unwrap_or_else(|| {
        (chrono::Utc::now().date_naive() + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string()
    });
    let from = args.from.unwrap_or_else(|| {
        chrono::NaiveDate::parse_from_str(&to, "%Y-%m-%d")
            .map(|to| {
                (to - chrono::Duration::days(args.days))
                    .format("%Y-%m-%d")
                    .to_string()
            })
            .unwrap_or_default()
    });
    let search = CrashSearch {
        signature: args.signature,
        proto_signature: args.proto_signature,
        product: args.product,
        release_channels: args.release_channel,
        versions: args.version,
        platforms: args.platform,
        process_types: args.process_type,
        date_from: from,
        date_to: to,
        limit: args.limit,
    };
    let token = std::env::var("SOCORRO_API_TOKEN_PATH")
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|token| token.trim().to_string());
    match fetch_set(
        &CrashSetStore::new(&args.store),
        &search,
        &args.api,
        token.as_deref(),
    )
    .await
    {
        Ok(id) => println!("{}", id),
        Err(message) => {
            eprintln!("{}", message);
            std::process::exit(1);
        }
    }
}
