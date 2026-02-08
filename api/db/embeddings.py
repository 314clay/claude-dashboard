"""Embedding-based semantic similarity search.

Generates vector embeddings for messages using OpenAI text-embedding-3-small,
stores them as BLOBs in SQLite, and computes cosine similarity at query time.
"""
import struct
import threading
from typing import Optional

import numpy as np

from .queries import get_connection
from . import llm

# In-memory embedding cache (lazy-loaded)
_cache_lock = threading.Lock()
_embedding_cache: Optional[dict] = None  # {message_ids: list[int], matrix: np.ndarray, model: str}


def _get_openai_client():
    """Get an OpenAI-compatible client for embeddings.

    OpenAI and Google (via OpenAI-compat endpoint) support embeddings.
    Anthropic does not have an embedding API.
    Falls back to OpenAI if available, else returns None.
    """
    import os

    # Direct OpenAI key - best option
    if os.environ.get("OPENAI_API_KEY"):
        from openai import OpenAI
        return OpenAI(api_key=os.environ["OPENAI_API_KEY"]), "text-embedding-3-small"

    # Google via OpenAI-compat endpoint
    if os.environ.get("GOOGLE_API_KEY"):
        from openai import OpenAI
        return OpenAI(
            api_key=os.environ["GOOGLE_API_KEY"],
            base_url="https://generativelanguage.googleapis.com/v1beta/openai/",
        ), "text-embedding-004"

    return None, None


def _invalidate_cache():
    """Invalidate the in-memory embedding cache."""
    global _embedding_cache
    with _cache_lock:
        _embedding_cache = None


def _ensure_table():
    """Create the message_embeddings table if it doesn't exist."""
    conn = get_connection()
    conn.execute("""
        CREATE TABLE IF NOT EXISTS message_embeddings (
            message_id  INTEGER PRIMARY KEY,
            model       TEXT    NOT NULL,
            dimensions  INTEGER NOT NULL,
            embedding   BLOB    NOT NULL,
            created_at  TEXT    DEFAULT (datetime('now')),
            FOREIGN KEY (message_id) REFERENCES messages(id) ON DELETE CASCADE
        )
    """)
    conn.commit()
    conn.close()


def embed_texts(texts: list[str], batch_size: int = 100) -> list[list[float]]:
    """Batch embed texts via OpenAI API.

    Args:
        texts: List of text strings to embed.
        batch_size: Number of texts per API call.

    Returns:
        List of embedding vectors (list of floats).

    Raises:
        RuntimeError: If no embedding provider is available.
    """
    client, model = _get_openai_client()
    if client is None:
        raise RuntimeError(
            "No embedding provider available. Set OPENAI_API_KEY or GOOGLE_API_KEY."
        )

    all_embeddings = []
    for i in range(0, len(texts), batch_size):
        batch = texts[i:i + batch_size]
        # Truncate very long texts to avoid token limits
        batch = [t[:8000] if len(t) > 8000 else t for t in batch]
        response = client.embeddings.create(input=batch, model=model)
        for item in response.data:
            all_embeddings.append(item.embedding)

    return all_embeddings


def get_unembedded_message_ids(limit: int = 1000) -> list[dict]:
    """Get messages that don't have embeddings yet.

    Returns:
        List of dicts: [{"id": int, "content": str}, ...]
    """
    _ensure_table()
    conn = get_connection()
    cur = conn.cursor()

    cur.execute("""
        SELECT m.id, m.content
        FROM messages m
        WHERE NOT EXISTS (
            SELECT 1 FROM message_embeddings me
            WHERE me.message_id = m.id
        )
        AND m.content IS NOT NULL
        AND length(m.content) > 0
        ORDER BY m.timestamp DESC
        LIMIT ?
    """, (limit,))

    messages = [dict(row) for row in cur.fetchall()]
    cur.close()
    conn.close()
    return messages


def save_embeddings(
    embeddings: list[tuple[int, list[float]]],
    model: str,
    dims: int,
):
    """Save embeddings to database as BLOBs.

    Args:
        embeddings: List of (message_id, vector) tuples.
        model: Model name used for embedding.
        dims: Dimensionality of the vectors.
    """
    _ensure_table()
    conn = get_connection()
    cur = conn.cursor()

    for msg_id, vector in embeddings:
        blob = struct.pack(f'{len(vector)}f', *vector)
        cur.execute("""
            INSERT OR REPLACE INTO message_embeddings
            (message_id, model, dimensions, embedding)
            VALUES (?, ?, ?, ?)
        """, (msg_id, model, dims, blob))

    conn.commit()
    cur.close()
    conn.close()
    _invalidate_cache()


def get_embedding_stats() -> dict:
    """Get embedding coverage statistics.

    Returns:
        dict with total, embedded, unembedded, model
    """
    _ensure_table()
    conn = get_connection()
    cur = conn.cursor()

    cur.execute("SELECT COUNT(*) as total FROM messages")
    total = cur.fetchone()['total']

    cur.execute("SELECT COUNT(*) as embedded FROM message_embeddings")
    embedded = cur.fetchone()['embedded']

    cur.execute("""
        SELECT model FROM message_embeddings LIMIT 1
    """)
    row = cur.fetchone()
    model = row['model'] if row else None

    cur.close()
    conn.close()

    return {
        "total": total,
        "embedded": embedded,
        "unembedded": total - embedded,
        "model": model,
    }


def _load_cache() -> dict:
    """Load all embeddings into memory as a numpy matrix.

    Returns:
        dict with message_ids (list[int]), matrix (np.ndarray), model (str)
    """
    global _embedding_cache

    with _cache_lock:
        if _embedding_cache is not None:
            return _embedding_cache

    _ensure_table()
    conn = get_connection()
    cur = conn.cursor()

    cur.execute("""
        SELECT message_id, dimensions, embedding
        FROM message_embeddings
        ORDER BY message_id
    """)

    message_ids = []
    vectors = []
    dims = None

    for row in cur:
        msg_id = row['message_id']
        d = row['dimensions']
        blob = row['embedding']

        if dims is None:
            dims = d

        vector = list(struct.unpack(f'{d}f', blob))
        message_ids.append(msg_id)
        vectors.append(vector)

    cur.close()
    conn.close()

    if not vectors:
        cache = {"message_ids": [], "matrix": np.array([]), "model": None}
    else:
        matrix = np.array(vectors, dtype=np.float32)
        # Normalize for cosine similarity (dot product on normalized = cosine)
        norms = np.linalg.norm(matrix, axis=1, keepdims=True)
        norms[norms == 0] = 1.0
        matrix = matrix / norms
        cache = {"message_ids": message_ids, "matrix": matrix, "model": None}

    with _cache_lock:
        _embedding_cache = cache

    return cache


def search_by_query(query_text: str) -> dict[int, float]:
    """Embed query and compute cosine similarity against all stored embeddings.

    Args:
        query_text: The search query.

    Returns:
        dict mapping message_id -> similarity score (0.0 to 1.0)
    """
    # Embed the query
    query_vectors = embed_texts([query_text])
    if not query_vectors:
        return {}

    query_vec = np.array(query_vectors[0], dtype=np.float32)
    # Normalize query vector
    norm = np.linalg.norm(query_vec)
    if norm > 0:
        query_vec = query_vec / norm

    # Load cached embeddings
    cache = _load_cache()
    if len(cache["message_ids"]) == 0:
        return {}

    # Cosine similarity = dot product of normalized vectors
    similarities = cache["matrix"] @ query_vec

    # Convert to dict, mapping cosine similarity from [-1, 1] to [0, 1]
    scores = {}
    for i, msg_id in enumerate(cache["message_ids"]):
        score = float((similarities[i] + 1.0) / 2.0)  # Remap to 0-1
        scores[msg_id] = score

    return scores


def generate_embeddings(
    batch_size: int = 100,
    max_messages: int = 1000,
) -> dict:
    """Generate embeddings for unembedded messages.

    Args:
        batch_size: Messages per API call.
        max_messages: Maximum messages to process.

    Returns:
        dict with generated count, model, dimensions, errors
    """
    client, model = _get_openai_client()
    if client is None:
        return {
            "generated": 0,
            "model": None,
            "dimensions": None,
            "error": "No embedding provider available. Set OPENAI_API_KEY or GOOGLE_API_KEY.",
        }

    messages = get_unembedded_message_ids(limit=max_messages)
    if not messages:
        return {
            "generated": 0,
            "model": model,
            "dimensions": None,
            "error": None,
        }

    generated = 0
    errors = []
    dims = None

    for i in range(0, len(messages), batch_size):
        batch = messages[i:i + batch_size]
        texts = []
        ids = []
        for msg in batch:
            content = msg['content'] or ""
            if len(content) > 8000:
                content = content[:8000]
            texts.append(content)
            ids.append(msg['id'])

        try:
            vectors = embed_texts(texts, batch_size=len(texts))
            if vectors:
                dims = len(vectors[0])
                pairs = list(zip(ids, vectors))
                save_embeddings(pairs, model, dims)
                generated += len(pairs)
        except Exception as e:
            errors.append(f"Batch {i // batch_size}: {str(e)}")

    return {
        "generated": generated,
        "model": model,
        "dimensions": dims,
        "errors": errors if errors else None,
    }
