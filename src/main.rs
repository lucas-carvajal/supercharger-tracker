mod coming_soon;
mod display;
mod loaders;
mod raw;
mod supercharger;

use clap::Parser;

use coming_soon::ComingSoonSupercharger;
use display::{print_coming_soon, print_superchargers};
use supercharger::Supercharger;

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "tesla-superchargers",
    version,
    about = "Fetch Tesla Supercharger locations"
)]
struct Args {
    /// Read from a local JSON file instead of fetching live data.
    #[arg(short, long, value_name = "PATH")]
    file: Option<String>,

    /// Use a raw cookie string instead of launching a browser.
    /// Can also be set via TESLA_COOKIE env var.
    #[arg(short, long, value_name = "COOKIE_STRING", env = "TESLA_COOKIE")]
    cookie: Option<String>,

    /// Country code (default: US — actually returns worldwide data).
    #[arg(long, default_value = "US")]
    country: String,

    /// Show the browser window while fetching (default: headless).
    #[arg(long)]
    show_browser: bool,

    /// Also print the table of open superchargers.
    #[arg(long)]
    show_open: bool,
}

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let result = if let Some(ref path) = args.file {
        loaders::load_from_file(path).await?
    } else if let Some(ref cookie) = args.cookie {
        loaders::load_with_cookie(&args.country, cookie).await?
    } else {
        loaders::load_from_browser(&args.country, args.show_browser).await?
    };

    let open: Vec<Supercharger> = result
        .locations
        .iter()
        .filter(|l| Supercharger::is_open_supercharger(l))
        .map(Supercharger::from)
        .collect();

    let coming_soon: Vec<ComingSoonSupercharger> = result
        .locations
        .iter()
        .filter(|l| ComingSoonSupercharger::is_coming_soon(l))
        .map(|l| {
            let details = result.coming_soon_details.get(&l.location_url_slug);
            ComingSoonSupercharger::from_location(l, details)
        })
        .collect();

    println!();
    println!("Total locations (all types) : {}", result.locations.len());
    println!("Open superchargers          : {}", open.len());
    println!("Coming soon superchargers   : {}", coming_soon.len());

    if args.show_open {
        println!();
        print_superchargers("OPEN SUPERCHARGERS", &open);
    }

    println!();
    print_coming_soon("COMING SOON SUPERCHARGERS", &coming_soon);

    println!("\nNote: country=US returns worldwide data — no need to repeat per country.");
    Ok(())
}
