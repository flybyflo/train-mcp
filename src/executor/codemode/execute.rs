use std::sync::Arc;

use rquickjs::{prelude::*, Ctx, Function, Object, Result as JsResult, Value as JsValue};

use crate::oebb::types::{
    LocationResolution, PlanJourneyInput, PlanTourInput, ResolveItineraryStopsInput,
};
use crate::transit::TransitProvider;

use super::{
    args_to_json, json_to_js, normalize_plan_journey_input, normalize_plan_tour_input,
    normalize_resolve_itinerary_input, normalize_tool_result, tool_error_payload,
};

pub(super) fn install_execute_tools<'js>(
    ctx: &Ctx<'js>,
    codemode: &Object<'js>,
    provider: Arc<dyn TransitProvider>,
) -> JsResult<()> {
    // oebbPlanJourney(input) — async, orchestrated plan_journey.
    {
        let provider_clone = provider.clone();
        let plan_fn = Function::new(
            ctx.clone(),
            Async(MutFn::new(
                move |ctx: Ctx<'js>, args: Rest<JsValue<'js>>| {
                    let provider = provider_clone.clone();
                    async move {
                        let input_json = args_to_json(&args);
                        let input_json = normalize_plan_journey_input(input_json);
                        let input: PlanJourneyInput = match serde_json::from_value(input_json) {
                            Ok(i) => i,
                            Err(e) => {
                                return json_to_js(
                                    &ctx,
                                    &tool_error_payload(
                                        "invalid_input",
                                        format!("Failed to parse input: {}", e),
                                    ),
                                )
                            }
                        };
                        match provider.plan_journey(input).await {
                            Ok(result) => json_to_js(
                                &ctx,
                                &normalize_tool_result(result, "plan_journey_failed"),
                            ),
                            Err(e) => json_to_js(
                                &ctx,
                                &tool_error_payload("plan_journey_failed", e.to_string()),
                            ),
                        }
                    }
                },
            )),
        )?;
        codemode.set("oebbPlanJourney", plan_fn)?;
    }

    // oebbPlanTour(input) — async, deterministic multi-leg planning.
    {
        let provider_clone = provider.clone();
        let tour_fn = Function::new(
            ctx.clone(),
            Async(MutFn::new(
                move |ctx: Ctx<'js>, args: Rest<JsValue<'js>>| {
                    let provider = provider_clone.clone();
                    async move {
                        let input_json = args_to_json(&args);
                        let input_json = normalize_plan_tour_input(input_json);
                        let input: PlanTourInput = match serde_json::from_value(input_json) {
                            Ok(i) => i,
                            Err(e) => {
                                return json_to_js(
                                    &ctx,
                                    &tool_error_payload(
                                        "invalid_input",
                                        format!("Failed to parse input: {}", e),
                                    ),
                                )
                            }
                        };
                        match provider.plan_tour(input).await {
                            Ok(result) => {
                                json_to_js(&ctx, &normalize_tool_result(result, "plan_tour_failed"))
                            }
                            Err(e) => json_to_js(
                                &ctx,
                                &tool_error_payload("plan_tour_failed", e.to_string()),
                            ),
                        }
                    }
                },
            )),
        )?;
        codemode.set("oebbPlanTour", tour_fn)?;
    }

    // oebbResolveItineraryStops(input) — async, pre-resolve all stop IDs for deterministic planning.
    {
        let provider_clone = provider.clone();
        let resolve_fn = Function::new(
            ctx.clone(),
            Async(MutFn::new(
                move |ctx: Ctx<'js>, args: Rest<JsValue<'js>>| {
                    let provider = provider_clone.clone();
                    async move {
                        let input_json = args_to_json(&args);
                        let input_json = normalize_resolve_itinerary_input(input_json);
                        let input: ResolveItineraryStopsInput =
                            match serde_json::from_value(input_json) {
                                Ok(i) => i,
                                Err(e) => {
                                    return json_to_js(
                                        &ctx,
                                        &tool_error_payload(
                                            "invalid_input",
                                            format!("Failed to parse input: {}", e),
                                        ),
                                    )
                                }
                            };

                        match provider.resolve_itinerary_stops(input).await {
                            Ok(result) => json_to_js(
                                &ctx,
                                &normalize_tool_result(result, "resolve_itinerary_stops_failed"),
                            ),
                            Err(e) => json_to_js(
                                &ctx,
                                &tool_error_payload(
                                    "resolve_itinerary_stops_failed",
                                    e.to_string(),
                                ),
                            ),
                        }
                    }
                },
            )),
        )?;
        codemode.set("oebbResolveItineraryStops", resolve_fn)?;
    }

    // oebbLocations(input) — async, search locations.
    {
        let provider_clone = provider.clone();
        let fn_ = Function::new(
            ctx.clone(),
            Async(MutFn::new(
                move |ctx: Ctx<'js>, args: Rest<JsValue<'js>>| {
                    let provider = provider_clone.clone();
                    async move {
                        let input = args_to_json(&args);
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
                            Ok(result) => json_to_js(
                                &ctx,
                                &normalize_tool_result(result, "oebbLocations_failed"),
                            ),
                            Err(e) => json_to_js(
                                &ctx,
                                &tool_error_payload("oebbLocations_failed", e.to_string()),
                            ),
                        }
                    }
                },
            )),
        )?;
        codemode.set("oebbLocations", fn_)?;
    }

    // oebbDepartures(input) — async, get departures.
    {
        let provider_clone = provider.clone();
        let fn_ = Function::new(
            ctx.clone(),
            Async(MutFn::new(
                move |ctx: Ctx<'js>, args: Rest<JsValue<'js>>| {
                    let provider = provider_clone.clone();
                    async move {
                        let input = args_to_json(&args);
                        let stop_id = input
                            .get("stopId")
                            .or_else(|| input.get("station"))
                            .or_else(|| input.get("stop"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if stop_id.is_empty() {
                            return json_to_js(
                            &ctx,
                            &tool_error_payload(
                                "invalid_input",
                                "Field \"stopId\" (or \"station\") is required and cannot be empty.",
                            ),
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
                                return json_to_js(
                                    &ctx,
                                    &normalize_tool_result(err, "oebbDepartures_failed"),
                                )
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
                            Ok(result) => json_to_js(
                                &ctx,
                                &normalize_tool_result(result, "oebbDepartures_failed"),
                            ),
                            Err(e) => json_to_js(
                                &ctx,
                                &tool_error_payload("oebbDepartures_failed", e.to_string()),
                            ),
                        }
                    }
                },
            )),
        )?;
        codemode.set("oebbDepartures", fn_)?;
    }

    // oebbJourneys(input) — async, raw journeys.
    {
        let provider_clone = provider.clone();
        let fn_ = Function::new(
            ctx.clone(),
            Async(MutFn::new(
                move |ctx: Ctx<'js>, args: Rest<JsValue<'js>>| {
                    let provider = provider_clone.clone();
                    async move {
                        let input = args_to_json(&args);
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
                        let mut query_params: Vec<(String, String)> =
                            vec![("from".into(), from.clone()), ("to".into(), to.clone())];
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
                        match provider.journeys_raw(&from, &to, &query_params).await {
                            Ok(result) => json_to_js(
                                &ctx,
                                &normalize_tool_result(result, "oebbJourneys_failed"),
                            ),
                            Err(e) => json_to_js(
                                &ctx,
                                &tool_error_payload("oebbJourneys_failed", e.to_string()),
                            ),
                        }
                    }
                },
            )),
        )?;
        codemode.set("oebbJourneys", fn_)?;
    }

    // oebbTrip(input) — async, get trip details.
    {
        let provider_clone = provider;
        let fn_ = Function::new(
            ctx.clone(),
            Async(MutFn::new(
                move |ctx: Ctx<'js>, args: Rest<JsValue<'js>>| {
                    let provider = provider_clone.clone();
                    async move {
                        let input = args_to_json(&args);
                        let trip_id = input
                            .get("tripId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let stopovers = input.get("stopovers").and_then(|v| v.as_bool());
                        let remarks = input.get("remarks").and_then(|v| v.as_bool());
                        match provider.trip(&trip_id, stopovers, remarks).await {
                            Ok(result) => {
                                json_to_js(&ctx, &normalize_tool_result(result, "oebbTrip_failed"))
                            }
                            Err(e) => json_to_js(
                                &ctx,
                                &tool_error_payload("oebbTrip_failed", e.to_string()),
                            ),
                        }
                    }
                },
            )),
        )?;
        codemode.set("oebbTrip", fn_)?;
    }

    Ok(())
}
