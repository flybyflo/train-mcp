use serde_json::Value;

use super::normalize::{as_str, compact_object, summarize_line, summarize_place};
use super::types::JourneySelection;

/// A ranked journey with computed metrics.
#[derive(Debug, Clone)]
pub struct RankedJourney {
    pub journey: Value,
    pub legs: Vec<Value>,
    pub departure: Option<String>,
    pub arrival: Option<String>,
    pub departure_ts: i64,
    pub arrival_ts: i64,
    pub duration_minutes: i64,
    pub transfers: usize,
    pub enriched_intermediate_stops: Vec<Vec<Value>>,
}

fn to_timestamp(value: Option<&str>) -> Option<i64> {
    value.and_then(|s| {
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.timestamp_millis())
    })
}

fn to_sortable(value: Option<i64>) -> i64 {
    value.unwrap_or(i64::MAX)
}

fn extract_journey_legs(journey: &Value) -> Vec<Value> {
    journey
        .get("legs")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|l| l.is_object())
        .collect()
}

fn is_transit_leg(leg: &Value) -> bool {
    leg.get("walking") != Some(&Value::Bool(true))
}

fn count_transfers(legs: &[Value]) -> usize {
    let transit = legs.iter().filter(|l| is_transit_leg(l)).count();
    if transit > 0 {
        transit.saturating_sub(1)
    } else {
        0
    }
}

fn pick_departure(journey: &Value, legs: &[Value]) -> Option<String> {
    if let Some(d) = journey.get("departure").and_then(|v| as_str(v)) {
        return Some(d.to_string());
    }
    for leg in legs {
        if let Some(d) = leg
            .get("departure")
            .or_else(|| leg.get("plannedDeparture"))
            .and_then(|v| as_str(v))
        {
            return Some(d.to_string());
        }
    }
    None
}

fn pick_arrival(journey: &Value, legs: &[Value]) -> Option<String> {
    if let Some(a) = journey.get("arrival").and_then(|v| as_str(v)) {
        return Some(a.to_string());
    }
    for leg in legs.iter().rev() {
        if let Some(a) = leg
            .get("arrival")
            .or_else(|| leg.get("plannedArrival"))
            .and_then(|v| as_str(v))
        {
            return Some(a.to_string());
        }
    }
    None
}

/// Rank journeys by the selected strategy.
pub fn rank_journeys(journeys: &[Value], selection: JourneySelection) -> Vec<RankedJourney> {
    let mut ranked: Vec<RankedJourney> = journeys
        .iter()
        .map(|journey| {
            let legs = extract_journey_legs(journey);
            let departure = pick_departure(journey, &legs);
            let arrival = pick_arrival(journey, &legs);
            let dep_ts = to_sortable(to_timestamp(departure.as_deref()));
            let arr_ts = to_sortable(to_timestamp(arrival.as_deref()));
            let duration = if dep_ts != i64::MAX && arr_ts != i64::MAX && arr_ts >= dep_ts {
                (arr_ts - dep_ts) / 60_000
            } else {
                i64::MAX
            };
            let transfers = count_transfers(&legs);
            let enriched = legs.iter().map(|_| Vec::new()).collect();
            RankedJourney {
                journey: journey.clone(),
                legs,
                departure,
                arrival,
                departure_ts: dep_ts,
                arrival_ts: arr_ts,
                duration_minutes: duration,
                transfers,
                enriched_intermediate_stops: enriched,
            }
        })
        .collect();

    ranked.sort_by(|a, b| match selection {
        JourneySelection::EarliestArrival => a
            .arrival_ts
            .cmp(&b.arrival_ts)
            .then(a.duration_minutes.cmp(&b.duration_minutes))
            .then(a.transfers.cmp(&b.transfers))
            .then(a.departure_ts.cmp(&b.departure_ts)),
        JourneySelection::FewestTransfers => a
            .transfers
            .cmp(&b.transfers)
            .then(a.duration_minutes.cmp(&b.duration_minutes))
            .then(a.arrival_ts.cmp(&b.arrival_ts))
            .then(a.departure_ts.cmp(&b.departure_ts)),
        JourneySelection::Fastest => a
            .duration_minutes
            .cmp(&b.duration_minutes)
            .then(a.departure_ts.cmp(&b.departure_ts))
            .then(a.transfers.cmp(&b.transfers))
            .then(a.arrival_ts.cmp(&b.arrival_ts)),
    });

    ranked
}

/// Summarize a ranked journey for output.
pub fn summarize_ranked_journey(journey: &RankedJourney, include_stopovers: bool) -> Value {
    let legs: Vec<Value> = journey
        .legs
        .iter()
        .enumerate()
        .map(|(i, leg)| {
            let mut m = serde_json::Map::new();
            if let Some(from) = leg.get("origin").and_then(summarize_place) {
                m.insert("from".into(), from);
            }
            if let Some(to) = leg.get("destination").and_then(summarize_place) {
                m.insert("to".into(), to);
            }
            let dep = leg
                .get("departure")
                .and_then(|v| as_str(v))
                .or_else(|| leg.get("plannedDeparture").and_then(|v| as_str(v)));
            if let Some(d) = dep {
                m.insert("departure".into(), Value::String(d.to_string()));
            }
            let arr = leg
                .get("arrival")
                .and_then(|v| as_str(v))
                .or_else(|| leg.get("plannedArrival").and_then(|v| as_str(v)));
            if let Some(a) = arr {
                m.insert("arrival".into(), Value::String(a.to_string()));
            }
            if let Some(line) = leg.get("line").and_then(summarize_line) {
                m.insert("line".into(), line);
            }
            if include_stopovers {
                let stops = journey
                    .enriched_intermediate_stops
                    .get(i)
                    .cloned()
                    .unwrap_or_default();
                m.insert("intermediateStops".into(), Value::Array(stops));
            }
            compact_object(&m)
        })
        .collect();

    let mut result = serde_json::Map::new();
    if let Some(ref d) = journey.departure {
        result.insert("departure".into(), Value::String(d.clone()));
    }
    if let Some(ref a) = journey.arrival {
        result.insert("arrival".into(), Value::String(a.clone()));
    }
    if journey.duration_minutes != i64::MAX {
        result.insert(
            "durationMinutes".into(),
            serde_json::json!(journey.duration_minutes),
        );
    }
    result.insert("transfers".into(), serde_json::json!(journey.transfers));
    result.insert("legs".into(), Value::Array(legs));
    compact_object(&result)
}

/// Extract intermediate stops from a leg's stopovers, excluding origin/destination.
pub fn extract_intermediate_stops_from_leg(leg: &Value) -> Vec<Value> {
    let stopovers = match leg.get("stopovers").and_then(|v| v.as_array()) {
        Some(s) => s,
        None => return vec![],
    };
    let origin = leg.get("origin").and_then(summarize_place);
    let destination = leg.get("destination").and_then(summarize_place);
    let origin_key = origin.as_ref().and_then(place_key);
    let dest_key = destination.as_ref().and_then(place_key);

    stopovers
        .iter()
        .filter_map(|stopover| {
            let stop_val = stopover
                .get("stop")
                .filter(|v| v.is_object())
                .unwrap_or(stopover);
            let summarized = summarize_place(stop_val)?;
            let key = place_key(&summarized);
            if key.is_some() && key == origin_key {
                return None;
            }
            if key.is_some() && key == dest_key {
                return None;
            }
            Some(summarized)
        })
        .collect()
}

fn place_key(value: &Value) -> Option<String> {
    if let Some(id) = value
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return Some(format!("id:{}", id));
    }
    if let Some(name) = value
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return Some(format!(
            "name:{}",
            super::normalize::normalize_comparable_text(name)
        ));
    }
    None
}
