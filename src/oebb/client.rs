use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use moka::future::Cache;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::metrics;

use super::filter::filter_journeys;
use super::normalize::{
    assert_payload_size, compact_json, compact_object, looks_like_stop_id,
    normalize_comparable_text, summarize_place,
};
use super::rank::{
    extract_intermediate_stops_from_leg, rank_journeys, summarize_ranked_journey, RankedJourney,
};
use super::types::*;

const LOCATION_RESOLVE_RESULTS: u32 = 8;
const LOCATION_AMBIGUOUS_CANDIDATES: usize = 5;
const RETRYABLE_STATUSES: &[u16] = &[429, 500, 502, 503, 504];

/// Async ÖBB REST API client with retry, TTL caching, and in-flight dedup.
#[derive(Clone)]
pub struct OebbClient {
    http: reqwest::Client,
    config: OebbConfig,
    cache: Cache<String, Value>,
    /// Gap 5: In-flight request deduplication.
    inflight: Arc<Mutex<HashMap<String, Arc<tokio::sync::Notify>>>>,
}

impl OebbClient {
    pub fn new(base_url: String) -> Self {
        let config = OebbConfig::new(base_url);
        let http = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .expect("failed to build reqwest client");
        let cache = Cache::builder().max_capacity(1000).build();
        Self {
            http,
            config,
            cache,
            inflight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Fetch JSON from an ÖBB API endpoint with retry, caching, and in-flight dedup.
    async fn fetch_json(
        &self,
        path: &str,
        query: &[(String, String)],
        cache_key: &str,
        ttl: Duration,
    ) -> anyhow::Result<Value> {
        // Check cache first.
        if let Some(cached) = self.cache.get(cache_key).await {
            metrics::observe_oebb_cache_event("hit");
            return Ok(cached);
        }
        metrics::observe_oebb_cache_event("miss");

        // Gap 5: In-flight deduplication — if another task is fetching the same key, wait for it.
        {
            let inflight = self.inflight.lock().await;
            if let Some(notify) = inflight.get(cache_key) {
                metrics::observe_oebb_cache_event("inflight_wait");
                let notify = notify.clone();
                drop(inflight);
                notify.notified().await;
                // After notification, the result should be in cache.
                if let Some(cached) = self.cache.get(cache_key).await {
                    metrics::observe_oebb_cache_event("hit_after_wait");
                    return Ok(cached);
                }
                metrics::observe_oebb_cache_event("miss_after_wait");
                // If not in cache after notification, fall through to fetch.
            }
        }

        // Register ourselves as the in-flight fetcher.
        let notify = Arc::new(tokio::sync::Notify::new());
        {
            let mut inflight = self.inflight.lock().await;
            inflight.insert(cache_key.to_string(), notify.clone());
        }

        let result = self.do_fetch(path, query, cache_key, ttl).await;

        // Notify waiters and remove from in-flight.
        {
            let mut inflight = self.inflight.lock().await;
            inflight.remove(cache_key);
        }
        notify.notify_waiters();

        result
    }

    /// Perform the actual HTTP fetch with retries.
    async fn do_fetch(
        &self,
        path: &str,
        query: &[(String, String)],
        cache_key: &str,
        ttl: Duration,
    ) -> anyhow::Result<Value> {
        let endpoint = normalize_oebb_endpoint(path);
        let url = format!("{}{}", self.config.base_url, path);
        let mut last_error: Option<anyhow::Error> = None;
        let started = Instant::now();

        for attempt in 1..=self.config.max_attempts {
            if started.elapsed() >= Duration::from_millis(self.config.retry_budget_ms) {
                break;
            }

            let attempt_started = Instant::now();
            let result = self
                .http
                .get(&url)
                .query(query)
                .header("accept", "application/json")
                .send()
                .await;

            match result {
                Ok(response) => {
                    let status = response.status();
                    let status_code = status.as_u16().to_string();
                    if !status.is_success() {
                        let body = response.text().await.unwrap_or_default();
                        metrics::observe_oebb_upstream_attempt(
                            endpoint,
                            &status_code,
                            "http_error",
                            attempt_started.elapsed(),
                        );
                        let msg = format!(
                            "Transit API error {}: {}",
                            status_code,
                            &body[..body.len().min(500)]
                        );
                        if RETRYABLE_STATUSES.contains(&status.as_u16())
                            && attempt < self.config.max_attempts
                        {
                            last_error = Some(anyhow::anyhow!(msg));
                            metrics::observe_oebb_retry(endpoint, "retryable_status");
                            let delay = compute_retry_delay_ms(
                                attempt,
                                self.config.retry_base_delay_ms,
                                self.config.retry_max_delay_ms,
                            );
                            if !sleep_within_retry_budget(
                                started,
                                self.config.retry_budget_ms,
                                delay,
                            )
                            .await
                            {
                                break;
                            }
                            continue;
                        }
                        return Err(anyhow::anyhow!(msg));
                    }
                    let payload: Value = match response.json().await {
                        Ok(payload) => payload,
                        Err(error) => {
                            metrics::observe_oebb_upstream_attempt(
                                endpoint,
                                "decode_error",
                                "decode_error",
                                attempt_started.elapsed(),
                            );
                            return Err(error.into());
                        }
                    };
                    metrics::observe_oebb_upstream_attempt(
                        endpoint,
                        &status_code,
                        "success",
                        attempt_started.elapsed(),
                    );
                    // Insert into cache with TTL.
                    self.cache
                        .insert(cache_key.to_string(), payload.clone())
                        .await;
                    let cache = self.cache.clone();
                    let key = cache_key.to_string();
                    tokio::spawn(async move {
                        tokio::time::sleep(ttl).await;
                        cache.invalidate(&key).await;
                    });
                    return Ok(payload);
                }
                Err(e) => {
                    let is_timeout = e.is_timeout();
                    last_error = Some(e.into());
                    if is_timeout {
                        metrics::observe_oebb_timeout(endpoint);
                    }
                    metrics::observe_oebb_upstream_attempt(
                        endpoint,
                        if is_timeout {
                            "timeout"
                        } else {
                            "network_error"
                        },
                        "network_error",
                        attempt_started.elapsed(),
                    );
                    if attempt < self.config.max_attempts {
                        metrics::observe_oebb_retry(
                            endpoint,
                            if is_timeout {
                                "timeout"
                            } else {
                                "network_error"
                            },
                        );
                        let delay = compute_retry_delay_ms(
                            attempt,
                            self.config.retry_base_delay_ms,
                            self.config.retry_max_delay_ms,
                        );
                        if !sleep_within_retry_budget(started, self.config.retry_budget_ms, delay)
                            .await
                        {
                            break;
                        }
                    }
                }
            }
        }

        let elapsed_ms = started.elapsed().as_millis();
        Err(last_error.unwrap_or_else(|| {
            anyhow::anyhow!(
                "Transit request failed after {} attempt(s) over {}ms",
                self.config.max_attempts,
                elapsed_ms
            )
        }))
    }

    /// Resolve a location name to a stop ID.
    pub async fn resolve_location(
        &self,
        tool_name: &str,
        field: &str,
        value: &str,
        resolve: bool,
    ) -> LocationResolution {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return LocationResolution::Err(serde_json::json!({
                "error": "invalid_location",
                "message": format!("Field \"{}\" cannot be empty.", field),
                "field": field
            }));
        }
        if !resolve || looks_like_stop_id(trimmed) {
            return LocationResolution::Ok {
                id: trimmed.to_string(),
                name: None,
                resolved: false,
            };
        }

        let query = vec![
            ("query".to_string(), trimmed.to_string()),
            ("results".to_string(), LOCATION_RESOLVE_RESULTS.to_string()),
            ("stops".to_string(), "true".to_string()),
            ("addresses".to_string(), "false".to_string()),
            ("poi".to_string(), "false".to_string()),
        ];
        let cache_key = format!("{}:resolve:{}:{}", tool_name, field, trimmed);

        let payload = match self
            .fetch_json("/locations", &query, &cache_key, Duration::from_secs(60))
            .await
        {
            Ok(p) => p,
            Err(e) => {
                return LocationResolution::Err(serde_json::json!({
                    "error": "location_lookup_failed",
                    "message": e.to_string(),
                    "field": field
                }));
            }
        };

        let items = extract_location_items(&payload);
        let mut candidates: Vec<LocationCandidate> = items
            .into_iter()
            .filter_map(|entry| {
                let id = entry.get("id")?.as_str()?.to_string();
                let name = entry.get("name")?.as_str()?.to_string();
                let loc_type = entry.get("type").and_then(|v| v.as_str()).map(String::from);
                let score = score_location_candidate(trimmed, &name, &id, loc_type.as_deref());
                if id.is_empty() || name.is_empty() {
                    return None;
                }
                Some(LocationCandidate {
                    id,
                    name,
                    loc_type,
                    score,
                })
            })
            .collect();

        candidates.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.id.cmp(&b.id))
        });

        if candidates.is_empty() {
            return LocationResolution::Err(serde_json::json!({
                "error": "location_not_found",
                "message": format!("No stop matched \"{}\" for field \"{}\".", trimmed, field),
                "field": field,
                "query": trimmed
            }));
        }

        let top_score = candidates[0].score;
        let top_candidates: Vec<&LocationCandidate> =
            candidates.iter().filter(|c| c.score == top_score).collect();

        if top_candidates.len() > 1 {
            let shown: Vec<Value> = top_candidates
                .iter()
                .take(LOCATION_AMBIGUOUS_CANDIDATES)
                .map(|c| {
                    serde_json::json!({
                        "id": c.id,
                        "name": c.name,
                        "type": c.loc_type,
                    })
                })
                .collect();
            return LocationResolution::Err(serde_json::json!({
                "error": "ambiguous_location",
                "message": format!("Location \"{}\" is ambiguous for field \"{}\". Provide an explicit station ID (IBNR).", trimmed, field),
                "field": field,
                "query": trimmed,
                "candidates": shown
            }));
        }

        let top = &candidates[0];
        LocationResolution::Ok {
            id: top.id.clone(),
            name: Some(top.name.clone()),
            resolved: true,
        }
    }

    /// Resolve all stop names/IDs for an itinerary before planning.
    pub async fn resolve_itinerary_stops(
        &self,
        input: ResolveItineraryStopsInput,
    ) -> anyhow::Result<Value> {
        if input.legs.is_empty() {
            return Ok(serde_json::json!({
                "error": "invalid_input",
                "message": "Field \"legs\" must contain at least one leg.",
            }));
        }

        let should_resolve = input.resolve_location_ids.unwrap_or(true);
        let mut resolved_legs: Vec<Value> = Vec::new();
        let mut retry_with_legs: Vec<Value> = Vec::new();
        let mut errors: Vec<Value> = Vec::new();

        for (idx, leg) in input.legs.iter().enumerate() {
            let leg_number = idx + 1;
            let mut from_obj = serde_json::Map::new();
            from_obj.insert("input".into(), Value::String(leg.from.clone()));
            let mut to_obj = serde_json::Map::new();
            to_obj.insert("input".into(), Value::String(leg.to.clone()));
            let mut via_arr: Vec<Value> = Vec::new();

            let mut retry_from = leg.from.clone();
            let mut retry_to = leg.to.clone();
            let mut retry_via: Vec<Value> = Vec::new();

            let errors_before_leg = errors.len();

            if leg.from.trim().is_empty() {
                let err = serde_json::json!({
                    "error": "invalid_location",
                    "message": "Field \"from\" cannot be empty.",
                });
                from_obj.insert("error".into(), err.clone());
                errors.push(build_resolution_error(leg_number, "from", &leg.from, &err));
            } else {
                match self
                    .resolve_location(
                        "oebbResolveItineraryStops",
                        "from",
                        &leg.from,
                        should_resolve,
                    )
                    .await
                {
                    LocationResolution::Ok { id, name, .. } => {
                        retry_from = id.clone();
                        from_obj.insert("resolvedId".into(), Value::String(id));
                        if let Some(name) = name {
                            from_obj.insert("resolvedName".into(), Value::String(name));
                        }
                    }
                    LocationResolution::Err(err) => {
                        from_obj.insert("error".into(), err.clone());
                        errors.push(build_resolution_error(leg_number, "from", &leg.from, &err));
                    }
                }
            }

            if leg.to.trim().is_empty() {
                let err = serde_json::json!({
                    "error": "invalid_location",
                    "message": "Field \"to\" cannot be empty.",
                });
                to_obj.insert("error".into(), err.clone());
                errors.push(build_resolution_error(leg_number, "to", &leg.to, &err));
            } else {
                match self
                    .resolve_location("oebbResolveItineraryStops", "to", &leg.to, should_resolve)
                    .await
                {
                    LocationResolution::Ok { id, name, .. } => {
                        retry_to = id.clone();
                        to_obj.insert("resolvedId".into(), Value::String(id));
                        if let Some(name) = name {
                            to_obj.insert("resolvedName".into(), Value::String(name));
                        }
                    }
                    LocationResolution::Err(err) => {
                        to_obj.insert("error".into(), err.clone());
                        errors.push(build_resolution_error(leg_number, "to", &leg.to, &err));
                    }
                }
            }

            let via_specs: Vec<ViaSpec> = leg
                .via
                .clone()
                .map(|v| v.to_vec())
                .unwrap_or_default()
                .into_iter()
                .filter(|s| !s.station().is_empty())
                .collect();

            for via in via_specs {
                let station = via.station().to_string();
                let mut via_obj = serde_json::Map::new();
                via_obj.insert("input".into(), Value::String(station.clone()));

                match self
                    .resolve_location("oebbResolveItineraryStops", "via", &station, should_resolve)
                    .await
                {
                    LocationResolution::Ok { id, name, .. } => {
                        retry_via.push(Value::String(id.clone()));
                        via_obj.insert("resolvedId".into(), Value::String(id));
                        if let Some(name) = name {
                            via_obj.insert("resolvedName".into(), Value::String(name));
                        }
                    }
                    LocationResolution::Err(err) => {
                        retry_via.push(Value::String(station.clone()));
                        via_obj.insert("error".into(), err.clone());
                        errors.push(build_resolution_error(leg_number, "via", &station, &err));
                    }
                }

                via_arr.push(compact_object(&via_obj));
            }

            let status = if errors.len() == errors_before_leg {
                "ok"
            } else {
                "partial"
            };

            resolved_legs.push(serde_json::json!({
                "leg": leg_number,
                "status": status,
                "from": compact_object(&from_obj),
                "to": compact_object(&to_obj),
                "via": via_arr,
            }));

            let mut retry_leg = serde_json::Map::new();
            retry_leg.insert("from".into(), Value::String(retry_from));
            retry_leg.insert("to".into(), Value::String(retry_to));
            if !retry_via.is_empty() {
                retry_leg.insert("via".into(), Value::Array(retry_via));
            }
            retry_with_legs.push(Value::Object(retry_leg));
        }

        Ok(serde_json::json!({
            "ok": errors.is_empty(),
            "query": {
                "resolveLocationIds": should_resolve,
                "legs": input.legs,
            },
            "resolvedLegs": resolved_legs,
            "retryWithLegs": retry_with_legs,
            "errors": errors,
        }))
    }

    /// Plan a journey end-to-end.
    /// Gap 2: Validates inputs (empty from/to, departure+arrival conflict, results range).
    pub async fn plan_journey(&self, mut input: PlanJourneyInput) -> anyhow::Result<Value> {
        // Normalize datetime/type aliases into departure/arrival.
        if let Some(dt) = input.datetime.take() {
            if input.departure.is_none() && input.arrival.is_none() {
                match input.datetime_type.as_deref() {
                    Some("arrival") => input.arrival = Some(dt),
                    _ => input.departure = Some(dt), // default to departure
                }
            }
        }

        // Gap 2: Input validation — match Zod .superRefine().
        if input.from.trim().is_empty() {
            return Ok(serde_json::json!({
                "error": "invalid_input",
                "message": "Field \"from\" cannot be empty.",
            }));
        }
        if input.to.trim().is_empty() {
            return Ok(serde_json::json!({
                "error": "invalid_input",
                "message": "Field \"to\" cannot be empty.",
            }));
        }
        if input.departure.is_some() && input.arrival.is_some() {
            return Ok(serde_json::json!({
                "error": "invalid_input",
                "message": "Provide either departure or arrival, not both.",
            }));
        }

        let selection = input.selection.unwrap_or(JourneySelection::Fastest);
        let should_resolve = input.resolve_location_ids.unwrap_or(true);
        let include_stopovers = input.include_stopovers.unwrap_or(true);
        let include_alternatives = input.include_alternatives.unwrap_or(false);
        let mode = input.response_mode.unwrap_or(ResponseMode::Summary);
        let results = input.results.unwrap_or(6).clamp(1, 12);

        // Resolve from.
        let from_resolved = self
            .resolve_location("oebbPlanJourney", "from", &input.from, should_resolve)
            .await;
        let (from_id, from_name) = match from_resolved {
            LocationResolution::Ok { id, name, .. } => (id, name),
            LocationResolution::Err(e) => return Ok(e),
        };

        // Resolve to.
        let to_resolved = self
            .resolve_location("oebbPlanJourney", "to", &input.to, should_resolve)
            .await;
        let (to_id, to_name) = match to_resolved {
            LocationResolution::Ok { id, name, .. } => (id, name),
            LocationResolution::Err(e) => return Ok(e),
        };

        // Save values for potential chained planning (before via moves input fields).
        let departure_clone = input.departure.clone();
        let arrival_clone = input.arrival.clone();
        let from_str_clone = input.from.clone();
        let to_str_clone = input.to.clone();
        let exclude_operators_clone = input.exclude_operators.clone();
        let _strict_via_opt = input.strict_via;
        let min_transfer_minutes = input.min_transfer_minutes.unwrap_or(0);

        // Resolve via.
        let via_specs: Vec<ViaSpec> = input
            .via
            .map(|v| v.to_vec())
            .unwrap_or_default()
            .into_iter()
            .filter(|s| !s.station().is_empty())
            .collect();

        let mut via_resolved: Vec<ViaResolution> = Vec::new();
        for via_spec in &via_specs {
            let station = via_spec.station();
            let resolved = self
                .resolve_location("oebbPlanJourney", "via", station, should_resolve)
                .await;
            match resolved {
                LocationResolution::Ok { id, name, .. } => {
                    via_resolved.push(ViaResolution {
                        input: station.to_string(),
                        id: Some(id),
                        name,
                        is_stopover: via_spec.is_stopover(),
                        min_stop_minutes: via_spec.min_stop_minutes(),
                        max_stop_minutes: via_spec.max_stop_minutes(),
                    });
                }
                LocationResolution::Err(e) => return Ok(e),
            }
        }

        // Check if chained planning is needed (any stopover vias).
        let has_stopover_vias = via_resolved.iter().any(|v| v.is_stopover);
        if has_stopover_vias {
            let exclude_ops: Vec<String> = exclude_operators_clone
                .map(|v| v.to_vec())
                .unwrap_or_default()
                .into_iter()
                .map(|s| normalize_operator_alias(&s))
                .filter(|s| !s.is_empty())
                .collect();

            return self
                .plan_chained_journey(
                    from_id,
                    from_name,
                    &from_str_clone,
                    to_id,
                    to_name,
                    &to_str_clone,
                    via_resolved,
                    departure_clone.as_deref(),
                    arrival_clone.as_deref(),
                    selection,
                    include_stopovers,
                    include_alternatives,
                    &exclude_ops,
                    results,
                    mode,
                    min_transfer_minutes,
                    should_resolve,
                )
                .await;
        }

        // Build query.
        let mut query: Vec<(String, String)> = vec![
            ("from".into(), from_id.clone()),
            ("to".into(), to_id.clone()),
            ("results".into(), results.to_string()),
        ];
        for via in &via_resolved {
            query.push((
                "via".into(),
                via.id.clone().unwrap_or_else(|| via.input.clone()),
            ));
        }
        if let Some(ref dep) = input.departure {
            query.push(("departure".into(), dep.clone()));
        } else if let Some(ref arr) = input.arrival {
            query.push(("arrival".into(), arr.clone()));
        } else {
            query.push(("departure".into(), chrono::Utc::now().to_rfc3339()));
        }
        if include_stopovers {
            query.push(("stopovers".into(), "true".into()));
        }

        let strict_via = input.strict_via.unwrap_or(!via_resolved.is_empty());
        let exclude_operators: Vec<String> = input
            .exclude_operators
            .map(|v| v.to_vec())
            .unwrap_or_default()
            .into_iter()
            .map(|s| normalize_operator_alias(&s))
            .filter(|s| !s.is_empty())
            .collect();

        let cache_key = format!(
            "oebbPlanJourney:{}:{}:{}",
            from_id,
            to_id,
            serde_json::to_string(&query).unwrap_or_default()
        );

        let payload = self
            .fetch_json("/journeys", &query, &cache_key, Duration::from_secs(25))
            .await?;

        // Filter journeys.
        let filtered = filter_journeys(&payload, &exclude_operators, strict_via, &via_resolved);
        let journeys: Vec<Value> = filtered
            .get("journeys")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|j| j.is_object())
            .collect();
        let filter_stats = filtered.get("filterStats").cloned();

        // Build query info.
        let query_info = serde_json::json!({
            "from": { "input": input.from, "resolvedId": from_id, "resolvedName": from_name },
            "to": { "input": input.to, "resolvedId": to_id, "resolvedName": to_name },
            "via": via_resolved.iter().map(|v| serde_json::json!({
                "input": v.input,
                "resolvedId": v.id,
                "resolvedName": v.name,
            })).collect::<Vec<_>>(),
            "departure": input.departure.as_deref().or_else(|| if input.arrival.is_none() { Some("now") } else { None }),
            "arrival": input.arrival,
            "results": results,
            "strictVia": strict_via,
            "excludeOperators": exclude_operators,
            "resolveLocationIds": should_resolve,
            "includeStopovers": include_stopovers,
            "includeAlternatives": include_alternatives,
        });

        if journeys.is_empty() {
            return Ok(serde_json::json!({
                "error": "no_matching_journeys",
                "message": "No journeys matched the requested constraints.",
                "query": query_info,
                "filterStats": filter_stats,
            }));
        }

        // Rank journeys.
        let mut ranked = rank_journeys(&journeys, selection);

        // Enrich stopovers.
        let mut warnings: Vec<String> = Vec::new();
        if include_stopovers {
            for rj in ranked.iter_mut() {
                self.enrich_stopovers(rj, &mut warnings).await;
            }
        }

        let selected = &ranked[0];
        let alternatives: Vec<&RankedJourney> = if include_alternatives {
            ranked.iter().skip(1).collect()
        } else {
            vec![]
        };

        // Build response.
        let mode_str = match mode {
            ResponseMode::Raw => "raw",
            ResponseMode::Summary => "summary",
        };

        let mut result = serde_json::Map::new();
        result.insert("query".into(), query_info);
        result.insert("selection".into(), serde_json::json!(selection));

        if mode == ResponseMode::Raw {
            result.insert(
                "selectedJourney".into(),
                serde_json::json!({
                    "journey": selected.journey,
                    "enriched": summarize_ranked_journey(selected, include_stopovers),
                }),
            );
        } else {
            result.insert(
                "selectedJourney".into(),
                summarize_ranked_journey(selected, include_stopovers),
            );
        }

        if include_alternatives && !alternatives.is_empty() {
            let alt_values: Vec<Value> = alternatives
                .iter()
                .map(|j| {
                    if mode == ResponseMode::Raw {
                        serde_json::json!({
                            "journey": j.journey,
                            "enriched": summarize_ranked_journey(j, include_stopovers),
                        })
                    } else {
                        summarize_ranked_journey(j, include_stopovers)
                    }
                })
                .collect();
            result.insert("alternatives".into(), Value::Array(alt_values));
        }

        if let Some(stats) = filter_stats {
            result.insert("filterStats".into(), stats);
        }
        if !warnings.is_empty() {
            result.insert("warnings".into(), serde_json::json!(warnings));
        }

        let output = compact_object(&result);

        // Gap 3+4: Structured payload-too-large error with compactJson fallback.
        if assert_payload_size(&output, mode_str).is_err() {
            // Try compacting before giving up.
            let compacted = compact_json(&output, 0);
            if let Err(e2) = assert_payload_size(&compacted, mode_str) {
                return Ok(e2.to_tool_error());
            }
            return Ok(compacted);
        }

        Ok(output)
    }

    /// Plan a deterministic multi-leg tour with optional stop durations between legs.
    pub async fn plan_tour(&self, mut input: PlanTourInput) -> anyhow::Result<Value> {
        if let Some(dt) = input.datetime.take() {
            if input.departure.is_none() {
                match input.datetime_type.as_deref() {
                    Some("arrival") => {
                        return Ok(serde_json::json!({
                            "error": "invalid_input",
                            "message": "oebbPlanTour only supports departure-based planning.",
                        }));
                    }
                    _ => input.departure = Some(dt),
                }
            }
        }

        if input.legs.is_empty() {
            return Ok(serde_json::json!({
                "error": "invalid_input",
                "message": "Field \"legs\" must contain at least one leg.",
            }));
        }

        for (idx, leg) in input.legs.iter().enumerate() {
            if leg.from.trim().is_empty() || leg.to.trim().is_empty() {
                return Ok(serde_json::json!({
                    "error": "invalid_input",
                    "message": format!("Leg {} must have non-empty from/to.", idx + 1),
                }));
            }
        }

        let selection = input.selection.unwrap_or(JourneySelection::Fastest);
        let should_resolve = input.resolve_location_ids.unwrap_or(true);
        let include_stopovers = input.include_stopovers.unwrap_or(true);
        let mode = input.response_mode.unwrap_or(ResponseMode::Summary);
        let results = input.results.unwrap_or(6).clamp(1, 12);
        let min_transfer_minutes = input.min_transfer_minutes.unwrap_or(0);
        let exclude_operators: Vec<String> = input
            .exclude_operators
            .map(|v| v.to_vec())
            .unwrap_or_default()
            .into_iter()
            .map(|s| normalize_operator_alias(&s))
            .filter(|s| !s.is_empty())
            .collect();

        let mut segment_results: Vec<SegmentPlanResult> = Vec::new();
        let mut segment_query: Vec<Value> = Vec::new();
        let mut planned_stopovers: Vec<Value> = Vec::new();

        let mut next_departure = input.departure.clone();

        for (idx, leg) in input.legs.iter().enumerate() {
            let from_resolved = self
                .resolve_location("oebbPlanTour", "from", &leg.from, should_resolve)
                .await;
            let (from_id, from_name) = match from_resolved {
                LocationResolution::Ok { id, name, .. } => (id, name),
                LocationResolution::Err(e) => return Ok(e),
            };

            let to_resolved = self
                .resolve_location("oebbPlanTour", "to", &leg.to, should_resolve)
                .await;
            let (to_id, to_name) = match to_resolved {
                LocationResolution::Ok { id, name, .. } => (id, name),
                LocationResolution::Err(e) => return Ok(e),
            };

            let via_specs: Vec<ViaSpec> = leg
                .via
                .clone()
                .map(|v| v.to_vec())
                .unwrap_or_default()
                .into_iter()
                .filter(|s| !s.station().is_empty())
                .collect();

            let mut via_resolved: Vec<ViaResolution> = Vec::new();
            for via_spec in &via_specs {
                if via_spec.is_stopover() {
                    return Ok(serde_json::json!({
                        "error": "invalid_input",
                        "message": format!("Leg {} contains stopover via; use minStopMinutesAfter on the leg instead.", idx + 1),
                    }));
                }
                let station = via_spec.station();
                let resolved = self
                    .resolve_location("oebbPlanTour", "via", station, should_resolve)
                    .await;
                match resolved {
                    LocationResolution::Ok { id, name, .. } => {
                        via_resolved.push(ViaResolution {
                            input: station.to_string(),
                            id: Some(id),
                            name,
                            is_stopover: false,
                            min_stop_minutes: None,
                            max_stop_minutes: None,
                        });
                    }
                    LocationResolution::Err(e) => return Ok(e),
                }
            }

            let segment_departure = next_departure.clone();
            let segment = self
                .plan_segment(
                    &from_id,
                    &to_id,
                    segment_departure.as_deref(),
                    &via_resolved,
                    selection,
                    include_stopovers,
                    &exclude_operators,
                    results,
                )
                .await?;

            let segment = match segment {
                Ok(s) => s,
                Err(e) => return Ok(e),
            };

            segment_query.push(serde_json::json!({
                "leg": idx + 1,
                "from": { "input": leg.from, "resolvedId": from_id, "resolvedName": from_name },
                "to": { "input": leg.to, "resolvedId": to_id, "resolvedName": to_name },
                "via": via_resolved.iter().map(|v| serde_json::json!({
                    "input": v.input,
                    "resolvedId": v.id,
                    "resolvedName": v.name,
                })).collect::<Vec<_>>(),
                "departure": segment_departure,
            }));

            segment_results.push(segment);

            if idx < input.legs.len() - 1 {
                let min_wait = std::cmp::max(
                    leg.min_stop_minutes_after.unwrap_or(0),
                    min_transfer_minutes,
                );
                let max_wait = leg.max_stop_minutes_after;

                let current = segment_results.last().unwrap();
                if let Some(ref arr) = current.selected.arrival {
                    if let Ok(arr_dt) = chrono::DateTime::parse_from_rfc3339(arr) {
                        let next_dep = arr_dt + chrono::Duration::minutes(min_wait as i64);
                        next_departure = Some(next_dep.to_rfc3339());
                    }
                }

                let departure_for_stop = next_departure.clone().unwrap_or_default();
                let arrival_for_stop = current.selected.arrival.clone().unwrap_or_default();
                let wait_minutes = compute_wait_minutes(&arrival_for_stop, &departure_for_stop);

                let mut so = serde_json::Map::new();
                so.insert("legAfter".into(), serde_json::json!(idx + 1));
                so.insert("arrival".into(), Value::String(arrival_for_stop));
                so.insert("departure".into(), Value::String(departure_for_stop));
                so.insert("waitMinutes".into(), serde_json::json!(wait_minutes));
                so.insert(
                    "requestedMinStopMinutes".into(),
                    serde_json::json!(min_wait),
                );
                if let Some(max) = max_wait {
                    so.insert("requestedMaxStopMinutes".into(), serde_json::json!(max));
                }
                planned_stopovers.push(compact_object(&so));
            }
        }

        let mut all_legs: Vec<Value> = Vec::new();
        let mut all_warnings: Vec<String> = Vec::new();
        let mut all_filter_stats: Vec<Value> = Vec::new();
        let mut segment_summaries: Vec<Value> = Vec::new();

        for result in &segment_results {
            let summary = summarize_ranked_journey(&result.selected, include_stopovers);
            let legs = summary
                .get("legs")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            all_legs.extend(legs);
            all_warnings.extend(result.warnings.clone());
            if let Some(ref stats) = result.filter_stats {
                all_filter_stats.push(stats.clone());
            }

            if mode == ResponseMode::Raw {
                segment_summaries.push(serde_json::json!({
                    "journey": result.selected.journey,
                    "enriched": summary,
                }));
            } else {
                segment_summaries.push(summary);
            }
        }

        let overall_departure = segment_results
            .first()
            .and_then(|r| r.selected.departure.clone());
        let overall_arrival = segment_results
            .last()
            .and_then(|r| r.selected.arrival.clone());
        let overall_duration = match (&overall_departure, &overall_arrival) {
            (Some(dep), Some(arr)) => {
                let dep_dt = chrono::DateTime::parse_from_rfc3339(dep).ok();
                let arr_dt = chrono::DateTime::parse_from_rfc3339(arr).ok();
                match (dep_dt, arr_dt) {
                    (Some(d), Some(a)) => {
                        Some((a.timestamp_millis() - d.timestamp_millis()) / 60_000)
                    }
                    _ => None,
                }
            }
            _ => None,
        };

        let total_transit_legs = all_legs.iter().filter(|l| l.get("line").is_some()).count();
        let total_transfers = total_transit_legs.saturating_sub(1);

        let mut selected_journey = serde_json::Map::new();
        if let Some(ref dep) = overall_departure {
            selected_journey.insert("departure".into(), Value::String(dep.clone()));
        }
        if let Some(ref arr) = overall_arrival {
            selected_journey.insert("arrival".into(), Value::String(arr.clone()));
        }
        if let Some(dur) = overall_duration {
            selected_journey.insert("durationMinutes".into(), serde_json::json!(dur));
        }
        selected_journey.insert("transfers".into(), serde_json::json!(total_transfers));
        selected_journey.insert("legs".into(), Value::Array(all_legs));
        if !planned_stopovers.is_empty() {
            selected_journey.insert("plannedStopovers".into(), Value::Array(planned_stopovers));
        }

        let query_info = serde_json::json!({
            "legs": segment_query,
            "departure": input.departure.as_deref().unwrap_or("now"),
            "excludeOperators": exclude_operators,
            "resolveLocationIds": should_resolve,
            "includeStopovers": include_stopovers,
            "minTransferMinutes": min_transfer_minutes,
        });

        let mut result = serde_json::Map::new();
        result.insert("query".into(), query_info);
        result.insert("selection".into(), serde_json::json!(selection));
        result.insert("selectedJourney".into(), compact_object(&selected_journey));
        result.insert("segments".into(), Value::Array(segment_summaries));
        if !all_filter_stats.is_empty() {
            result.insert("segmentFilterStats".into(), Value::Array(all_filter_stats));
        }
        if !all_warnings.is_empty() {
            result.insert("warnings".into(), serde_json::json!(all_warnings));
        }

        let output = compact_object(&result);
        let mode_str = match mode {
            ResponseMode::Raw => "raw",
            ResponseMode::Summary => "summary",
        };
        if assert_payload_size(&output, mode_str).is_err() {
            let compacted = compact_json(&output, 0);
            if let Err(e2) = assert_payload_size(&compacted, mode_str) {
                return Ok(e2.to_tool_error());
            }
            return Ok(compacted);
        }

        Ok(output)
    }

    // --- Chained (stopover via) planning ---

    /// Plan a single segment (from → to with passthrough vias). Returns the best
    /// ranked journey for the segment, its filter stats, and any warnings.
    async fn plan_segment(
        &self,
        from_id: &str,
        to_id: &str,
        departure: Option<&str>,
        passthrough_vias: &[ViaResolution],
        selection: JourneySelection,
        include_stopovers: bool,
        exclude_operators: &[String],
        results: u32,
    ) -> anyhow::Result<Result<SegmentPlanResult, Value>> {
        let mut query: Vec<(String, String)> = vec![
            ("from".into(), from_id.to_string()),
            ("to".into(), to_id.to_string()),
            ("results".into(), results.to_string()),
        ];
        for via in passthrough_vias {
            query.push((
                "via".into(),
                via.id.clone().unwrap_or_else(|| via.input.clone()),
            ));
        }
        if let Some(dep) = departure {
            query.push(("departure".into(), dep.to_string()));
        } else {
            query.push(("departure".into(), chrono::Utc::now().to_rfc3339()));
        }
        if include_stopovers {
            query.push(("stopovers".into(), "true".into()));
        }

        let strict_via = !passthrough_vias.is_empty();
        let cache_key = format!(
            "oebbPlanJourney:seg:{}:{}:{}",
            from_id,
            to_id,
            serde_json::to_string(&query).unwrap_or_default()
        );

        let payload = self
            .fetch_json("/journeys", &query, &cache_key, Duration::from_secs(25))
            .await?;

        let filtered = filter_journeys(&payload, exclude_operators, strict_via, passthrough_vias);
        let journeys: Vec<Value> = filtered
            .get("journeys")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|j| j.is_object())
            .collect();
        let filter_stats = filtered.get("filterStats").cloned();

        if journeys.is_empty() {
            return Ok(Err(serde_json::json!({
                "error": "no_matching_journeys",
                "message": format!(
                    "No journeys found for segment {} → {}.",
                    from_id, to_id
                ),
                "filterStats": filter_stats,
            })));
        }

        let mut ranked = rank_journeys(&journeys, selection);
        let mut warnings = Vec::new();
        if include_stopovers {
            for rj in ranked.iter_mut() {
                self.enrich_stopovers(rj, &mut warnings).await;
            }
        }

        let selected = ranked.into_iter().next().unwrap();
        Ok(Ok(SegmentPlanResult {
            selected,
            filter_stats,
            warnings,
        }))
    }

    /// Plan a chained journey split at stopover vias.
    #[allow(clippy::too_many_arguments)]
    async fn plan_chained_journey(
        &self,
        from_id: String,
        from_name: Option<String>,
        from_input_str: &str,
        to_id: String,
        to_name: Option<String>,
        to_input_str: &str,
        via_resolved: Vec<ViaResolution>,
        departure: Option<&str>,
        _arrival: Option<&str>,
        selection: JourneySelection,
        include_stopovers: bool,
        _include_alternatives: bool,
        exclude_operators: &[String],
        results: u32,
        mode: ResponseMode,
        min_transfer_minutes: u32,
        should_resolve: bool,
    ) -> anyhow::Result<Value> {
        // Split vias into segments at stopover points.
        let mut segments: Vec<SegmentDef> = Vec::new();
        let mut stopover_defs: Vec<StopoverDef> = Vec::new();
        let mut current_from_id = from_id.clone();
        let mut current_from_name = from_name.clone();
        let mut current_passthrough_vias: Vec<ViaResolution> = Vec::new();

        for via in &via_resolved {
            if via.is_stopover {
                let via_id = via.id.clone().unwrap_or_else(|| via.input.clone());
                let via_name = via.name.clone();
                segments.push(SegmentDef {
                    from_id: current_from_id.clone(),
                    from_name: current_from_name.clone(),
                    to_id: via_id.clone(),
                    to_name: via_name.clone(),
                    passthrough_vias: std::mem::take(&mut current_passthrough_vias),
                });
                let effective_min =
                    std::cmp::max(via.min_stop_minutes.unwrap_or(0), min_transfer_minutes);
                stopover_defs.push(StopoverDef {
                    station_id: via_id.clone(),
                    station_name: via_name.clone(),
                    min_stop_minutes: effective_min,
                    max_stop_minutes: via.max_stop_minutes,
                });
                current_from_id = via_id;
                current_from_name = via_name;
            } else {
                current_passthrough_vias.push(via.clone());
            }
        }
        // Final segment: last stopover (or origin) → destination.
        segments.push(SegmentDef {
            from_id: current_from_id,
            from_name: current_from_name,
            to_id: to_id.clone(),
            to_name: to_name.clone(),
            passthrough_vias: current_passthrough_vias,
        });

        // Plan each segment sequentially.
        let mut segment_results: Vec<SegmentPlanResult> = Vec::new();
        for (i, seg) in segments.iter().enumerate() {
            let seg_departure: Option<String> = if i == 0 {
                departure.map(|s| s.to_string())
            } else {
                // Compute departure from previous segment's arrival + min_stop_minutes.
                let prev = &segment_results[i - 1];
                let stopover = &stopover_defs[i - 1];
                if let Some(ref arr) = prev.selected.arrival {
                    if let Ok(arr_dt) = chrono::DateTime::parse_from_rfc3339(arr) {
                        let next_dep =
                            arr_dt + chrono::Duration::minutes(stopover.min_stop_minutes as i64);
                        Some(next_dep.to_rfc3339())
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            let result = self
                .plan_segment(
                    &seg.from_id,
                    &seg.to_id,
                    seg_departure.as_deref(),
                    &seg.passthrough_vias,
                    selection,
                    include_stopovers,
                    exclude_operators,
                    results,
                )
                .await?;

            match result {
                Ok(r) => segment_results.push(r),
                Err(err_val) => return Ok(err_val),
            }
        }

        // Combine segment results into a unified response.
        let mut all_legs: Vec<Value> = Vec::new();
        let mut all_warnings: Vec<String> = Vec::new();
        let mut all_filter_stats: Vec<Value> = Vec::new();

        for result in &segment_results {
            let summary = summarize_ranked_journey(&result.selected, include_stopovers);
            let legs = summary
                .get("legs")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            all_legs.extend(legs);
            all_warnings.extend(result.warnings.clone());
            if let Some(ref stats) = result.filter_stats {
                all_filter_stats.push(stats.clone());
            }
        }

        // Overall departure and arrival.
        let overall_departure = segment_results
            .first()
            .and_then(|r| r.selected.departure.clone());
        let overall_arrival = segment_results
            .last()
            .and_then(|r| r.selected.arrival.clone());

        // Overall duration.
        let overall_duration = match (&overall_departure, &overall_arrival) {
            (Some(dep), Some(arr)) => {
                let dep_dt = chrono::DateTime::parse_from_rfc3339(dep).ok();
                let arr_dt = chrono::DateTime::parse_from_rfc3339(arr).ok();
                match (dep_dt, arr_dt) {
                    (Some(d), Some(a)) => {
                        let dur = (a.timestamp_millis() - d.timestamp_millis()) / 60_000;
                        Some(dur)
                    }
                    _ => None,
                }
            }
            _ => None,
        };

        // Total transfers = total transit legs - 1.
        let total_transit_legs = all_legs.iter().filter(|l| l.get("line").is_some()).count();
        let total_transfers = total_transit_legs.saturating_sub(1);

        // Build planned stopovers.
        let mut planned_stopovers: Vec<Value> = Vec::new();
        for i in 0..segment_results.len().saturating_sub(1) {
            let prev = &segment_results[i];
            let next = &segment_results[i + 1];
            let stopover = &stopover_defs[i];

            let arrival_str = prev.selected.arrival.clone().unwrap_or_default();
            let departure_str = next.selected.departure.clone().unwrap_or_default();

            let wait_minutes = compute_wait_minutes(&arrival_str, &departure_str);

            let mut so = serde_json::Map::new();
            let mut station = serde_json::Map::new();
            station.insert("id".into(), Value::String(stopover.station_id.clone()));
            if let Some(ref name) = stopover.station_name {
                station.insert("name".into(), Value::String(name.clone()));
            }
            so.insert("station".into(), compact_object(&station));
            so.insert("arrival".into(), Value::String(arrival_str));
            so.insert("departure".into(), Value::String(departure_str));
            so.insert("waitMinutes".into(), serde_json::json!(wait_minutes));
            so.insert(
                "requestedMinStopMinutes".into(),
                serde_json::json!(stopover.min_stop_minutes),
            );
            if let Some(max) = stopover.max_stop_minutes {
                so.insert("requestedMaxStopMinutes".into(), serde_json::json!(max));
                if wait_minutes > max as i64 {
                    all_warnings.push(format!(
                        "Stopover at {} is {}min, exceeding requested maximum of {}min.",
                        stopover
                            .station_name
                            .as_deref()
                            .unwrap_or(&stopover.station_id),
                        wait_minutes,
                        max
                    ));
                }
            }
            planned_stopovers.push(compact_object(&so));
        }

        // Build combined selected journey.
        let mut selected_journey = serde_json::Map::new();
        if let Some(ref dep) = overall_departure {
            selected_journey.insert("departure".into(), Value::String(dep.clone()));
        }
        if let Some(ref arr) = overall_arrival {
            selected_journey.insert("arrival".into(), Value::String(arr.clone()));
        }
        if let Some(dur) = overall_duration {
            selected_journey.insert("durationMinutes".into(), serde_json::json!(dur));
        }
        selected_journey.insert("transfers".into(), serde_json::json!(total_transfers));
        selected_journey.insert("legs".into(), Value::Array(all_legs));
        if !planned_stopovers.is_empty() {
            selected_journey.insert("plannedStopovers".into(), Value::Array(planned_stopovers));
        }

        // Build query info.
        let query_info = serde_json::json!({
            "from": { "input": from_input_str, "resolvedId": from_id, "resolvedName": from_name },
            "to": { "input": to_input_str, "resolvedId": to_id, "resolvedName": to_name },
            "via": via_resolved.iter().map(|v| {
                let mut m = serde_json::Map::new();
                m.insert("input".into(), Value::String(v.input.clone()));
                if let Some(ref id) = v.id {
                    m.insert("resolvedId".into(), Value::String(id.clone()));
                }
                if let Some(ref name) = v.name {
                    m.insert("resolvedName".into(), Value::String(name.clone()));
                }
                if v.is_stopover {
                    m.insert("stopType".into(), Value::String("stopover".into()));
                }
                if let Some(min) = v.min_stop_minutes {
                    m.insert("minStopMinutes".into(), serde_json::json!(min));
                }
                if let Some(max) = v.max_stop_minutes {
                    m.insert("maxStopMinutes".into(), serde_json::json!(max));
                }
                compact_object(&m)
            }).collect::<Vec<_>>(),
            "departure": departure.or_else(|| if _arrival.is_none() { Some("now") } else { None }),
            "arrival": _arrival,
            "excludeOperators": exclude_operators,
            "resolveLocationIds": should_resolve,
            "includeStopovers": include_stopovers,
        });

        // Build response.
        let mode_str = match mode {
            ResponseMode::Raw => "raw",
            ResponseMode::Summary => "summary",
        };

        let mut result = serde_json::Map::new();
        result.insert("query".into(), query_info);
        result.insert("selection".into(), serde_json::json!(selection));
        result.insert("selectedJourney".into(), compact_object(&selected_journey));
        if !all_filter_stats.is_empty() {
            result.insert("segmentFilterStats".into(), Value::Array(all_filter_stats));
        }
        if !all_warnings.is_empty() {
            result.insert("warnings".into(), serde_json::json!(all_warnings));
        }

        let output = compact_object(&result);

        if assert_payload_size(&output, mode_str).is_err() {
            let compacted = compact_json(&output, 0);
            if let Err(e2) = assert_payload_size(&compacted, mode_str) {
                return Ok(e2.to_tool_error());
            }
            return Ok(compacted);
        }

        Ok(output)
    }

    // --- Low-level OEBB API methods (Gap 6) ---

    /// Search for locations by name.
    pub async fn locations(
        &self,
        query_str: &str,
        results: Option<u32>,
        stops: Option<bool>,
        addresses: Option<bool>,
        poi: Option<bool>,
    ) -> anyhow::Result<Value> {
        let mut query = vec![
            ("query".to_string(), query_str.to_string()),
            ("results".to_string(), results.unwrap_or(6).to_string()),
        ];
        if let Some(s) = stops {
            query.push(("stops".into(), s.to_string()));
        }
        if let Some(a) = addresses {
            query.push(("addresses".into(), a.to_string()));
        }
        if let Some(p) = poi {
            query.push(("poi".into(), p.to_string()));
        }

        let cache_key = format!(
            "oebbLocations:{}:{}:{:?}:{:?}:{:?}",
            query_str,
            results.unwrap_or(6),
            stops,
            addresses,
            poi
        );
        self.fetch_json("/locations", &query, &cache_key, Duration::from_secs(60))
            .await
    }

    /// Get departures for a stop.
    pub async fn departures(
        &self,
        stop_id: &str,
        duration: Option<u32>,
        when: Option<&str>,
        results: Option<u32>,
        stopovers: Option<bool>,
        remarks: Option<bool>,
    ) -> anyhow::Result<Value> {
        let mut query = vec![
            ("duration".to_string(), duration.unwrap_or(20).to_string()),
            (
                "when".to_string(),
                when.unwrap_or(&chrono::Utc::now().to_rfc3339()).to_string(),
            ),
        ];
        if let Some(r) = results {
            query.push(("results".into(), r.to_string()));
        }
        if let Some(s) = stopovers {
            query.push(("stopovers".into(), s.to_string()));
        }
        if let Some(r) = remarks {
            query.push(("remarks".into(), r.to_string()));
        }

        let path = format!("/stops/{}/departures", urlencoding::encode(stop_id));
        let cache_key = format!(
            "oebbDepartures:{}:{:?}:{:?}:{:?}",
            stop_id, duration, when, results
        );
        self.fetch_json(&path, &query, &cache_key, Duration::from_secs(15))
            .await
    }

    /// Fetch raw journeys (without ranking/filtering).
    pub async fn journeys_raw(
        &self,
        from: &str,
        to: &str,
        query_params: &[(String, String)],
    ) -> anyhow::Result<Value> {
        let cache_key = format!(
            "oebbJourneys:{}:{}:{}",
            from,
            to,
            serde_json::to_string(query_params).unwrap_or_default()
        );
        self.fetch_json(
            "/journeys",
            query_params,
            &cache_key,
            Duration::from_secs(25),
        )
        .await
    }

    /// Fetch a trip by ID.
    pub async fn trip(
        &self,
        trip_id: &str,
        stopovers: Option<bool>,
        remarks: Option<bool>,
    ) -> anyhow::Result<Value> {
        let mut query: Vec<(String, String)> = Vec::new();
        if let Some(s) = stopovers {
            query.push(("stopovers".into(), s.to_string()));
        }
        if let Some(r) = remarks {
            query.push(("remarks".into(), r.to_string()));
        }

        let path = format!("/trips/{}", urlencoding::encode(trip_id));
        let cache_key = format!("oebbTrip:{}:{:?}:{:?}", trip_id, stopovers, remarks);
        self.fetch_json(&path, &query, &cache_key, Duration::from_secs(30))
            .await
    }

    /// Enrich a ranked journey with intermediate stopovers.
    async fn enrich_stopovers(&self, journey: &mut RankedJourney, warnings: &mut Vec<String>) {
        for (index, leg) in journey.legs.iter().enumerate() {
            if leg.get("walking") == Some(&Value::Bool(true)) {
                continue;
            }

            // Try direct stops first.
            let direct = extract_intermediate_stops_from_leg(leg);
            if !direct.is_empty() {
                journey.enriched_intermediate_stops[index] = direct;
                continue;
            }

            // Fallback: fetch trip stopovers.
            let trip_id = match leg.get("tripId").and_then(|v| v.as_str()) {
                Some(id) if !id.is_empty() => id,
                _ => continue,
            };

            let query = vec![("stopovers".into(), "true".into())];
            let path = format!("/trips/{}", urlencoding::encode(trip_id));
            let cache_key = format!("oebbPlanJourney:trip:{}", trip_id);

            match self
                .fetch_json(&path, &query, &cache_key, Duration::from_secs(30))
                .await
            {
                Ok(trip_payload) => {
                    let stops = extract_trip_intermediate_stops(
                        &trip_payload,
                        leg.get("origin"),
                        leg.get("destination"),
                    );
                    if !stops.is_empty() {
                        journey.enriched_intermediate_stops[index] = stops;
                    }
                    warnings.push(format!(
                        "Used trip fallback stopovers for leg {} ({}).",
                        index + 1,
                        trip_id
                    ));
                }
                Err(e) => {
                    warnings.push(format!(
                        "Could not enrich stopovers for leg {} ({}): {}.",
                        index + 1,
                        trip_id,
                        e
                    ));
                }
            }
        }
    }
}

fn normalize_oebb_endpoint(path: &str) -> &str {
    if path.starts_with("/trips/") {
        "/trips/:id"
    } else {
        path
    }
}

// --- Helper functions ---

fn extract_location_items(payload: &Value) -> Vec<&Value> {
    if let Some(arr) = payload.as_array() {
        return arr.iter().filter(|v| v.is_object()).collect();
    }
    if let Some(obj) = payload.as_object() {
        if let Some(locations) = obj.get("locations").and_then(|v| v.as_array()) {
            return locations.iter().filter(|v| v.is_object()).collect();
        }
    }
    vec![]
}

fn score_location_candidate(
    query: &str,
    candidate_name: &str,
    candidate_id: &str,
    candidate_type: Option<&str>,
) -> i32 {
    let nq = normalize_comparable_text(query);
    let nc = normalize_comparable_text(candidate_name);
    if nq.is_empty() || nc.is_empty() {
        return 0;
    }
    let mut score = 0i32;
    if nc == nq {
        score += 100;
    } else if nc.starts_with(&nq) {
        score += 70;
    } else if nc.contains(&nq) {
        score += 50;
    }
    if candidate_type == Some("stop") {
        score += 5;
    }
    // Prefer main station variants (Bahnhof/Hbf) when the query is a bare city name
    // (i.e. query doesn't already contain "bahnhof", "hbf", etc.).
    let nc_lower = candidate_name.to_lowercase();
    let nq_lower = query.to_lowercase();
    let query_has_station_keyword =
        nq_lower.contains("bahnhof") || nq_lower.contains("hbf") || nq_lower.contains("station");
    if !query_has_station_keyword {
        if nc_lower.contains("bahnhof") || nc_lower.contains(" hbf") {
            score += 3;
        }
    }
    // Avoid selecting metro/U-Bahn style pseudo stops when user asks for rail stations.
    if candidate_name.contains("(U)") {
        score -= 20;
    }
    // Prefer probable rail-network stop IDs over synthetic IDs (often 12* or 13* in local/meta nodes).
    if candidate_id.starts_with("81") || candidate_id.starts_with("80") {
        score += 4;
    }
    score
}

fn extract_trip_intermediate_stops(
    payload: &Value,
    origin: Option<&Value>,
    destination: Option<&Value>,
) -> Vec<Value> {
    let stopovers = extract_trip_stopovers(payload);
    if stopovers.is_empty() {
        return vec![];
    }

    let origin_summary = origin.and_then(summarize_place);
    let dest_summary = destination.and_then(summarize_place);
    let origin_key = origin_summary.as_ref().and_then(place_key);
    let dest_key = dest_summary.as_ref().and_then(place_key);

    let stops: Vec<Value> = stopovers
        .iter()
        .filter_map(|stopover| {
            let stop_val = stopover
                .get("stop")
                .filter(|v| v.is_object())
                .unwrap_or(stopover);
            summarize_place(stop_val)
        })
        .collect();

    if stops.is_empty() {
        return vec![];
    }

    if let (Some(ref ok), Some(ref dk)) = (origin_key, dest_key) {
        let keys: Vec<Option<String>> = stops.iter().map(place_key).collect();
        let start_idx = keys.iter().position(|k| k.as_deref() == Some(ok));
        if let Some(si) = start_idx {
            let end_idx = keys[si + 1..]
                .iter()
                .position(|k| k.as_deref() == Some(dk))
                .map(|i| i + si + 1);
            if let Some(ei) = end_idx {
                return stops[si + 1..ei].to_vec();
            }
            return stops[si + 1..].to_vec();
        }
        return stops
            .into_iter()
            .filter(|s| place_key(s).as_deref() != Some(dk))
            .collect();
    }

    stops
}

fn extract_trip_stopovers(payload: &Value) -> Vec<&Value> {
    if let Some(arr) = payload.get("stopovers").and_then(|v| v.as_array()) {
        return arr.iter().filter(|v| v.is_object()).collect();
    }
    if let Some(trip) = payload.get("trip") {
        if let Some(arr) = trip.get("stopovers").and_then(|v| v.as_array()) {
            return arr.iter().filter(|v| v.is_object()).collect();
        }
    }
    vec![]
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
        return Some(format!("name:{}", normalize_comparable_text(name)));
    }
    None
}

fn build_resolution_error(leg: usize, field: &str, input: &str, err: &Value) -> Value {
    let code = err
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("location_lookup_failed");
    let message = err
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("Location lookup failed.");

    let mut out = serde_json::Map::new();
    out.insert("leg".into(), serde_json::json!(leg));
    out.insert("field".into(), Value::String(field.to_string()));
    out.insert("input".into(), Value::String(input.to_string()));
    out.insert("error".into(), Value::String(code.to_string()));
    out.insert("message".into(), Value::String(message.to_string()));
    if let Some(query) = err.get("query") {
        out.insert("query".into(), query.clone());
    }
    if let Some(candidates) = err.get("candidates") {
        out.insert("candidates".into(), candidates.clone());
    }
    Value::Object(out)
}

fn normalize_operator_alias(value: &str) -> String {
    let normalized = value.trim().to_lowercase();
    if normalized.is_empty() {
        return normalized;
    }

    if normalized.contains("westbahn") || normalized == "wb" {
        return "westbahn-management-gmbh".to_string();
    }

    normalized
}

fn compute_retry_delay_ms(attempt: u32, base_delay_ms: u64, max_delay_ms: u64) -> u64 {
    let exp = attempt.saturating_sub(1).min(16);
    let unjittered = base_delay_ms
        .saturating_mul(1u64 << exp)
        .min(max_delay_ms.max(base_delay_ms));
    let jitter_percent = pseudo_jitter_percent();
    unjittered.saturating_mul(jitter_percent) / 100
}

fn pseudo_jitter_percent() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    70 + (nanos % 61)
}

async fn sleep_within_retry_budget(started: Instant, retry_budget_ms: u64, delay_ms: u64) -> bool {
    let elapsed_ms = started.elapsed().as_millis() as u64;
    if elapsed_ms >= retry_budget_ms {
        return false;
    }
    let remaining_ms = retry_budget_ms - elapsed_ms;
    tokio::time::sleep(Duration::from_millis(delay_ms.min(remaining_ms))).await;
    true
}

// --- Chained planning internal types ---

/// A journey segment between two stops (with optional passthrough vias).
#[allow(dead_code)]
struct SegmentDef {
    from_id: String,
    from_name: Option<String>,
    to_id: String,
    to_name: Option<String>,
    passthrough_vias: Vec<ViaResolution>,
}

/// Definition of a planned stopover between two segments.
struct StopoverDef {
    station_id: String,
    station_name: Option<String>,
    min_stop_minutes: u32,
    max_stop_minutes: Option<u32>,
}

/// Result of planning a single segment.
struct SegmentPlanResult {
    selected: RankedJourney,
    filter_stats: Option<Value>,
    warnings: Vec<String>,
}

/// Compute wait time in minutes between two RFC 3339 timestamps.
fn compute_wait_minutes(arrival: &str, departure: &str) -> i64 {
    let arr = chrono::DateTime::parse_from_rfc3339(arrival).ok();
    let dep = chrono::DateTime::parse_from_rfc3339(departure).ok();
    match (arr, dep) {
        (Some(a), Some(d)) => (d.timestamp_millis() - a.timestamp_millis()) / 60_000,
        _ => 0,
    }
}
