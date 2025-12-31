#!/bin/bash
# Start the Dashboard Native app with its Python API backend

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}Starting Claude Activity Dashboard (Native)${NC}"

# Check if API is already running
if curl -s http://127.0.0.1:8000/health > /dev/null 2>&1; then
    echo -e "${YELLOW}API already running on port 8000${NC}"
else
    echo "Starting Python API..."
    cd api
    /Users/clayarnold/w/connect/venv/bin/uvicorn main:app --host 127.0.0.1 --port 8000 &
    API_PID=$!
    cd ..

    # Wait for API to be ready
    echo -n "Waiting for API..."
    for i in {1..30}; do
        if curl -s http://127.0.0.1:8000/health > /dev/null 2>&1; then
            echo -e " ${GREEN}ready${NC}"
            break
        fi
        echo -n "."
        sleep 0.5
    done
fi

# Build if needed
if [ ! -f target/release/dashboard-native ]; then
    echo "Building Rust app (first time may take a while)..."
    source ~/.cargo/env
    cargo build --release
fi

# Run the app
echo -e "${GREEN}Launching dashboard...${NC}"
source ~/.cargo/env
./target/release/dashboard-native

# Cleanup - kill API if we started it
if [ -n "$API_PID" ]; then
    echo "Shutting down API..."
    kill $API_PID 2>/dev/null || true
fi
