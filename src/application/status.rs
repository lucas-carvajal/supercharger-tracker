use crate::repository::{ScrapeRunRepository, SuperchargerRepository};

pub async fn run_status(
    supercharger_repo: &SuperchargerRepository,
    scrape_run_repo: &ScrapeRunRepository,
) -> Result<(), Box<dyn std::error::Error>> {
    let run = scrape_run_repo.get_last_run_stats().await?;
    let stats = supercharger_repo.get_db_stats().await?;

    match run {
        None => println!("No runs recorded yet."),
        Some(r) => {
            println!(
                "Last run #{} ({}) — {}",
                r.id,
                r.run_type,
                r.scraped_at.format("%Y-%m-%d %H:%M UTC")
            );
            println!(
                "  Scraped: {}  |  Detail failures: {}  |  Status changes: {}",
                r.total_count, r.details_failures, r.status_changes_count
            );
        }
    }

    println!();
    println!("Active chargers: {}", stats.active);
    println!("  In Development:     {}", stats.in_development);
    println!("  Under Construction: {}", stats.under_construction);
    if stats.unknown > 0 {
        println!("  Unknown:            {}", stats.unknown);
    }
    if stats.details_failed > 0 {
        println!(
            "  ({} with failed detail fetch — run retry-failed to resolve)",
            stats.details_failed
        );
    }
    if stats.open_status_check_failed > 0 {
        println!(
            "  ({} with failed open-status check — run retry-failed to resolve)",
            stats.open_status_check_failed
        );
    }

    Ok(())
}
