CREATE EXTENSION IF NOT EXISTS pg_trgm;
CREATE EXTENSION IF NOT EXISTS unaccent;

CREATE SCHEMA IF NOT EXISTS gtfs_stage;
CREATE SCHEMA IF NOT EXISTS transit;

CREATE OR REPLACE FUNCTION transit.normalize_search_text(input text)
RETURNS text
LANGUAGE sql
IMMUTABLE
AS $$
    SELECT trim(
        regexp_replace(
            regexp_replace(
                replace(
                    replace(
                        replace(
                            replace(
                                lower(coalesce(input, '')),
                                'ä', 'ae'
                            ),
                            'ö', 'oe'
                        ),
                        'ü', 'ue'
                    ),
                    'ß', 'ss'
                ),
                '[^[:alnum:]_]+',
                ' ',
                'g'
            ),
            '\s+',
            ' ',
            'g'
        )
    );
$$;

-- Import raw files exactly as delivered, then transform into transit.*.
CREATE TABLE IF NOT EXISTS gtfs_stage.stops_raw (
    row_id bigserial PRIMARY KEY,
    stop_id text NOT NULL,
    stop_name text NOT NULL,
    stop_lat numeric(10, 8),
    stop_lon numeric(11, 8),
    zone_id text,
    location_type text,
    parent_station text,
    level_id text,
    platform_code text,
    source_file text NOT NULL DEFAULT 'stops.txt'
);

CREATE TABLE IF NOT EXISTS gtfs_stage.europa_raw (
    row_id bigserial PRIMARY KEY,
    europa_id bigint NOT NULL,
    name_primary text NOT NULL,
    name_alt_1 text,
    name_alt_2 text,
    name_alt_3 text,
    name_alt_4 text,
    name_alt_5 text,
    source_encoding text NOT NULL DEFAULT 'cp1252',
    source_file text NOT NULL DEFAULT 'europa.csv',
    CONSTRAINT europa_raw_europa_id_unique UNIQUE (europa_id)
);

CREATE TABLE IF NOT EXISTS gtfs_stage.hafas_mapping_raw (
    row_id bigserial PRIMARY KEY,
    ifopt_id text NOT NULL,
    ibnr bigint,
    rating numeric(6, 5),
    delfi_name text,
    db_name text,
    source_file text NOT NULL DEFAULT 'hafas-delfi-gtfs-stations-mapping.csv'
);

CREATE TABLE IF NOT EXISTS gtfs_stage.netex_stop_place_raw (
    row_id bigserial PRIMARY KEY,
    netex_stop_place_id text NOT NULL,
    version text,
    name text,
    short_name text,
    stop_place_type text,
    transport_mode text,
    private_code text,
    road_name text,
    mobility_impaired_access boolean,
    wheelchair_access boolean,
    from_date timestamptz,
    to_date timestamptz,
    assistance_facility text,
    assistance_availability text,
    passenger_comms_facility text,
    ticketing_facility_list text,
    access_facility_list text,
    hire_facility text,
    raw_xml xml,
    source_file text NOT NULL DEFAULT 'NETEX_OEBB_SITEFRAME_2025.xml',
    CONSTRAINT netex_stop_place_raw_unique UNIQUE (netex_stop_place_id)
);

CREATE TABLE IF NOT EXISTS gtfs_stage.netex_quay_raw (
    row_id bigserial PRIMARY KEY,
    netex_stop_place_id text NOT NULL,
    netex_quay_id text NOT NULL,
    quay_type text,
    label text,
    mobility_impaired_access boolean,
    raw_xml xml,
    source_file text NOT NULL DEFAULT 'NETEX_OEBB_SITEFRAME_2025.xml',
    CONSTRAINT netex_quay_raw_unique UNIQUE (netex_quay_id)
);

CREATE TABLE IF NOT EXISTS transit.station (
    station_uuid uuid PRIMARY KEY DEFAULT uuidv7(),
    canonical_name text NOT NULL,
    canonical_name_norm text NOT NULL,
    gtfs_parent_stop_id text NOT NULL,
    gtfs_core_stop_id text,
    europa_id bigint,
    resolver_source text,
    resolver_ref_type text,
    resolver_ref_value text,
    resolver_name text,
    resolver_score numeric(6, 5),
    resolver_method text,
    netex_stop_place_id text,
    netex_private_code text,
    netex_short_name text,
    road_name text,
    station_type text NOT NULL DEFAULT 'station',
    transport_mode text,
    lat numeric(10, 8),
    lon numeric(11, 8),
    mobility_impaired_access boolean,
    wheelchair_access boolean,
    valid_from timestamptz,
    valid_to timestamptz,
    assistance_facility text,
    assistance_availability text,
    passenger_comms_facility text,
    ticketing_facility_list text,
    access_facility_list text,
    hire_facility text,
    match_quality text NOT NULL DEFAULT 'derived_from_gtfs_parent',
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT station_gtfs_parent_stop_id_unique UNIQUE (gtfs_parent_stop_id),
    CONSTRAINT station_gtfs_core_stop_id_unique UNIQUE (gtfs_core_stop_id),
    CONSTRAINT station_europa_id_unique UNIQUE (europa_id),
    CONSTRAINT station_netex_stop_place_id_unique UNIQUE (netex_stop_place_id),
    CONSTRAINT station_type_check CHECK (station_type IN ('station', 'stop_place', 'hub')),
    CONSTRAINT match_quality_check CHECK (
        match_quality IN (
            'derived_from_gtfs_parent',
            'matched_by_id',
            'matched_by_name',
            'matched_by_name_and_geo',
            'manual_review'
        )
    )
);

ALTER TABLE transit.station
    ADD COLUMN IF NOT EXISTS resolver_source text,
    ADD COLUMN IF NOT EXISTS resolver_ref_type text,
    ADD COLUMN IF NOT EXISTS resolver_ref_value text,
    ADD COLUMN IF NOT EXISTS resolver_name text,
    ADD COLUMN IF NOT EXISTS resolver_score numeric(6, 5),
    ADD COLUMN IF NOT EXISTS resolver_method text;

CREATE INDEX IF NOT EXISTS station_name_trgm_idx
    ON transit.station USING gin (canonical_name_norm gin_trgm_ops);

CREATE TABLE IF NOT EXISTS transit.station_name (
    station_name_uuid uuid PRIMARY KEY DEFAULT uuidv7(),
    station_uuid uuid NOT NULL REFERENCES transit.station(station_uuid) ON DELETE CASCADE,
    source_name text NOT NULL,
    name text NOT NULL,
    name_norm text NOT NULL,
    language_code text,
    is_primary boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT station_name_source_name_check CHECK (
        source_name IN ('gtfs', 'europa', 'hafas_delfi', 'hafas_db', 'netex', 'manual')
    )
);

CREATE INDEX IF NOT EXISTS station_name_norm_trgm_idx
    ON transit.station_name USING gin (name_norm gin_trgm_ops);

CREATE TABLE IF NOT EXISTS transit.station_external_ref (
    station_external_ref_uuid uuid PRIMARY KEY DEFAULT uuidv7(),
    station_uuid uuid NOT NULL REFERENCES transit.station(station_uuid) ON DELETE CASCADE,
    source_name text NOT NULL,
    ref_type text NOT NULL,
    ref_value text NOT NULL,
    is_primary boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT station_external_ref_source_name_check CHECK (
        source_name IN ('gtfs', 'europa', 'hafas', 'netex')
    ),
    CONSTRAINT station_external_ref_unique UNIQUE (source_name, ref_type, ref_value)
);

ALTER TABLE transit.station_external_ref
    DROP CONSTRAINT IF EXISTS station_external_ref_source_name_check;

ALTER TABLE transit.station_external_ref
    ADD CONSTRAINT station_external_ref_source_name_check CHECK (
        source_name IN ('gtfs', 'europa', 'hafas', 'netex', 'mcp_local')
    );

CREATE TABLE IF NOT EXISTS transit.station_match (
    station_match_uuid uuid PRIMARY KEY DEFAULT uuidv7(),
    station_uuid uuid NOT NULL REFERENCES transit.station(station_uuid) ON DELETE CASCADE,
    source_name text NOT NULL,
    source_row_id bigint NOT NULL,
    matched_ref text,
    matched_name text,
    matched_name_norm text,
    score numeric(6, 5),
    match_method text NOT NULL,
    match_status text NOT NULL DEFAULT 'candidate',
    notes text,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT station_match_source_name_check CHECK (
        source_name IN ('europa', 'hafas', 'netex')
    ),
    CONSTRAINT station_match_method_check CHECK (
        match_method IN ('exact_id', 'exact_name', 'normalized_name', 'name_plus_geo', 'manual')
    ),
    CONSTRAINT station_match_status_check CHECK (
        match_status IN ('candidate', 'accepted', 'rejected', 'needs_review')
    )
);

-- One row per child stop from stops.txt that is not the parent station itself.
CREATE TABLE IF NOT EXISTS transit.station_component (
    station_component_uuid uuid PRIMARY KEY DEFAULT uuidv7(),
    station_uuid uuid NOT NULL REFERENCES transit.station(station_uuid) ON DELETE CASCADE,
    gtfs_stop_id text NOT NULL,
    component_name text NOT NULL,
    component_name_norm text NOT NULL,
    component_kind text NOT NULL,
    location_type smallint,
    level_id text,
    platform_code text,
    lat numeric(10, 8),
    lon numeric(11, 8),
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT station_component_gtfs_stop_id_unique UNIQUE (gtfs_stop_id),
    CONSTRAINT station_component_kind_check CHECK (
        component_kind IN (
            'platform_stop',
            'entrance_exit',
            'generic_node',
            'boarding_area',
            'bus_area',
            'park_ride',
            'bike_ride',
            'mezzanine',
            'replacement_service',
            'other'
        )
    )
);

CREATE INDEX IF NOT EXISTS station_component_name_trgm_idx
    ON transit.station_component USING gin (component_name_norm gin_trgm_ops);

CREATE TABLE IF NOT EXISTS transit.station_platform (
    station_platform_uuid uuid PRIMARY KEY DEFAULT uuidv7(),
    station_uuid uuid NOT NULL REFERENCES transit.station(station_uuid) ON DELETE CASCADE,
    platform_code text NOT NULL,
    label text NOT NULL,
    level_id text,
    lat numeric(10, 8),
    lon numeric(11, 8),
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT station_platform_unique UNIQUE (station_uuid, platform_code)
);

CREATE TABLE IF NOT EXISTS transit.station_platform_stop (
    station_platform_stop_uuid uuid PRIMARY KEY DEFAULT uuidv7(),
    station_platform_uuid uuid NOT NULL REFERENCES transit.station_platform(station_platform_uuid) ON DELETE CASCADE,
    station_component_uuid uuid NOT NULL REFERENCES transit.station_component(station_component_uuid) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT station_platform_stop_unique UNIQUE (station_platform_uuid, station_component_uuid)
);

CREATE TABLE IF NOT EXISTS transit.station_parking (
    station_parking_uuid uuid PRIMARY KEY DEFAULT uuidv7(),
    station_uuid uuid NOT NULL REFERENCES transit.station(station_uuid) ON DELETE CASCADE,
    station_component_uuid uuid REFERENCES transit.station_component(station_component_uuid) ON DELETE CASCADE,
    parking_type text NOT NULL,
    name text NOT NULL,
    name_norm text NOT NULL,
    level_id text,
    lat numeric(10, 8),
    lon numeric(11, 8),
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT station_parking_type_check CHECK (
        parking_type IN ('park_ride', 'bike_ride', 'other')
    )
);

-- NeTEx quays can represent grouped physical platforms such as "1/11" or "2/3".
CREATE TABLE IF NOT EXISTS transit.station_quay_group (
    station_quay_group_uuid uuid PRIMARY KEY DEFAULT uuidv7(),
    station_uuid uuid NOT NULL REFERENCES transit.station(station_uuid) ON DELETE CASCADE,
    netex_quay_id text NOT NULL,
    quay_type text,
    label text NOT NULL,
    label_norm text NOT NULL,
    mobility_impaired_access boolean,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT station_quay_group_netex_quay_id_unique UNIQUE (netex_quay_id)
);

CREATE TABLE IF NOT EXISTS transit.station_quay_group_member (
    station_quay_group_member_uuid uuid PRIMARY KEY DEFAULT uuidv7(),
    station_quay_group_uuid uuid NOT NULL REFERENCES transit.station_quay_group(station_quay_group_uuid) ON DELETE CASCADE,
    station_platform_uuid uuid NOT NULL REFERENCES transit.station_platform(station_platform_uuid) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT station_quay_group_member_unique UNIQUE (station_quay_group_uuid, station_platform_uuid)
);

CREATE OR REPLACE VIEW transit.station_search AS
SELECT
    s.station_uuid,
    s.canonical_name,
    s.canonical_name_norm,
    s.gtfs_parent_stop_id,
    s.gtfs_core_stop_id,
    s.europa_id,
    s.netex_stop_place_id,
    s.netex_private_code,
    s.netex_short_name,
    s.road_name,
    s.lat,
    s.lon,
    array_remove(array_agg(DISTINCT sn.name), NULL) AS names,
    array_remove(array_agg(DISTINCT ser.ref_value), NULL) AS external_refs
FROM transit.station s
LEFT JOIN transit.station_name sn
    ON sn.station_uuid = s.station_uuid
LEFT JOIN transit.station_external_ref ser
    ON ser.station_uuid = s.station_uuid
GROUP BY
    s.station_uuid,
    s.canonical_name,
    s.canonical_name_norm,
    s.gtfs_parent_stop_id,
    s.gtfs_core_stop_id,
    s.europa_id,
    s.netex_stop_place_id,
    s.netex_private_code,
    s.netex_short_name,
    s.road_name,
    s.lat,
    s.lon;

-- Import rules for the current files:
-- 1. Decode europa.csv as cp1252, not UTF-8. The visible "?" problem is an import-encoding problem.
-- 2. Build one transit.station row from each GTFS parent row where location_type = '1'.
-- 3. Set gtfs_core_stop_id by stripping the leading 'P' from GTFS parent IDs like Pat:43:7242 -> at:43:7242.
-- 4. Load all non-parent GTFS rows into transit.station_component.
-- 5. For rows with platform_code, create one transit.station_platform row per (station_uuid, platform_code).
-- 6. Load NeTEx StopPlace rows by netex_stop_place_id and attach them when gtfs_core_stop_id = netex_stop_place_id.
-- 7. Load NeTEx Quay rows into transit.station_quay_group and parse labels like 'Bahnsteig 1/11' into members 1 and 11.
-- 7a. Keep NeTEx name/address/facility/accessibility/validity fields on transit.station.
-- 8. Match europa_raw to station by exact normalized name first. For Austrian rows this already matches many stations, including:
--      europa_id 8100009 <-> Pat:43:7242 <-> at:43:7242 <-> 'St.Valentin'
-- 9. Load hafas_mapping_raw into transit.station_match, do not force it directly into station unless the match is high confidence.
--    The file contains multiple low-confidence rows and sometimes platform-level IFOPT IDs, so it must remain auditable.
-- 10. Search against transit.station_search or transit.station joined to station_name using trigram on canonical_name_norm/name_norm.
