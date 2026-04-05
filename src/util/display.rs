#![allow(dead_code)]

use crate::domain::coming_soon::ComingSoonSupercharger;
use crate::domain::supercharger::Supercharger;

pub fn print_superchargers(title: &str, locations: &[Supercharger]) {
    println!("┌{:─<76}┐", "");
    println!("│ {:<74} │", title);
    println!("├{:─<5}┬{:─<37}┬{:─<11}┬{:─<9}┬{:─<9}┤", "", "", "", "", "");
    println!(
        "│ {:>3} │ {:<35} │ {:>9} │ {:>7} │ {:<7} │",
        "#", "Name", "Lat", "Lon", "Non-T"
    );
    println!("├{:─<5}┼{:─<37}┼{:─<11}┼{:─<9}┼{:─<9}┤", "", "", "", "", "");
    for (i, sc) in locations.iter().enumerate() {
        let non_tesla = sc.open_to_non_tesla.map_or("?", |v| if v { "yes" } else { "no" });
        println!(
            "│ {:>3} │ {:<35} │ {:>9.4} │ {:>7.4} │ {:<7} │",
            i + 1,
            truncate(&sc.title, 35),
            sc.latitude,
            sc.longitude,
            non_tesla,
        );
    }
    println!("└{:─<5}┴{:─<37}┴{:─<11}┴{:─<9}┴{:─<9}┘", "", "", "", "", "");
    println!("  {} locations", locations.len());
}

pub fn print_coming_soon(title: &str, locations: &[ComingSoonSupercharger]) {
    // Columns: # (5), Name (30), ETA/Status (27), Lat (11), Lon (9), Slug (22) — total: 110
    println!("┌{:─<108}┐", "");
    println!("│ {:<106} │", title);
    println!("├{:─<5}┬{:─<30}┬{:─<27}┬{:─<11}┬{:─<9}┬{:─<22}┤", "", "", "", "", "", "");
    println!(
        "│ {:>3} │ {:<28} │ {:<25} │ {:>9} │ {:>7} │ {:<20} │",
        "#", "Name", "ETA / Status", "Lat", "Lon", "Slug"
    );
    println!("├{:─<5}┼{:─<30}┼{:─<27}┼{:─<11}┼{:─<9}┼{:─<22}┤", "", "", "", "", "", "");
    for (i, sc) in locations.iter().enumerate() {
        let status = sc.status.to_string();
        println!(
            "│ {:>3} │ {:<28} │ {:<25} │ {:>9.4} │ {:>7.4} │ {:<20} │",
            i + 1,
            truncate(&sc.title, 28),
            truncate(&status, 25),
            sc.latitude,
            sc.longitude,
            truncate(&sc.id, 20),
        );
        println!("  ↳ {}", sc.url());
    }
    println!("└{:─<5}┴{:─<30}┴{:─<27}┴{:─<11}┴{:─<9}┴{:─<22}┘", "", "", "", "", "", "");
    println!("  {} locations", locations.len());
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}
