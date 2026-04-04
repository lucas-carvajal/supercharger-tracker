#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use crate::scraper::raw::Location;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChargingAccess {
    AllVehicles,
    NacsPartner,
    NacsPartnerTest,
    TeslaOnly,
    Unknown,
}

impl ChargingAccess {
    fn from_str(s: &str) -> Self {
        match s {
            "All Vehicles (Production)" => Self::AllVehicles,
            "NACS Partner Enabled (Production)" => Self::NacsPartner,
            "NACS Partner Enabled (Test)" => Self::NacsPartnerTest,
            "Tesla Only" => Self::TeslaOnly,
            _ => Self::Unknown,
        }
    }
}

impl std::fmt::Display for ChargingAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AllVehicles => write!(f, "All Vehicles"),
            Self::NacsPartner => write!(f, "NACS Partner"),
            Self::NacsPartnerTest => write!(f, "NACS Partner (Test)"),
            Self::TeslaOnly => write!(f, "Tesla Only"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Supercharger {
    pub uuid: String,
    pub title: String,
    pub latitude: f64,
    pub longitude: f64,
    pub open_to_non_tesla: Option<bool>,
    pub charging_accessibility: Option<ChargingAccess>,
}

impl Supercharger {
    /// Returns true for all location types that represent an open supercharger.
    /// This includes contest winners ("winner_supercharger", "current_winner_supercharger")
    /// and "party" entries — all of which have a supercharger_function block.
    pub fn is_open_supercharger(location: &Location) -> bool {
        location.location_type.iter().any(|t| {
            matches!(t.as_str(), "supercharger" | "party")
        })
    }
}

impl From<&Location> for Supercharger {
    fn from(l: &Location) -> Self {
        Self {
            uuid: l.uuid.clone(),
            title: l.title.clone(),
            latitude: l.latitude,
            longitude: l.longitude,
            open_to_non_tesla: l.supercharger_function.as_ref().and_then(|f| f.open_to_non_tesla),
            charging_accessibility: l
                .supercharger_function
                .as_ref()
                .and_then(|f| f.charging_accessibility.as_ref())
                .map(|s| ChargingAccess::from_str(s)),
        }
    }
}
