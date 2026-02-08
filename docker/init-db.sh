#!/bin/bash
# Initialize the claude_sessions schema on PostgreSQL startup.
# This script runs as the Docker entrypoint init script
# (placed in /docker-entrypoint-initdb.d/).

set -e

echo "Initializing claude_sessions schema..."

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" \
    -f /docker-entrypoint-initdb.d/schema.sql

echo "claude_sessions schema initialized successfully."
