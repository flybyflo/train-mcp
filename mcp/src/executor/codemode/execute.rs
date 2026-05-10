use std::sync::Arc;

use rquickjs::{prelude::*, Ctx, Function, Object, Result as JsResult, Value as JsValue};

use crate::transit::TransitProvider;

use super::{args_to_json, json_to_js, transit_ops};

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
                        let out = transit_ops::oebb_plan_journey(provider, input_json).await;
                        json_to_js(&ctx, &out)
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
                        let out = transit_ops::oebb_plan_tour(provider, input_json).await;
                        json_to_js(&ctx, &out)
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
                        let out =
                            transit_ops::oebb_resolve_itinerary_stops(provider, input_json).await;
                        json_to_js(&ctx, &out)
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
                        let out = transit_ops::oebb_locations(provider, input).await;
                        json_to_js(&ctx, &out)
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
                        let out = transit_ops::oebb_departures(provider, input).await;
                        json_to_js(&ctx, &out)
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
                        let out = transit_ops::oebb_journeys(provider, input).await;
                        json_to_js(&ctx, &out)
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
                        let out = transit_ops::oebb_trip(provider, input).await;
                        json_to_js(&ctx, &out)
                    }
                },
            )),
        )?;
        codemode.set("oebbTrip", fn_)?;
    }

    Ok(())
}
