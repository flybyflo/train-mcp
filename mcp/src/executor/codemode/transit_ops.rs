//! Direct [`TransitProvider`] calls shared by the QuickJS `codemode` bindings and the REST API.

use std::sync::Arc;

use serde_json::Value;

use crate::oebb::types::{
    LocationResolution, PlanJourneyInput, PlanTourInput, ResolveItineraryStopsInput,
};
use crate::transit::TransitProvider;

use super::{
    normalize_plan_journey_input, normalize_plan_tour_input, normalize_resolve_itinerary_input,
    normalize_tool_result, tool_error_payload,
};

pub async fn dispatch_transit_op(
    op: &str,
    provider: Arc<dyn TransitProvider>,
    input: Value,
) -> Value {
    match op {
        "oebbPlanJourney" => oebb_plan_journey(provider, input).await,
        "oebbPlanTour" => oebb_plan_tour(provider, input).await,
        "oebbResolveItineraryStops" => oebb_resolve_itinerary_stops(provider, input).await,
        "oebbLocations" => oebb_locations(provider, input).await,
        "oebbDepartures" => oebb_departures(provider, input).await,
        "oebbJourneys" => oebb_journeys(provider, input).await,
        "oebbTrip" => oebb_trip(provider, input).await,
        _ => tool_error_payload("unknown_operation", format!("Unknown operation: {op}")),
    }
}

pub(crate) async fn oebb_plan_journey(
    provider: Arc<dyn TransitProvider>,
    input_json: Value,
) -> Value {
    let input_json = normalize_plan_journey_input(input_json);
    let input: PlanJourneyInput = match serde_json::from_value(input_json) {
        Ok(i) => i,
        Err(e) => {
            return tool_error_payload("invalid_input", format!("Failed to parse input: {e}"));
        }
    };
    match provider.plan_journey(input).await {
        Ok(result) => normalize_tool_result(result, "plan_journey_failed"),
        Err(e) => tool_error_payload("plan_journey_failed", e.to_string()),
    }
}

pub(crate) async fn oebb_plan_tour(provider: Arc<dyn TransitProvider>, input_json: Value) -> Value {
    let input_json = normalize_plan_tour_input(input_json);
    let input: PlanTourInput = match serde_json::from_value(input_json) {
        Ok(i) => i,
        Err(e) => {
            return tool_error_payload("invalid_input", format!("Failed to parse input: {e}"));
        }
    };
    match provider.plan_tour(input).await {
        Ok(result) => normalize_tool_result(result, "plan_tour_failed"),
        Err(e) => tool_error_payload("plan_tour_failed", e.to_string()),
    }
}

pub(crate) async fn oebb_resolve_itinerary_stops(
    provider: Arc<dyn TransitProvider>,
    input_json: Value,
) -> Value {
    let input_json = normalize_resolve_itinerary_input(input_json);
    let input: ResolveItineraryStopsInput = match serde_json::from_value(input_json) {
        Ok(i) => i,
        Err(e) => {
            return tool_error_payload("invalid_input", format!("Failed to parse input: {e}"));
        }
    };

    match provider.resolve_itinerary_stops(input).await {
        Ok(result) => normalize_tool_result(result, "resolve_itinerary_stops_failed"),
        Err(e) => tool_error_payload("resolve_itinerary_stops_failed", e.to_string()),
    }
}

pub(crate) async fn oebb_locations(provider: Arc<dyn TransitProvider>, input: Value) -> Value {
    let query = input
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let results = input
        .get("results")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);
    let stops = input.get("stops").and_then(|v| v.as_bool());
    let addresses = input.get("addresses").and_then(|v| v.as_bool());
    let poi = input.get("poi").and_then(|v| v.as_bool());
    match provider
        .locations(&query, results, stops, addresses, poi)
        .await
    {
        Ok(result) => normalize_tool_result(result, "oebbLocations_failed"),
        Err(e) => tool_error_payload("oebbLocations_failed", e.to_string()),
    }
}

pub(crate) async fn oebb_departures(provider: Arc<dyn TransitProvider>, input: Value) -> Value {
    let stop_id = input
        .get("stopId")
        .or_else(|| input.get("station"))
        .or_else(|| input.get("stop"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if stop_id.is_empty() {
        return tool_error_payload(
            "invalid_input",
            "Field \"stopId\" (or \"station\") is required and cannot be empty.",
        );
    }
    let should_resolve = input
        .get("resolveLocationIds")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let resolved_stop_id = match provider
        .resolve_location("oebbDepartures", "station", &stop_id, should_resolve)
        .await
    {
        LocationResolution::Ok { id, .. } => id,
        LocationResolution::Err(err) => {
            return normalize_tool_result(err, "oebbDepartures_failed");
        }
    };
    let duration = input
        .get("duration")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);
    let when = input.get("when").and_then(|v| v.as_str()).map(String::from);
    let results = input
        .get("results")
        .or_else(|| input.get("limit"))
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);
    let stopovers = input.get("stopovers").and_then(|v| v.as_bool());
    let remarks = input.get("remarks").and_then(|v| v.as_bool());
    match provider
        .departures(
            &resolved_stop_id,
            duration,
            when.as_deref(),
            results,
            stopovers,
            remarks,
        )
        .await
    {
        Ok(result) => normalize_tool_result(result, "oebbDepartures_failed"),
        Err(e) => tool_error_payload("oebbDepartures_failed", e.to_string()),
    }
}

pub(crate) async fn oebb_journeys(provider: Arc<dyn TransitProvider>, input: Value) -> Value {
    let from = input
        .get("from")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let to = input
        .get("to")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let should_resolve = input
        .get("resolveLocationIds")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let resolved_from = match provider
        .resolve_location("oebbJourneys", "from", &from, should_resolve)
        .await
    {
        LocationResolution::Ok { id, .. } => id,
        LocationResolution::Err(err) => {
            return normalize_tool_result(err, "oebbJourneys_failed");
        }
    };
    let resolved_to = match provider
        .resolve_location("oebbJourneys", "to", &to, should_resolve)
        .await
    {
        LocationResolution::Ok { id, .. } => id,
        LocationResolution::Err(err) => {
            return normalize_tool_result(err, "oebbJourneys_failed");
        }
    };
    let mut query_params: Vec<(String, String)> = vec![
        ("from".into(), resolved_from.clone()),
        ("to".into(), resolved_to.clone()),
    ];
    if let Some(r) = input.get("results").and_then(|v| v.as_u64()) {
        query_params.push(("results".into(), r.to_string()));
    }
    if let Some(d) = input.get("departure").and_then(|v| v.as_str()) {
        query_params.push(("departure".into(), d.to_string()));
    }
    if let Some(a) = input.get("arrival").and_then(|v| v.as_str()) {
        query_params.push(("arrival".into(), a.to_string()));
    }
    if let Some(s) = input.get("stopovers").and_then(|v| v.as_bool()) {
        query_params.push(("stopovers".into(), s.to_string()));
    }
    match provider
        .journeys_raw(&resolved_from, &resolved_to, &query_params)
        .await
    {
        Ok(result) => normalize_tool_result(result, "oebbJourneys_failed"),
        Err(e) => tool_error_payload("oebbJourneys_failed", e.to_string()),
    }
}

pub(crate) async fn oebb_trip(provider: Arc<dyn TransitProvider>, input: Value) -> Value {
    let trip_id = input
        .get("tripId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let stopovers = input.get("stopovers").and_then(|v| v.as_bool());
    let remarks = input.get("remarks").and_then(|v| v.as_bool());
    match provider.trip(&trip_id, stopovers, remarks).await {
        Ok(result) => normalize_tool_result(result, "oebbTrip_failed"),
        Err(e) => tool_error_payload("oebbTrip_failed", e.to_string()),
    }
}
