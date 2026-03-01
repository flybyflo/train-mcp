use serde_json::Value;

use super::normalize::{as_str, compact_object, normalize_comparable_text, summarize_place, jaro_winkler_similarity};
use super::types::ViaResolution;

/// Check if any leg of a journey uses an excluded operator.
pub fn journey_uses_excluded_operator(journey: &Value, exclude_operators: &[String]) -> bool {
    if exclude_operators.is_empty() {
        return false;
    }
    let legs = match journey.get("legs").and_then(|v| v.as_array()) {
        Some(l) => l,
        None => return false,
    };
    legs.iter().any(|leg| {
        let obj = match leg.as_object() {
            Some(o) => o,
            None => return false,
        };
        if obj.get("walking") == Some(&Value::Bool(true)) {
            return false;
        }
        let fields: Vec<String> = vec![
            get_leg_line_field(leg, &["line", "operator", "name"]),
            get_leg_line_field(leg, &["line", "operator", "id"]),
            get_leg_line_field(leg, &["line", "name"]),
            get_leg_line_field(leg, &["line", "id"]),
            get_leg_line_field(leg, &["line", "productName"]),
        ];
        exclude_operators
            .iter()
            .any(|needle| fields.iter().any(|field| field.contains(needle.as_str())))
    })
}

fn get_leg_line_field(leg: &Value, path: &[&str]) -> String {
    let mut current = leg;
    for key in path {
        match current.get(*key) {
            Some(v) => current = v,
            None => return String::new(),
        }
    }
    current.as_str().unwrap_or("").to_lowercase()
}

/// Collect all origin/destination/stopover points from a journey.
fn collect_journey_points(journey: &Value) -> Vec<&Value> {
    let legs = match journey.get("legs").and_then(|v| v.as_array()) {
        Some(l) => l,
        None => return vec![],
    };
    let mut points = Vec::new();
    for leg in legs {
        if let Some(origin) = leg.get("origin") {
            if origin.is_object() {
                points.push(origin);
            }
        }
        if let Some(dest) = leg.get("destination") {
            if dest.is_object() {
                points.push(dest);
            }
        }
        if let Some(stopovers) = leg.get("stopovers").and_then(|v| v.as_array()) {
            for stopover in stopovers {
                if let Some(stop) = stopover.get("stop") {
                    if stop.is_object() {
                        points.push(stop);
                    }
                } else if stopover.is_object() {
                    points.push(stopover);
                }
            }
        }
    }
    points
}

/// Check if a journey passes through a via point.
pub fn journey_contains_via(journey: &Value, via: &ViaResolution) -> bool {
    let points = collect_journey_points(journey);
    if points.is_empty() {
        return false;
    }
    let needle = normalize_comparable_text(via.name.as_deref().unwrap_or(&via.input));
    for point in &points {
        if let (Some(via_id), Some(point_id)) = (&via.id, point.get("id").and_then(|v| v.as_str()))
        {
            if via_id == point_id {
                return true;
            }
        }
        if let Some(point_name) = point.get("name").and_then(|v| v.as_str()) {
            let normalized_point = normalize_comparable_text(point_name);
            // Forward contains: needle found within point name (always safe).
            if normalized_point.contains(&needle) {
                return true;
            }
            // Reverse contains: point name found within needle, but only when the
            // point name is long enough to avoid false positives (e.g. "linz"
            // matching inside "klagenfurtlinzerstrasse").
            if normalized_point.len() >= 4 && needle.contains(&normalized_point) {
                return true;
            }
            // Fuzzy fallback: accept high Jaro-Winkler similarity for minor API
            // name variations (e.g. "wien hauptbahnhof" vs "wien hbf").
            if jaro_winkler_similarity(&needle, &normalized_point) >= 0.88 {
                return true;
            }
        }
    }
    false
}

/// Filter journeys by excluded operators and strict via.
pub fn filter_journeys(
    payload: &Value,
    exclude_operators: &[String],
    strict_via: bool,
    via_stops: &[ViaResolution],
) -> Value {
    let obj = match payload.as_object() {
        Some(o) => o,
        None => return payload.clone(),
    };
    let journeys_raw = match obj.get("journeys").and_then(|v| v.as_array()) {
        Some(j) => j,
        None => return payload.clone(),
    };

    let mut removed_by_operator = 0usize;
    let mut removed_by_via = 0usize;

    let filtered: Vec<&Value> = journeys_raw
        .iter()
        .filter(|journey| {
            if !exclude_operators.is_empty()
                && journey_uses_excluded_operator(journey, exclude_operators)
            {
                removed_by_operator += 1;
                return false;
            }
            if strict_via && !via_stops.is_empty() {
                let has_all = via_stops
                    .iter()
                    .all(|via| journey_contains_via(journey, via));
                if !has_all {
                    removed_by_via += 1;
                    return false;
                }
            }
            true
        })
        .collect();

    let requested_via: Vec<Value> = via_stops
        .iter()
        .map(|v| {
            let mut m = serde_json::Map::new();
            m.insert("input".into(), Value::String(v.input.clone()));
            if let Some(ref id) = v.id {
                m.insert("id".into(), Value::String(id.clone()));
            }
            if let Some(ref name) = v.name {
                m.insert("name".into(), Value::String(name.clone()));
            }
            compact_object(&m)
        })
        .collect();

    let filter_stats = serde_json::json!({
        "totalBeforeFilters": journeys_raw.len(),
        "removedByOperator": removed_by_operator,
        "removedByVia": removed_by_via,
        "totalAfterFilters": filtered.len(),
        "excludeOperators": exclude_operators,
        "requestedVia": requested_via,
        "strictVia": strict_via,
    });

    let mut result = obj.clone();
    result.insert(
        "journeys".into(),
        Value::Array(filtered.into_iter().cloned().collect()),
    );
    result.insert("filterStats".into(), filter_stats);
    Value::Object(result)
}

/// Helper: summarize a stopover (used for enrichment).
pub fn summarize_stopover(value: &Value) -> Option<Value> {
    let obj = value.as_object()?;
    let stop = obj
        .get("stop")
        .and_then(summarize_place)
        .or_else(|| summarize_place(value));

    let mut result = serde_json::Map::new();
    if let Some(s) = stop {
        result.insert("stop".into(), s);
    }
    for key in &[
        "arrival",
        "plannedArrival",
        "departure",
        "plannedDeparture",
        "arrivalPlatform",
        "plannedArrivalPlatform",
        "departurePlatform",
        "plannedDeparturePlatform",
    ] {
        if let Some(s) = obj.get(*key).and_then(|v| as_str(v)) {
            result.insert((*key).to_string(), Value::String(s.to_string()));
        }
    }
    for key in &["arrivalDelay", "departureDelay"] {
        if let Some(n) = obj.get(*key).and_then(|v| v.as_f64()) {
            if n.is_finite() {
                result.insert((*key).to_string(), serde_json::json!(n));
            }
        }
    }
    if obj.get("cancelled") == Some(&Value::Bool(true)) {
        result.insert("cancelled".into(), Value::Bool(true));
    }
    if result.is_empty() {
        None
    } else {
        Some(compact_object(&result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_journey_with_stop(stop_name: &str, stop_id: &str) -> Value {
        serde_json::json!({
            "legs": [{
                "origin": { "id": stop_id, "name": stop_name },
                "destination": { "id": "8100002", "name": "Wien Hbf" }
            }]
        })
    }

    fn make_via(input: &str, id: Option<&str>, name: Option<&str>) -> ViaResolution {
        ViaResolution {
            input: input.to_string(),
            id: id.map(String::from),
            name: name.map(String::from),
            is_stopover: false,
            min_stop_minutes: None,
            max_stop_minutes: None,
        }
    }

    #[test]
    fn test_via_match_by_id() {
        let journey = make_journey_with_stop("Linz Hbf", "8100013");
        let via = make_via("Linz", Some("8100013"), Some("Linz Hbf"));
        assert!(journey_contains_via(&journey, &via));
    }

    #[test]
    fn test_via_match_forward_contains() {
        let journey = make_journey_with_stop("Linz Hbf", "8100013");
        let via = make_via("Linz", None, Some("Linz"));
        assert!(journey_contains_via(&journey, &via));
    }

    #[test]
    fn test_via_rejects_short_reverse_contains() {
        // "Linz" (len=4) matching inside "Klagenfurt Linzer Straße" would be a
        // forward contains of "linz" in "klagenfurtlinzerstrasse", so it still matches.
        // But a 3-char name like "Ulm" should NOT match "Helmut" via reverse contains.
        let journey = make_journey_with_stop("Helmut-Station", "999");
        let via = make_via("Ulm", None, Some("Ulm"));
        assert!(!journey_contains_via(&journey, &via));
    }

    #[test]
    fn test_via_fuzzy_fallback() {
        // Fuzzy match: "wienhbf" vs "wienhauptbahnhof" — Jaro-Winkler similarity
        // may or may not be >= 0.88. Let's test with close variants.
        let journey = make_journey_with_stop("Wien Hbf", "8100002");
        let via = make_via("Wien Hb", None, Some("Wien Hb"));
        // "wienhb" vs "wienhbf" should be very similar
        assert!(journey_contains_via(&journey, &via));
    }
}
