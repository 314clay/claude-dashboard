#!/bin/bash
# Smoke test: build and run the full setup in a clean Docker container.
#
# This simulates a new user cloning the repo on a fresh Ubuntu machine.
# It validates: system deps, Python deps, Rust compilation, API health,
# and all tests (Rust + Python).
#
# Usage:
#   ./scripts/smoke-test.sh           # full smoke test
#   ./scripts/smoke-test.sh --no-cache # rebuild from scratch

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(dirname "$SCRIPT_DIR")"

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${GREEN}Dashboard Native — Smoke Test${NC}"
echo "Simulating fresh user setup in Docker..."
echo ""

# Check Docker is available
if ! command -v docker &> /dev/null; then
    echo -e "${RED}Error: Docker is not installed or not in PATH${NC}"
    echo "Install Docker: https://docs.docker.com/get-docker/"
    exit 1
fi

# Pass through any flags (e.g., --no-cache)
DOCKER_ARGS=""
for arg in "$@"; do
    case "$arg" in
        --no-cache) DOCKER_ARGS="$DOCKER_ARGS --no-cache" ;;
        *) echo -e "${YELLOW}Unknown argument: $arg${NC}" ;;
    esac
done

cd "$REPO_DIR"

echo "Building smoke test image (this may take a few minutes on first run)..."
echo ""

docker build \
    -f Dockerfile.smoke-test \
    -t dashboard-native-smoke-test \
    $DOCKER_ARGS \
    .

echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}  Smoke test completed successfully!${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo "Verified:"
echo "  - System dependencies install on clean Ubuntu"
echo "  - Python dependencies install and import correctly"
echo "  - Rust compiles in release mode"
echo "  - Python tests pass"
echo "  - Rust tests pass"
echo "  - API starts and responds to health + data endpoints"
echo "  - Release binary is built"
