use serde_json::Value;

const MAX_TOOL_JSON_CHARS: usize = 45_000;
const MAX_ARRAY_ITEMS: usize = 24;
const MAX_OBJECT_KEYS: usize = 40;
const MAX_STRING_CHARS: usize = 700;
const MAX_OBJECT_DEPTH: usize = 5;

/// Recursively compact a JSON value to fit within size limits.
/// Mirrors the TypeScript `compactJson` function.
pub fn compact_json(value: &Value, depth: usize) -> Value {
    if depth >= MAX_OBJECT_DEPTH {
        return Value::String("[truncated-depth]".to_string());
    }
    match value {
        Value::String(s) => {
            if s.len() <= MAX_STRING_CHARS {
                value.clone()
            } else {
                let truncated = &s[..MAX_STRING_CHARS];
                Value::String(format!(
                    "{}...[truncated {} chars]",
                    truncated,
                    s.len() - MAX_STRING_CHARS
                ))
            }
        }
        Value::Array(arr) => {
            let mut items: Vec<Value> = arr
                .iter()
                .take(MAX_ARRAY_ITEMS)
                .map(|item| compact_json(item, depth + 1))
                .collect();
            if arr.len() > MAX_ARRAY_ITEMS {
                items.push(serde_json::json!({ "_truncatedItems": arr.len() - MAX_ARRAY_ITEMS }));
            }
            Value::Array(items)
        }
        Value::Object(obj) => {
            let mut next = serde_json::Map::new();
            for (key, val) in obj.iter().take(MAX_OBJECT_KEYS) {
                next.insert(key.clone(), compact_json(val, depth + 1));
            }
            if obj.len() > MAX_OBJECT_KEYS {
                next.insert(
                    "_truncatedKeys".to_string(),
                    Value::Number((obj.len() - MAX_OBJECT_KEYS).into()),
                );
            }
            Value::Object(next)
        }
        _ => value.clone(),
    }
}

/// Assert that a serialized payload doesn't exceed the character limit.
pub fn assert_payload_size(payload: &Value, mode: &str) -> Result<(), PayloadTooLargeError> {
    let serialized = sorted_stringify(payload);
    if serialized.len() > MAX_TOOL_JSON_CHARS {
        return Err(PayloadTooLargeError {
            size_chars: serialized.len(),
            mode: mode.to_string(),
        });
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
#[error("payload_too_large: response is {size_chars} characters, limit is {MAX_TOOL_JSON_CHARS}")]
pub struct PayloadTooLargeError {
    pub size_chars: usize,
    pub mode: String,
}

impl PayloadTooLargeError {
    /// Convert to a structured JSON error matching Cloudflare's `toPayloadTooLargeToolError`.
    pub fn to_tool_error(&self) -> serde_json::Value {
        let hints = if self.mode == "raw" {
            vec![
                "Lower results",
                "Disable stopovers/remarks/tickets",
                "Use responseMode=\"summary\"",
            ]
        } else {
            vec![
                "Lower results",
                "Disable detail flags like stopovers/remarks/tickets",
            ]
        };
        serde_json::json!({
            "error": "payload_too_large",
            "message": self.to_string(),
            "sizeChars": self.size_chars,
            "mode": self.mode,
            "hints": hints,
        })
    }
}

/// Produce a deterministic JSON string with sorted keys.
fn sorted_stringify(value: &Value) -> String {
    serde_json::to_string(&sorted_json_value(value)).unwrap_or_default()
}

fn sorted_json_value(value: &Value) -> Value {
    match value {
        Value::Array(arr) => Value::Array(arr.iter().map(sorted_json_value).collect()),
        Value::Object(obj) => {
            let mut keys: Vec<&String> = obj.keys().collect();
            keys.sort();
            let mut next = serde_json::Map::new();
            for key in keys {
                if let Some(val) = obj.get(key) {
                    next.insert(key.clone(), sorted_json_value(val));
                }
            }
            Value::Object(next)
        }
        _ => value.clone(),
    }
}

/// Remove null/empty entries from a JSON object.
pub fn compact_object(obj: &serde_json::Map<String, Value>) -> Value {
    let mut next = serde_json::Map::new();
    for (key, value) in obj {
        if value.is_null() {
            continue;
        }
        if let Value::Array(arr) = value {
            if arr.is_empty() {
                continue;
            }
        }
        next.insert(key.clone(), value.clone());
    }
    Value::Object(next)
}

/// Extract a string field from a JSON object if it's a non-empty string.
pub fn as_str(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| !s.is_empty())
}

/// Extract a numeric field from a JSON object if it's a finite number.
pub fn as_f64(value: &Value) -> Option<f64> {
    value.as_f64().filter(|n| n.is_finite())
}

/// Summarize an ÖBB place (station/stop).
pub fn summarize_place(value: &Value) -> Option<Value> {
    let obj = value.as_object()?;
    let mut result = serde_json::Map::new();
    if let Some(id) = obj.get("id").and_then(|v| as_str(v)) {
        result.insert("id".into(), Value::String(id.to_string()));
    }
    if let Some(name) = obj.get("name").and_then(|v| as_str(v)) {
        result.insert("name".into(), Value::String(name.to_string()));
    }
    if let Some(p) = obj.get("platform").and_then(|v| as_str(v)) {
        result.insert("platform".into(), Value::String(p.to_string()));
    }
    if let Some(p) = obj.get("plannedPlatform").and_then(|v| as_str(v)) {
        result.insert("plannedPlatform".into(), Value::String(p.to_string()));
    }
    if result.is_empty() {
        None
    } else {
        Some(compact_object(&result))
    }
}

/// Summarize an ÖBB line.
pub fn summarize_line(value: &Value) -> Option<Value> {
    let obj = value.as_object()?;
    let mut result = serde_json::Map::new();

    for key in &["id", "name", "product", "productName", "mode", "fahrtNr"] {
        if let Some(s) = obj.get(*key).and_then(|v| as_str(v)) {
            result.insert((*key).to_string(), Value::String(s.to_string()));
        }
    }

    if let Some(op) = obj.get("operator").and_then(|v| v.as_object()) {
        let mut op_obj = serde_json::Map::new();
        if let Some(id) = op.get("id").and_then(|v| as_str(v)) {
            op_obj.insert("id".into(), Value::String(id.to_string()));
        }
        if let Some(name) = op.get("name").and_then(|v| as_str(v)) {
            op_obj.insert("name".into(), Value::String(name.to_string()));
        }
        if !op_obj.is_empty() {
            result.insert("operator".into(), compact_object(&op_obj));
        }
    }

    if result.is_empty() {
        None
    } else {
        Some(compact_object(&result))
    }
}

/// Summarize a journey leg.
pub fn summarize_leg(value: &Value) -> Option<Value> {
    let obj = value.as_object()?;
    let mut result = serde_json::Map::new();

    for key in &[
        "departure",
        "plannedDeparture",
        "arrival",
        "plannedArrival",
        "direction",
        "tripId",
    ] {
        if let Some(s) = obj.get(*key).and_then(|v| as_str(v)) {
            result.insert((*key).to_string(), Value::String(s.to_string()));
        }
    }
    for key in &["departureDelay", "arrivalDelay", "distance"] {
        if let Some(n) = obj.get(*key).and_then(as_f64) {
            result.insert((*key).to_string(), serde_json::json!(n));
        }
    }
    if obj.get("cancelled") == Some(&Value::Bool(true)) {
        result.insert("cancelled".into(), Value::Bool(true));
    }
    if obj.get("walking") == Some(&Value::Bool(true)) {
        result.insert("walking".into(), Value::Bool(true));
    }
    if let Some(origin) = obj.get("origin").and_then(summarize_place) {
        result.insert("origin".into(), origin);
    }
    if let Some(dest) = obj.get("destination").and_then(summarize_place) {
        result.insert("destination".into(), dest);
    }
    if let Some(line) = obj.get("line").and_then(summarize_line) {
        result.insert("line".into(), line);
    }

    if result.is_empty() {
        None
    } else {
        Some(compact_object(&result))
    }
}

/// Summarize an ÖBB journeys payload (multiple journey options).
pub fn summarize_journeys_payload(payload: &Value) -> Option<Value> {
    let obj = payload.as_object()?;
    let journeys_raw = obj.get("journeys")?.as_array()?;

    let journeys: Vec<Value> = journeys_raw
        .iter()
        .enumerate()
        .map(|(index, journey)| {
            let j_obj = match journey.as_object() {
                Some(o) => o,
                None => return serde_json::json!({"option": index + 1, "error": "invalid-journey"}),
            };
            let legs_raw = j_obj
                .get("legs")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let legs: Vec<Value> = legs_raw.iter().filter_map(summarize_leg).collect();

            let transit_count = legs
                .iter()
                .filter(|l| l.get("walking") != Some(&Value::Bool(true)))
                .count();
            let transfers = if transit_count > 0 {
                transit_count.saturating_sub(1)
            } else {
                legs.len().saturating_sub(1)
            };

            let departure = as_str(journey)
                .map(String::from)
                .or_else(|| {
                    j_obj
                        .get("departure")
                        .and_then(|v| as_str(v))
                        .map(String::from)
                })
                .or_else(|| {
                    legs.first()
                        .and_then(|l| l.get("departure"))
                        .and_then(|v| as_str(v))
                        .map(String::from)
                });
            let arrival = j_obj
                .get("arrival")
                .and_then(|v| as_str(v))
                .map(String::from)
                .or_else(|| {
                    legs.last()
                        .and_then(|l| l.get("arrival"))
                        .and_then(|v| as_str(v))
                        .map(String::from)
                });

            let mut result = serde_json::Map::new();
            result.insert("option".into(), serde_json::json!(index + 1));
            if let Some(d) = departure {
                result.insert("departure".into(), Value::String(d));
            }
            if let Some(a) = arrival {
                result.insert("arrival".into(), Value::String(a));
            }
            result.insert("transfers".into(), serde_json::json!(transfers));
            result.insert("legs".into(), Value::Array(legs));
            compact_object(&result)
        })
        .collect();

    let mut result = serde_json::Map::new();
    if let Some(earlier) = obj.get("earlierRef").and_then(|v| as_str(v)) {
        result.insert("earlierRef".into(), Value::String(earlier.to_string()));
    }
    if let Some(later) = obj.get("laterRef").and_then(|v| as_str(v)) {
        result.insert("laterRef".into(), Value::String(later.to_string()));
    }
    result.insert("journeys".into(), Value::Array(journeys));
    if let Some(stats) = obj.get("filterStats") {
        if stats.is_object() {
            result.insert("filterStats".into(), stats.clone());
        }
    }
    Some(compact_object(&result))
}

/// Normalize text for comparison: lowercase, strip whitespace + non-alphanumeric.
pub fn normalize_comparable_text(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// Check if a string looks like a numeric stop ID.
pub fn looks_like_stop_id(value: &str) -> bool {
    value.trim().chars().all(|c| c.is_ascii_digit()) && !value.trim().is_empty()
}
