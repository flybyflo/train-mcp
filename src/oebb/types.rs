use serde::{Deserialize, Serialize};

/// Input for the oebbPlanJourney tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanJourneyInput {
    pub from: String,
    pub to: String,
    #[serde(alias = "departureTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub departure: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arrival: Option<String>,
    /// Alias: callers can pass `datetime` + `type` instead of `departure`/`arrival`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub datetime: Option<String>,
    /// "departure" or "arrival" — used with `datetime` alias.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub datetime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<ViaInputs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_transfer_minutes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict_via: Option<bool>,
    #[serde(alias = "excludedOperators")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_operators: Option<StringOrVec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolve_location_ids: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<JourneySelection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_stopovers: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_alternatives: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_mode: Option<ResponseMode>,
}

/// One leg in an end-to-end tour plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TourLegInput {
    pub from: String,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<ViaInputs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_stop_minutes_after: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_stop_minutes_after: Option<u32>,
}

/// Input for the oebbPlanTour tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanTourInput {
    pub legs: Vec<TourLegInput>,
    #[serde(alias = "departureTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub departure: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub datetime: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub datetime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_transfer_minutes: Option<u32>,
    #[serde(alias = "excludedOperators")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_operators: Option<StringOrVec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolve_location_ids: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<JourneySelection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_stopovers: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_mode: Option<ResponseMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StringOrVec {
    Single(String),
    Multiple(Vec<String>),
}

impl StringOrVec {
    pub fn to_vec(&self) -> Vec<String> {
        match self {
            StringOrVec::Single(s) => vec![s.clone()],
            StringOrVec::Multiple(v) => v.clone(),
        }
    }
}

/// The type of via stop behavior.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ViaStopType {
    /// Train passes through (default) — no alighting.
    Passthrough,
    /// Alight, spend time, then board a later train.
    Stopover,
}

/// A structured via stop with optional stopover configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViaStopSpec {
    pub station: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_type: Option<ViaStopType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_stop_minutes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_stop_minutes: Option<u32>,
}

/// A single via specification — either a station name or a structured stop.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ViaSpec {
    Simple(String),
    Detailed(ViaStopSpec),
}

impl ViaSpec {
    pub fn station(&self) -> &str {
        match self {
            ViaSpec::Simple(s) => s.trim(),
            ViaSpec::Detailed(d) => d.station.trim(),
        }
    }

    pub fn is_stopover(&self) -> bool {
        match self {
            ViaSpec::Simple(_) => false,
            ViaSpec::Detailed(d) => {
                d.stop_type == Some(ViaStopType::Stopover)
                    || d.min_stop_minutes.is_some()
                    || d.max_stop_minutes.is_some()
            }
        }
    }

    pub fn min_stop_minutes(&self) -> Option<u32> {
        match self {
            ViaSpec::Simple(_) => None,
            ViaSpec::Detailed(d) => d.min_stop_minutes,
        }
    }

    pub fn max_stop_minutes(&self) -> Option<u32> {
        match self {
            ViaSpec::Simple(_) => None,
            ViaSpec::Detailed(d) => d.max_stop_minutes,
        }
    }
}

/// One or more via specifications.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ViaInputs {
    Single(ViaSpec),
    Multiple(Vec<ViaSpec>),
}

impl ViaInputs {
    pub fn to_vec(self) -> Vec<ViaSpec> {
        match self {
            ViaInputs::Single(s) => vec![s],
            ViaInputs::Multiple(v) => v,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JourneySelection {
    Fastest,
    EarliestArrival,
    FewestTransfers,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseMode {
    Summary,
    Raw,
}

/// Result of resolving a location name to a stop ID.
#[derive(Debug, Clone)]
pub enum LocationResolution {
    Ok {
        id: String,
        name: Option<String>,
        resolved: bool,
    },
    Err(serde_json::Value),
}

#[derive(Debug, Clone)]
pub struct ViaResolution {
    pub input: String,
    pub id: Option<String>,
    pub name: Option<String>,
    pub is_stopover: bool,
    pub min_stop_minutes: Option<u32>,
    pub max_stop_minutes: Option<u32>,
}

/// A scored location candidate from the ÖBB locations API.
#[derive(Debug, Clone)]
pub struct LocationCandidate {
    pub id: String,
    pub name: String,
    pub loc_type: Option<String>,
    pub score: i32,
}

/// Runtime configuration for the ÖBB client.
#[derive(Debug, Clone)]
pub struct OebbConfig {
    pub base_url: String,
    pub timeout_ms: u64,
    pub max_attempts: u32,
    pub retry_base_delay_ms: u64,
    pub retry_max_delay_ms: u64,
    pub retry_budget_ms: u64,
}

/// One leg for stop resolution prior to planning.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveStopsLegInput {
    pub from: String,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<ViaInputs>,
}

/// Input for the oebbResolveItineraryStops tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveItineraryStopsInput {
    pub legs: Vec<ResolveStopsLegInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolve_location_ids: Option<bool>,
}

impl OebbConfig {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            timeout_ms: u64_from_env("OEBB_TIMEOUT_MS", 12_000),
            max_attempts: u32_from_env("OEBB_MAX_ATTEMPTS", 3).max(1),
            retry_base_delay_ms: u64_from_env("OEBB_RETRY_BASE_DELAY_MS", 350),
            retry_max_delay_ms: u64_from_env("OEBB_RETRY_MAX_DELAY_MS", 4_000),
            retry_budget_ms: u64_from_env("OEBB_RETRY_BUDGET_MS", 20_000),
        }
    }
}

fn u64_from_env(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}

fn u32_from_env(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(default)
}
