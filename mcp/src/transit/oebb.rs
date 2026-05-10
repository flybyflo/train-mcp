use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

use crate::oebb::client::OebbClient;
use crate::oebb::types::{
    LocationResolution, PlanJourneyInput, PlanTourInput, ResolveItineraryStopsInput,
};
use crate::persistence::Persistence;

use super::provider::TransitProvider;

#[derive(Clone)]
pub struct OebbTransitProvider {
    client: OebbClient,
    persistence: Option<Arc<Persistence>>,
}

impl OebbTransitProvider {
    pub fn new(base_url: String, persistence: Option<Arc<Persistence>>) -> Self {
        Self {
            client: OebbClient::new(base_url),
            persistence,
        }
    }

    pub fn from_client(client: OebbClient, persistence: Option<Arc<Persistence>>) -> Self {
        Self {
            client,
            persistence,
        }
    }
}

#[async_trait]
impl TransitProvider for OebbTransitProvider {
    fn provider_name(&self) -> &'static str {
        "oebb"
    }

    async fn resolve_location(
        &self,
        tool_name: &str,
        field: &str,
        value: &str,
        resolve: bool,
    ) -> LocationResolution {
        let trimmed = value.trim();
        if resolve && !trimmed.is_empty() && !crate::oebb::normalize::looks_like_stop_id(trimmed) {
            if let Some(persistence) = &self.persistence {
                match persistence.resolve_station_id(trimmed).await {
                    Ok(Some(station)) => {
                        return LocationResolution::Ok {
                            id: station.station_id,
                            name: Some(station.name),
                            resolved: true,
                        };
                    }
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(
                            "station DB resolution failed for {field}={trimmed}: {error}"
                        );
                    }
                }
            }
        }

        self.client
            .resolve_location(tool_name, field, value, resolve)
            .await
    }

    async fn resolve_itinerary_stops(
        &self,
        input: ResolveItineraryStopsInput,
    ) -> anyhow::Result<Value> {
        self.client.resolve_itinerary_stops(input).await
    }

    async fn plan_journey(&self, input: PlanJourneyInput) -> anyhow::Result<Value> {
        self.client.plan_journey(input).await
    }

    async fn plan_tour(&self, input: PlanTourInput) -> anyhow::Result<Value> {
        self.client.plan_tour(input).await
    }

    async fn locations(
        &self,
        query: &str,
        results: Option<u32>,
        stops: Option<bool>,
        addresses: Option<bool>,
        poi: Option<bool>,
    ) -> anyhow::Result<Value> {
        self.client
            .locations(query, results, stops, addresses, poi)
            .await
    }

    async fn departures(
        &self,
        stop_id: &str,
        duration: Option<u32>,
        when: Option<&str>,
        results: Option<u32>,
        stopovers: Option<bool>,
        remarks: Option<bool>,
    ) -> anyhow::Result<Value> {
        self.client
            .departures(stop_id, duration, when, results, stopovers, remarks)
            .await
    }

    async fn journeys_raw(
        &self,
        from: &str,
        to: &str,
        query_params: &[(String, String)],
    ) -> anyhow::Result<Value> {
        self.client.journeys_raw(from, to, query_params).await
    }

    async fn trip(
        &self,
        trip_id: &str,
        stopovers: Option<bool>,
        remarks: Option<bool>,
    ) -> anyhow::Result<Value> {
        self.client.trip(trip_id, stopovers, remarks).await
    }
}
