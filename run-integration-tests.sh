#!/usr/bin/env bash
set -euo pipefail

CONTAINER="wisecrow-test-db"
DB_NAME="wisecrow_test"
DB_USER="wisecrow"
DB_PASS="wisecrow"
DB_PORT="${WISECROW_TEST_DB_PORT:-5433}"
TEST_URL="postgres://${DB_USER}:${DB_PASS}@localhost:${DB_PORT}/${DB_NAME}"

cleanup() {
    local exit_status=$?
    echo "Cleaning up..."
    if docker container inspect "${CONTAINER}" >/dev/null 2>&1; then
        if docker rm -f "${CONTAINER}"; then
            echo "Removed test PostgreSQL container."
        else
            echo "Failed to remove test PostgreSQL container." >&2
            exit_status=1
        fi
    else
        echo "Test PostgreSQL container was already absent."
    fi
    return "${exit_status}"
}
trap cleanup EXIT

echo "Starting test PostgreSQL container on port ${DB_PORT}..."
docker run -d \
    --name "${CONTAINER}" \
    -e POSTGRES_DB="${DB_NAME}" \
    -e POSTGRES_USER="${DB_USER}" \
    -e POSTGRES_PASSWORD="${DB_PASS}" \
    -p "${DB_PORT}:5432" \
    postgres:15-alpine >/dev/null

echo "Waiting for PostgreSQL to be ready..."
for i in $(seq 1 30); do
    if docker exec "${CONTAINER}" pg_isready -U "${DB_USER}" -d "${DB_NAME}" >/dev/null 2>&1; then
        echo "PostgreSQL ready after ${i}s."
        break
    fi
    if [ "$i" -eq 30 ]; then
        echo "Timed out waiting for PostgreSQL."
        exit 1
    fi
    sleep 1
done

echo "Running integration tests..."
TEST_DATABASE_URL="${TEST_URL}" CC=cc cargo nextest run \
    -p wisecrow-core \
    --run-ignored ignored-only \
    --test-threads=1 \
    "$@"

echo "Running web server integration tests..."
TEST_DATABASE_URL="${TEST_URL}" CC=cc cargo nextest run \
    -p wisecrow-web \
    --features server \
    --run-ignored ignored-only \
    --test-threads=1 \
    "$@"

echo "Done."
