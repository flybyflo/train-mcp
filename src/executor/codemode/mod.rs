mod execute;
mod search;

use std::sync::Arc;

use rquickjs::{prelude::*, Ctx, Object, Result as JsResult, Value as JsValue};
use serde_json::Value;

use crate::transit::TransitProvider;

use super::quickjs::Mode;

pub const TOOL_ERROR_MARKER: &str = "__trainToolError";

/// Inject the `codemode` global object into a QuickJS context.
/// In Search mode: getCatalog(), listTools()
/// In Execute mode: oebbPlanJourney, oebbPlanTour, oebbResolveItineraryStops, oebbLocations, oebbDepartures, oebbJourneys, oebbTrip
pub fn inject_codemode<'js>(
    ctx: &Ctx<'js>,
    mode: Mode,
    provider: Arc<dyn TransitProvider>,
    catalog: Value,
) -> JsResult<()> {
    let globals = ctx.globals();
    let codemode = Object::new(ctx.clone())?;

    match mode {
        Mode::Search => search::install_search_tools(ctx, &codemode, catalog)?,
        Mode::Execute => execute::install_execute_tools(ctx, &codemode, provider)?,
    }

    globals.set("codemode", codemode)?;
    Ok(())
}

/// Extract the first argument as JSON, or return an empty object.
pub(super) fn args_to_json<'js>(args: &Rest<JsValue<'js>>) -> Value {
    match args.0.first() {
        Some(v) => js_value_to_serde(v),
        None => Value::Object(serde_json::Map::new()),
    }
}

/// Convert a serde_json::Value to a rquickjs JsValue.
pub(super) fn json_to_js<'js>(ctx: &Ctx<'js>, value: &Value) -> JsResult<JsValue<'js>> {
    let json_str = serde_json::to_string(value).unwrap_or_else(|_| "null".to_string());
    ctx.json_parse(json_str)
}

/// Convert a rquickjs JsValue to serde_json::Value.
fn js_value_to_serde(value: &JsValue) -> Value {
    if value.is_undefined() || value.is_null() {
        return Value::Null;
    }
    if let Some(b) = value.as_bool() {
        return Value::Bool(b);
    }
    if let Some(n) = value.as_int() {
        return Value::Number(n.into());
    }
    if let Some(n) = value.as_float() {
        if n.is_finite() {
            return serde_json::json!(n);
        }
        return Value::Null;
    }
    if let Some(s) = value.clone().into_string() {
        if let Ok(owned) = s.to_string() {
            return Value::String(owned);
        }
    }
    if let Some(obj) = value.clone().into_object() {
        let ctx = obj.ctx().clone();
        if let Ok(Some(s)) = ctx.json_stringify(obj) {
            if let Ok(owned) = s.to_string() {
                if let Ok(parsed) = serde_json::from_str::<Value>(&owned) {
                    return parsed;
                }
            }
        }
    }
    Value::Null
}

pub(super) fn tool_error_payload(code: impl Into<String>, message: impl Into<String>) -> Value {
    serde_json::json!({
        TOOL_ERROR_MARKER: true,
        "error": code.into(),
        "message": message.into(),
    })
}

pub(super) fn normalize_tool_result(value: Value, fallback_code: &str) -> Value {
    let Some(obj) = value.as_object() else {
        return value;
    };

    let Some(code) = obj.get("error").and_then(|v| v.as_str()) else {
        return value;
    };

    let message = obj
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or(fallback_code);

    let mut wrapped = obj.clone();
    wrapped.insert(TOOL_ERROR_MARKER.to_string(), Value::Bool(true));
    wrapped.insert("error".to_string(), Value::String(code.to_string()));
    wrapped.insert("message".to_string(), Value::String(message.to_string()));
    Value::Object(wrapped)
}

pub(super) fn normalize_plan_journey_input(input: Value) -> Value {
    let mut obj = match input {
        Value::Object(obj) => obj,
        other => return other,
    };

    if obj.get("departure").is_none() {
        if let Some(v) = obj.remove("departureTime") {
            obj.insert("departure".to_string(), v);
        }
    } else {
        obj.remove("departureTime");
    }
    if obj.get("excludeOperators").is_none() {
        if let Some(v) = obj.remove("excludedOperators") {
            obj.insert("excludeOperators".to_string(), v);
        }
    } else {
        obj.remove("excludedOperators");
    }

    Value::Object(obj)
}

pub(super) fn normalize_plan_tour_input(input: Value) -> Value {
    let mut obj = match input {
        Value::Object(obj) => obj,
        other => return other,
    };

    if obj.get("departure").is_none() {
        if let Some(v) = obj.remove("departureTime") {
            obj.insert("departure".to_string(), v);
        }
    } else {
        obj.remove("departureTime");
    }
    if obj.get("excludeOperators").is_none() {
        if let Some(v) = obj.remove("excludedOperators") {
            obj.insert("excludeOperators".to_string(), v);
        }
    } else {
        obj.remove("excludedOperators");
    }

    if obj.get("legs").is_none() {
        if let Some(Value::Array(tour_points)) = obj.get("tour").cloned() {
            let mut legs = Vec::new();
            if tour_points.len() >= 2 {
                for i in 0..tour_points.len() - 1 {
                    if let (Some(from), Some(to)) = (
                        tour_points[i]
                            .as_str()
                            .map(str::trim)
                            .filter(|s| !s.is_empty()),
                        tour_points[i + 1]
                            .as_str()
                            .map(str::trim)
                            .filter(|s| !s.is_empty()),
                    ) {
                        legs.push(serde_json::json!({ "from": from, "to": to }));
                    }
                }
            }
            obj.insert("legs".to_string(), Value::Array(legs));
        }
    }

    let mut waits: Vec<u64> = Vec::new();
    if let Some(Value::Array(a)) = obj.get("waits") {
        waits = a.iter().filter_map(|v| v.as_u64()).collect();
    } else if let Some(Value::Array(a)) = obj.get("stopoverMinutes") {
        waits = a.iter().filter_map(|v| v.as_u64()).collect();
    }

    let via_per_leg_alias = obj.get("via").cloned();

    if let Some(Value::Array(legs)) = obj.get_mut("legs") {
        for (idx, leg) in legs.iter_mut().enumerate() {
            if idx >= waits.len() {
                break;
            }
            if let Value::Object(map) = leg {
                if map.get("minStopMinutesAfter").is_none() {
                    map.insert(
                        "minStopMinutesAfter".to_string(),
                        serde_json::json!(waits[idx]),
                    );
                }
            }
        }

        if let Some(Value::Array(via_per_leg)) = via_per_leg_alias {
            for (idx, leg) in legs.iter_mut().enumerate() {
                if idx >= via_per_leg.len() {
                    break;
                }
                if let (Some(via_entry), Value::Object(map)) = (via_per_leg.get(idx), leg) {
                    if !via_entry.is_null() && map.get("via").is_none() {
                        map.insert("via".to_string(), via_entry.clone());
                    }
                }
            }
        }
    }

    Value::Object(obj)
}

pub(super) fn normalize_resolve_itinerary_input(input: Value) -> Value {
    let mut obj = match input {
        Value::Object(obj) => obj,
        other => return other,
    };

    if obj.get("legs").is_none() {
        if let Some(v) = obj.get("resolvedLegs").cloned() {
            obj.insert("legs".to_string(), v);
        }
    }

    Value::Object(obj)
}
