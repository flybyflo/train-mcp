use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::auth::AuthenticatedUser;

#[derive(Clone)]
pub struct Persistence {
    pool: PgPool,
    history_max_limit: i64,
}

#[derive(Debug, Clone)]
pub struct QueryLogInput {
    pub source: String,
    pub operation: String,
    pub request_payload: Value,
    pub response_payload: Option<Value>,
    pub succeeded: bool,
    pub duration_ms: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct QueryHistoryItem {
    pub id: i64,
    pub source: String,
    pub operation: String,
    pub request_payload: Value,
    pub response_payload: Option<Value>,
    pub succeeded: bool,
    pub duration_ms: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct StationSearchItem {
    pub station_uuid: String,
    pub name: String,
    pub normalized_name: String,
    pub station_id: String,
    pub europa_id: Option<i64>,
    pub gtfs_parent_stop_id: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
}

impl Persistence {
    pub async fn from_env() -> anyhow::Result<Option<Self>> {
        let database_url = std::env::var("DATABASE_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let Some(database_url) = database_url else {
            return Ok(None);
        };

        let max_connections = u32_from_env("DB_MAX_CONNECTIONS", 10).max(1);
        let acquire_timeout_seconds = u64_from_env("DB_ACQUIRE_TIMEOUT_SECONDS", 5).max(1);
        let history_max_limit = i64_from_env("HISTORY_MAX_LIMIT", 100).max(1);

        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(std::time::Duration::from_secs(acquire_timeout_seconds))
            .connect(&database_url)
            .await?;

        let persistence = Self {
            pool,
            history_max_limit,
        };
        persistence.init_schema().await?;
        Ok(Some(persistence))
    }

    async fn init_schema(&self) -> anyhow::Result<()> {
        sqlx::raw_sql(include_str!("../sql/001_app_schema.sql"))
            .execute(&self.pool)
            .await?;
        sqlx::raw_sql(include_str!("../sql/002_transit_schema.sql"))
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn log_call(
        &self,
        user: &AuthenticatedUser,
        input: QueryLogInput,
    ) -> anyhow::Result<()> {
        self.upsert_user(user).await?;
        self.upsert_session(user).await?;

        let duration_ms = input.duration_ms.clamp(0, i64::from(i32::MAX)) as i32;
        sqlx::query(
            r#"
            INSERT INTO app.query_log (
                user_id,
                session_key,
                source,
                operation,
                request_payload,
                response_payload,
                succeeded,
                duration_ms
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8);
            "#,
        )
        .bind(&user.user_id)
        .bind(&user.session_key)
        .bind(&input.source)
        .bind(&input.operation)
        .bind(&input.request_payload)
        .bind(&input.response_payload)
        .bind(input.succeeded)
        .bind(duration_ms)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn recent_queries(
        &self,
        user_id: &str,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> anyhow::Result<Vec<QueryHistoryItem>> {
        let (limit, offset) = self.clamp_pagination(limit, offset);
        let items = sqlx::query_as::<_, QueryHistoryItem>(
            r#"
            SELECT
                id,
                source,
                operation,
                request_payload,
                response_payload,
                succeeded,
                duration_ms,
                created_at
            FROM app.query_log
            WHERE user_id = $1
            ORDER BY created_at DESC
            LIMIT $2 OFFSET $3;
            "#,
        )
        .bind(user_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(items)
    }

    pub async fn recent_journeys(
        &self,
        user_id: &str,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> anyhow::Result<Vec<QueryHistoryItem>> {
        let (limit, offset) = self.clamp_pagination(limit, offset);
        let items = sqlx::query_as::<_, QueryHistoryItem>(
            r#"
            SELECT
                id,
                source,
                operation,
                request_payload,
                response_payload,
                succeeded,
                duration_ms,
                created_at
            FROM app.query_log
            WHERE user_id = $1
              AND operation IN ('oebbPlanJourney', 'oebbPlanTour', 'oebbJourneys')
            ORDER BY created_at DESC
            LIMIT $2 OFFSET $3;
            "#,
        )
        .bind(user_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(items)
    }

    pub async fn search_stations(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> anyhow::Result<Vec<StationSearchItem>> {
        let requested = i64::from(limit.unwrap_or(10)).clamp(1, 50);
        let items = sqlx::query_as::<_, StationSearchItem>(
            r#"
            WITH normalized AS (
                SELECT transit.normalize_search_text($1) AS q
            )
            SELECT
                s.station_uuid::text AS station_uuid,
                s.canonical_name AS name,
                s.canonical_name_norm AS normalized_name,
                COALESCE(s.europa_id::text, s.resolver_ref_value) AS station_id,
                s.europa_id,
                s.gtfs_parent_stop_id,
                s.lat::double precision AS lat,
                s.lon::double precision AS lon
            FROM transit.station s
            CROSS JOIN normalized n
            LEFT JOIN transit.station_name sn
              ON sn.station_uuid = s.station_uuid
            WHERE
                s.canonical_name_norm %% n.q
                OR s.canonical_name_norm LIKE '%' || n.q || '%'
                OR EXISTS (
                    SELECT 1
                    FROM transit.station_name sn2
                    WHERE sn2.station_uuid = s.station_uuid
                      AND (
                        sn2.name_norm %% n.q
                        OR sn2.name_norm LIKE '%' || n.q || '%'
                      )
                )
            GROUP BY s.station_uuid, s.canonical_name, s.canonical_name_norm, s.europa_id, s.resolver_ref_value, s.gtfs_parent_stop_id, s.lat, s.lon, n.q
            ORDER BY greatest(
                similarity(s.canonical_name_norm, n.q),
                coalesce(max(similarity(sn.name_norm, n.q)), 0)
            ) DESC, s.canonical_name
            LIMIT $2
            "#,
        )
        .bind(query)
        .bind(requested)
        .fetch_all(&self.pool)
        .await?;
        Ok(items)
    }

    pub async fn resolve_station_id(
        &self,
        value: &str,
    ) -> anyhow::Result<Option<StationSearchItem>> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        let exact = sqlx::query_as::<_, StationSearchItem>(
            r#"
            WITH normalized AS (
                SELECT transit.normalize_search_text($1) AS q
            )
            SELECT
                s.station_uuid::text AS station_uuid,
                s.canonical_name AS name,
                s.canonical_name_norm AS normalized_name,
                COALESCE(s.europa_id::text, s.resolver_ref_value) AS station_id,
                s.europa_id,
                s.gtfs_parent_stop_id,
                s.lat::double precision AS lat,
                s.lon::double precision AS lon
            FROM transit.station s
            CROSS JOIN normalized n
            WHERE
                s.gtfs_parent_stop_id = $1
                OR s.gtfs_core_stop_id = $1
                OR s.europa_id::text = $1
                OR s.resolver_ref_value = $1
                OR s.canonical_name_norm = n.q
                OR EXISTS (
                    SELECT 1
                    FROM transit.station_name sn
                    WHERE sn.station_uuid = s.station_uuid
                      AND sn.name_norm = n.q
                )
            ORDER BY CASE WHEN s.europa_id IS NOT NULL THEN 0 ELSE 1 END, s.canonical_name
            LIMIT 1
            "#,
        )
        .bind(trimmed)
        .fetch_optional(&self.pool)
        .await?;
        if exact.is_some() {
            return Ok(exact);
        }

        let fallback = self.search_stations(trimmed, Some(1)).await?;
        Ok(fallback.into_iter().next())
    }

    fn clamp_pagination(&self, limit: Option<u32>, offset: Option<u32>) -> (i64, i64) {
        let requested = i64::from(limit.unwrap_or(20)).max(1);
        let limit = requested.min(self.history_max_limit);
        let offset = i64::from(offset.unwrap_or(0)).max(0);
        (limit, offset)
    }

    async fn upsert_user(&self, user: &AuthenticatedUser) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO app.user_identity (user_id, sub, iss, email, name)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (user_id)
            DO UPDATE SET
                iss = EXCLUDED.iss,
                email = EXCLUDED.email,
                name = EXCLUDED.name,
                updated_at = NOW();
            "#,
        )
        .bind(&user.user_id)
        .bind(&user.sub)
        .bind(&user.iss)
        .bind(&user.email)
        .bind(&user.name)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn upsert_session(&self, user: &AuthenticatedUser) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO app.user_session (session_key, user_id, jti, issuer)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (session_key)
            DO UPDATE SET
                user_id = EXCLUDED.user_id,
                jti = EXCLUDED.jti,
                issuer = EXCLUDED.issuer,
                last_seen_at = NOW();
            "#,
        )
        .bind(&user.session_key)
        .bind(&user.user_id)
        .bind(&user.jti)
        .bind(&user.iss)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

fn u32_from_env(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(default)
}

fn u64_from_env(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}

fn i64_from_env(name: &str, default: i64) -> i64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(default)
}
