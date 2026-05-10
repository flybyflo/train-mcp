#!/usr/bin/env bash
set -euo pipefail

if [[ ! -f /docker-entrypoint-initdb.d/03_station_data.dump ]]; then
  echo "station data dump not found, skipping restore"
  exit 0
fi

echo "restoring station data dump into ${POSTGRES_DB}"
pg_restore \
  --verbose \
  --data-only \
  --disable-triggers \
  --no-owner \
  --no-privileges \
  -U "${POSTGRES_USER}" \
  -d "${POSTGRES_DB}" \
  /docker-entrypoint-initdb.d/03_station_data.dump

echo "station data restore complete"
