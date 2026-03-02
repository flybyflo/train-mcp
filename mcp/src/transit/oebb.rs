use async_trait::async_trait;
use serde_json::Value;

use crate::oebb::client::OebbClient;
use crate::oebb::types::{
    LocationResolution, PlanJourneyInput, PlanTourInput, ResolveItineraryStopsInput,
};

use super::provider::TransitProvider;

#[derive(Clone)]
pub struct OebbTransitProvider {
    client: OebbClient,
}

impl OebbTransitProvider {
    pub fn new(base_url: String) -> Self {
        Self {
            client: OebbClient::new(base_url),
        }
    }

    pub fn from_client(client: OebbClient) -> Self {
        Self { client }
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
