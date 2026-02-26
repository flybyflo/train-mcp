use async_trait::async_trait;
use serde_json::Value;

use crate::oebb::types::{
    LocationResolution, PlanJourneyInput, PlanTourInput, ResolveItineraryStopsInput,
};

#[async_trait]
pub trait TransitProvider: Send + Sync {
    fn provider_name(&self) -> &'static str;

    async fn resolve_location(
        &self,
        tool_name: &str,
        field: &str,
        value: &str,
        resolve: bool,
    ) -> LocationResolution;

    async fn resolve_itinerary_stops(
        &self,
        input: ResolveItineraryStopsInput,
    ) -> anyhow::Result<Value>;

    async fn plan_journey(&self, input: PlanJourneyInput) -> anyhow::Result<Value>;

    async fn plan_tour(&self, input: PlanTourInput) -> anyhow::Result<Value>;

    async fn locations(
        &self,
        query: &str,
        results: Option<u32>,
        stops: Option<bool>,
        addresses: Option<bool>,
        poi: Option<bool>,
    ) -> anyhow::Result<Value>;

    async fn departures(
        &self,
        stop_id: &str,
        duration: Option<u32>,
        when: Option<&str>,
        results: Option<u32>,
        stopovers: Option<bool>,
        remarks: Option<bool>,
    ) -> anyhow::Result<Value>;

    async fn journeys_raw(
        &self,
        from: &str,
        to: &str,
        query_params: &[(String, String)],
    ) -> anyhow::Result<Value>;

    async fn trip(
        &self,
        trip_id: &str,
        stopovers: Option<bool>,
        remarks: Option<bool>,
    ) -> anyhow::Result<Value>;
}
