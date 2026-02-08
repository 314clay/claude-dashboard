.PHONY: setup run build import import-recent api test clean help

help:              ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*##' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*## "}; {printf "  %-15s %s\n", $$1, $$2}'

setup:             ## Install deps, import history, build release binary
	pip install -r api/requirements.txt
	python3 ingest.py
	cargo build --release

run:               ## Launch the full dashboard (API + Rust app)
	./start.sh

build:             ## Build release binary
	cargo build --release

import:            ## Import all Claude Code history into SQLite
	python3 ingest.py

import-recent:     ## Import last 7 days of history
	python3 ingest.py --since 7d

api:               ## Start the Python API server only
	cd api && python3 -m uvicorn main:app --host 127.0.0.1 --port 8000

test:              ## Run all tests (Rust + Python)
	cargo test
	cd api && python3 -m pytest test_main.py -v

clean:             ## Remove build artifacts
	cargo clean
