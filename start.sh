#!/bin/bash
# Start the Dashboard Native app with its Python API backend

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${GREEN}Starting Claude Activity Dashboard (Native)${NC}"

# Find Python - check multiple options
find_python() {
    # 1. Check CONNECT_VENV env var (preferred for ConnectingServices)
    if [ -n "$CONNECT_VENV" ] && [ -f "$CONNECT_VENV/bin/python" ]; then
        echo "$CONNECT_VENV/bin/python"
        return
    fi

    # 2. Check for local venv
    if [ -f "venv/bin/python" ]; then
        echo "venv/bin/python"
        return
    fi

    # 3. Check for .venv (common alternative)
    if [ -f ".venv/bin/python" ]; then
        echo ".venv/bin/python"
        return
    fi

    # 4. Check VIRTUAL_ENV if set
    if [ -n "$VIRTUAL_ENV" ] && [ -f "$VIRTUAL_ENV/bin/python" ]; then
        echo "$VIRTUAL_ENV/bin/python"
        return
    fi

    # 5. Check for uvicorn in PATH
    if command -v uvicorn &> /dev/null; then
        echo "python"
        return
    fi

    # 6. Try system Python
    if command -v python3 &> /dev/null; then
        echo "python3"
        return
    fi

    echo ""
}

PYTHON=$(find_python)

if [ -z "$PYTHON" ]; then
    echo -e "${RED}Error: Could not find Python installation${NC}"
    echo ""
    echo "Please set up a Python environment:"
    echo "  python3 -m venv venv"
    echo "  source venv/bin/activate"
    echo "  pip install -r api/requirements.txt"
    exit 1
fi

# Check if API is already running
if curl -s http://127.0.0.1:8000/health > /dev/null 2>&1; then
    echo -e "${YELLOW}API already running on port 8000${NC}"
else
    echo "Starting Python API..."
    echo "  Using Python: $PYTHON"
    cd api

    # Install dependencies if needed (check for fastapi)
    if ! $PYTHON -c "import fastapi" 2>/dev/null; then
        echo -e "${YELLOW}Installing Python dependencies...${NC}"
        $PYTHON -m pip install -r requirements.txt --quiet
    fi

    $PYTHON -m uvicorn main:app --host 127.0.0.1 --port 8000 &
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

    # Check if we timed out
    if ! curl -s http://127.0.0.1:8000/health > /dev/null 2>&1; then
        echo -e " ${RED}failed${NC}"
        echo "API failed to start. Check the logs above for errors."
        exit 1
    fi
fi

# Build if needed
if [ ! -f target/release/dashboard-native ]; then
    echo "Building Rust app (first time may take a while)..."
    # Source cargo env if it exists
    [ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
    cargo build --release
fi

# Run the app
echo -e "${GREEN}Launching dashboard...${NC}"
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
./target/release/dashboard-native

# Cleanup - kill API if we started it
if [ -n "$API_PID" ]; then
    echo "Shutting down API..."
    kill $API_PID 2>/dev/null || true
fi
