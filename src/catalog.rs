use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize)]
pub struct CatalogTool {
    pub name: &'static str,
    pub description: &'static str,
    pub provider: &'static str,
    pub input_schema: Value,
    pub examples: Vec<Value>,
}

pub fn catalog_tools() -> Vec<CatalogTool> {
    vec![
        CatalogTool {
            name: "oebbPlanJourney",
            description: "Plan a single A-to-B train journey on ÖBB. Use this when the user wants to travel from one station to another (not a round trip or multi-city tour). Supports optional intermediate via stations (pass-through or with a stopover/wait), operator exclusion (e.g. exclude WESTbahn), and result selection strategy (fastest, earliest arrival, fewest transfers). For multi-city trips or round trips with planned stops at each city, use oebbPlanTour instead.",
            provider: "oebb",
            input_schema: json!({
            "type": "object",
            "properties": {
                "from": { "type": "string", "minLength": 1, "maxLength": 120 },
                "to": { "type": "string", "minLength": 1, "maxLength": 120 },
                "departure": { "type": "string" },
                "arrival": { "type": "string" },
                "via": {
                    "description": "Intermediate stations: strings for passthrough, objects for planned stopovers with wait time.",
                    "oneOf": [
                        { "type": "string", "minLength": 1, "maxLength": 120 },
                        {
                            "type": "object",
                            "properties": {
                                "station": { "type": "string", "minLength": 1, "maxLength": 120, "description": "Station name or IBNR ID" },
                                "stopType": { "type": "string", "enum": ["passthrough", "stopover"], "description": "passthrough (default): train passes through. stopover: alight, wait, board later train." },
                                "minStopMinutes": { "type": "integer", "minimum": 0, "maximum": 1440, "description": "Minimum wait time at this station in minutes. Implies stopType=stopover." },
                                "maxStopMinutes": { "type": "integer", "minimum": 0, "maximum": 1440, "description": "Maximum acceptable wait time (advisory, triggers warning if exceeded)." }
                            },
                            "required": ["station"]
                        },
                        {
                            "type": "array", "minItems": 1, "maxItems": 6,
                            "items": {
                                "oneOf": [
                                    { "type": "string", "minLength": 1, "maxLength": 120 },
                                    {
                                        "type": "object",
                                        "properties": {
                                            "station": { "type": "string", "minLength": 1, "maxLength": 120 },
                                            "stopType": { "type": "string", "enum": ["passthrough", "stopover"] },
                                            "minStopMinutes": { "type": "integer", "minimum": 0, "maximum": 1440 },
                                            "maxStopMinutes": { "type": "integer", "minimum": 0, "maximum": 1440 }
                                        },
                                        "required": ["station"]
                                    }
                                ]
                            }
                        }
                    ]
                },
                "results": { "type": "integer", "minimum": 1, "maximum": 12 },
                "minTransferMinutes": { "type": "integer", "minimum": 0, "maximum": 120, "description": "Global minimum buffer (minutes) between arriving and departing at stopover vias. Defaults to 0." },
                "strictVia": { "type": "boolean" },
                "excludeOperators": {
                    "oneOf": [
                        { "type": "string", "minLength": 1, "maxLength": 120 },
                        { "type": "array", "minItems": 1, "maxItems": 10, "items": { "type": "string", "minLength": 1, "maxLength": 120 } }
                    ]
                },
                "resolveLocationIds": { "type": "boolean" },
                "selection": { "type": "string", "enum": ["fastest", "earliest_arrival", "fewest_transfers"] },
                "includeStopovers": { "type": "boolean" },
                "includeAlternatives": { "type": "boolean" },
                "responseMode": { "type": "string", "enum": ["summary", "raw"] }
            },
            "required": ["from", "to"]
        }),
        examples: vec![
            json!({
                "from": "Nürnberg Hbf",
                "to": "Amstetten NÖ Bahnhof",
                "departure": "2026-02-23T18:30:00+01:00",
                "excludeOperators": ["WESTbahn"],
                "includeStopovers": true,
                "selection": "fastest"
            }),
            json!({
                "from": "8000284",
                "to": "8100012",
                "selection": "fewest_transfers",
                "results": 8,
                "responseMode": "summary"
            }),
            json!({
                "from": "Amstetten NÖ Bahnhof",
                "to": "Wien Hbf",
                "departure": "2026-02-27T08:00:00+01:00",
                "via": [{ "station": "St. Pölten Hbf", "minStopMinutes": 90 }],
                "excludeOperators": ["WESTbahn"],
                "selection": "earliest_arrival"
            }),
            ],
        },
        CatalogTool {
            name: "oebbPlanTour",
            description: "Plan a multi-city train tour or round trip on ÖBB. Use this when the user wants to visit multiple cities in sequence with planned stops/layovers at each city (e.g. 'go from A to B, stay 1.5h, then to C, stay 1h, then home via D'). Each leg is defined with from/to, and minStopMinutesAfter controls the minimum layover time at the destination before starting the next leg. Supports operator exclusion (e.g. exclude WESTbahn), per-leg via stations (passthrough only — for planned stops at a city, add a separate leg instead), and selection strategy. The departure time of each subsequent leg is automatically computed from the previous leg's arrival plus the stop duration. Use `via` on a leg when the train should pass through a station; use a separate leg when the user wants to actually stop and spend time in that city.",
            provider: "oebb",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "legs": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 8,
                        "items": {
                            "type": "object",
                            "properties": {
                                "from": { "type": "string", "minLength": 1, "maxLength": 120 },
                                "to": { "type": "string", "minLength": 1, "maxLength": 120 },
                                "via": {
                                    "oneOf": [
                                        { "type": "string", "minLength": 1, "maxLength": 120 },
                                        {
                                            "type": "array", "minItems": 1, "maxItems": 6,
                                            "items": {
                                                "oneOf": [
                                                    { "type": "string", "minLength": 1, "maxLength": 120 },
                                                    {
                                                        "type": "object",
                                                        "properties": {
                                                            "station": { "type": "string", "minLength": 1, "maxLength": 120 },
                                                            "stopType": { "type": "string", "enum": ["passthrough"] }
                                                        },
                                                        "required": ["station"]
                                                    }
                                                ]
                                            }
                                        }
                                    ]
                                },
                                "minStopMinutesAfter": { "type": "integer", "minimum": 0, "maximum": 1440 },
                                "maxStopMinutesAfter": { "type": "integer", "minimum": 0, "maximum": 1440 }
                            },
                            "required": ["from", "to"]
                        }
                    },
                    "departure": { "type": "string" },
                    "datetime": { "type": "string" },
                    "type": { "type": "string", "enum": ["departure"] },
                    "results": { "type": "integer", "minimum": 1, "maximum": 12 },
                    "minTransferMinutes": { "type": "integer", "minimum": 0, "maximum": 120 },
                    "excludeOperators": {
                        "oneOf": [
                            { "type": "string", "minLength": 1, "maxLength": 120 },
                            { "type": "array", "minItems": 1, "maxItems": 10, "items": { "type": "string", "minLength": 1, "maxLength": 120 } }
                        ]
                    },
                    "resolveLocationIds": { "type": "boolean" },
                    "selection": { "type": "string", "enum": ["fastest", "earliest_arrival", "fewest_transfers"] },
                    "includeStopovers": { "type": "boolean" },
                    "responseMode": { "type": "string", "enum": ["summary", "raw"] }
                },
                "required": ["legs"]
            }),
            examples: vec![
                json!({
                    "departure": "2026-02-27T08:00:00+01:00",
                    "excludeOperators": ["WESTbahn"],
                    "selection": "earliest_arrival",
                    "legs": [
                        {
                            "from": "Amstetten NÖ Bahnhof",
                            "to": "Salzburg Hbf",
                            "minStopMinutesAfter": 90
                        },
                        {
                            "from": "Salzburg Hbf",
                            "to": "Klagenfurt Hbf",
                            "minStopMinutesAfter": 60
                        },
                        {
                            "from": "Klagenfurt Hbf",
                            "to": "Wien Meidling Bahnhof"
                        },
                        {
                            "from": "Wien Meidling Bahnhof",
                            "to": "Amstetten NÖ Bahnhof"
                        }
                    ]
                })
            ],
        },
        CatalogTool {
            name: "oebbResolveItineraryStops",
            description: "Pre-validate and resolve station names to ÖBB station IDs before calling oebbPlanTour. Use this when station names are ambiguous (e.g. 'Amstetten' could be in NÖ or elsewhere). Returns resolved legs ready for oebbPlanTour, plus any ambiguities or errors. Optional but recommended for user-provided station names.",
            provider: "oebb",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "legs": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 8,
                        "items": {
                            "type": "object",
                            "properties": {
                                "from": { "type": "string", "minLength": 1, "maxLength": 120 },
                                "to": { "type": "string", "minLength": 1, "maxLength": 120 },
                                "via": {
                                    "oneOf": [
                                        { "type": "string", "minLength": 1, "maxLength": 120 },
                                        {
                                            "type": "array",
                                            "minItems": 1,
                                            "maxItems": 6,
                                            "items": {
                                                "oneOf": [
                                                    { "type": "string", "minLength": 1, "maxLength": 120 },
                                                    {
                                                        "type": "object",
                                                        "properties": {
                                                            "station": { "type": "string", "minLength": 1, "maxLength": 120 }
                                                        },
                                                        "required": ["station"]
                                                    }
                                                ]
                                            }
                                        }
                                    ]
                                }
                            },
                            "required": ["from", "to"]
                        }
                    },
                    "resolveLocationIds": { "type": "boolean" }
                },
                "required": ["legs"]
            }),
            examples: vec![
                json!({
                    "legs": [
                        { "from": "Amstetten", "to": "Salzburg Hbf" },
                        { "from": "Salzburg Hbf", "to": "Klagenfurt Hbf" },
                        { "from": "Klagenfurt Hbf", "to": "Amstetten", "via": ["Wien Hbf"] }
                    ]
                })
            ],
        },
        CatalogTool {
            name: "oebbLocations",
            description: "Search for ÖBB stations, addresses, or points of interest by name. Use this to find the exact station name or IBNR ID when the user gives an ambiguous or partial station name. Returns matching locations with IDs.",
            provider: "oebb",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "minLength": 1, "maxLength": 120 },
                    "results": { "type": "integer", "minimum": 1, "maximum": 20 },
                    "stops": { "type": "boolean" },
                    "addresses": { "type": "boolean" },
                    "poi": { "type": "boolean" }
                },
                "required": ["query"]
            }),
            examples: vec![json!({ "query": "Amstetten", "results": 5, "stops": true })],
        },
        CatalogTool {
            name: "oebbDepartures",
            description: "Get upcoming departures from a station. Use this when the user asks 'when does the next train leave from X?' or wants to see a departure board. Accepts station names (auto-resolved) or IBNR station IDs. Returns a list of departing trains with times, destinations, and platform info.",
            provider: "oebb",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "stopId": { "type": "string", "minLength": 1 },
                    "station": { "type": "string", "minLength": 1, "description": "Alias for stopId; can be IBNR or station name." },
                    "stop": { "type": "string", "minLength": 1, "description": "Alias for stopId." },
                    "duration": { "type": "integer", "minimum": 1, "maximum": 1440 },
                    "when": { "type": "string" },
                    "results": { "type": "integer", "minimum": 1, "maximum": 200 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200, "description": "Alias for results." },
                    "stopovers": { "type": "boolean" },
                    "remarks": { "type": "boolean" },
                    "resolveLocationIds": { "type": "boolean", "description": "Default true. Resolve station names to IBNR." }
                },
                "anyOf": [
                    { "required": ["stopId"] },
                    { "required": ["station"] },
                    { "required": ["stop"] }
                ]
            }),
            examples: vec![
                json!({ "station": "Amstetten NÖ Bahnhof", "duration": 120, "limit": 10 }),
                json!({ "stopId": "1230501", "results": 5, "remarks": true })
            ],
        },
        CatalogTool {
            name: "oebbJourneys",
            description: "Query raw ÖBB journey alternatives between two stations. Low-level tool that returns multiple journey options without automatic selection or filtering. Prefer oebbPlanJourney for single trips (adds selection/filtering) or oebbPlanTour for multi-city tours.",
            provider: "oebb",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "from": { "type": "string", "minLength": 1 },
                    "to": { "type": "string", "minLength": 1 },
                    "results": { "type": "integer", "minimum": 1, "maximum": 20 },
                    "departure": { "type": "string" },
                    "arrival": { "type": "string" },
                    "stopovers": { "type": "boolean" }
                },
                "required": ["from", "to"]
            }),
            examples: vec![json!({ "from": "8100012", "to": "8100003", "results": 6 })],
        },
        CatalogTool {
            name: "oebbTrip",
            description: "Fetch full details for a specific trip by its trip ID (returned from journey/departure results). Use this to get stopovers, remarks, and detailed schedule for one specific train service.",
            provider: "oebb",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "tripId": { "type": "string", "minLength": 1 },
                    "stopovers": { "type": "boolean" },
                    "remarks": { "type": "boolean" }
                },
                "required": ["tripId"]
            }),
            examples: vec![json!({ "tripId": "2|#VN#1#...", "stopovers": true })],
        },
    ]
}

pub fn create_catalog_payload() -> Value {
    let tools = catalog_tools();
    json!({ "tools": tools })
}
