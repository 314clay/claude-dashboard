"""Smoke tests for the dashboard API."""
from fastapi.testclient import TestClient
from main import app

client = TestClient(app)


def test_health():
    r = client.get("/health")
    assert r.status_code == 200
    assert r.json() == {"status": "ok"}


def test_graph_empty_db():
    r = client.get("/graph?hours=1")
    assert r.status_code == 200
    assert "nodes" in r.json()


def test_sessions_empty_db():
    r = client.get("/sessions?hours=1")
    assert r.status_code == 200
    assert "sessions" in r.json()


def test_semantic_filters_list():
    r = client.get("/semantic-filters")
    assert r.status_code == 200
    assert "filters" in r.json()
