"""Unified LLM client with auto-detected provider support.

Supports multiple providers with automatic fallback:
1. OPENAI_API_KEY -> OpenAI (gpt-4o-mini default)
2. ANTHROPIC_API_KEY -> Anthropic (claude-sonnet-4-5-20250929 default)
3. GOOGLE_API_KEY -> Google Gemini (gemini-2.5-flash default)
4. LITELLM_BASE_URL -> LiteLLM proxy (backwards compat)
5. None set -> AI features disabled

Includes transparent response caching in SQLite to avoid repeating
identical LLM calls across app restarts.
"""
import hashlib
import json
import os
import sqlite3
from typing import Optional

# Provider detection order
_PROVIDER: Optional[str] = None
_DEFAULT_MODEL: Optional[str] = None
_CLIENT = None
_initialized = False


class LLMUnavailableError(Exception):
    """Raised when no LLM provider is configured."""
    pass


def _detect_provider() -> tuple[Optional[str], Optional[str]]:
    """Detect which LLM provider is available.

    Returns (provider_name, default_model) or (None, None).
    """
    forced = os.environ.get("LLM_PROVIDER", "").lower().strip()
    if forced:
        defaults = {
            "openai": "gpt-4o-mini",
            "anthropic": "claude-sonnet-4-5-20250929",
            "google": "gemini-2.5-flash",
            "litellm": "fast",
        }
        if forced in defaults:
            return forced, defaults[forced]

    if os.environ.get("OPENAI_API_KEY"):
        return "openai", "gpt-4o-mini"
    if os.environ.get("ANTHROPIC_API_KEY"):
        return "anthropic", "claude-sonnet-4-5-20250929"
    if os.environ.get("GOOGLE_API_KEY"):
        return "google", "gemini-2.5-flash"
    if os.environ.get("LITELLM_BASE_URL"):
        return "litellm", os.environ.get("SUMMARY_MODEL", "fast")

    return None, None


def _get_client():
    """Get or create the LLM client for the detected provider."""
    global _PROVIDER, _DEFAULT_MODEL, _CLIENT, _initialized

    if _initialized:
        return _CLIENT, _PROVIDER, _DEFAULT_MODEL

    _PROVIDER, _DEFAULT_MODEL = _detect_provider()
    _initialized = True

    if _PROVIDER is None:
        _CLIENT = None
        return _CLIENT, _PROVIDER, _DEFAULT_MODEL

    if _PROVIDER == "openai":
        from openai import OpenAI
        _CLIENT = OpenAI(api_key=os.environ["OPENAI_API_KEY"])

    elif _PROVIDER == "anthropic":
        # Use anthropic SDK directly
        import anthropic
        _CLIENT = anthropic.Anthropic(api_key=os.environ["ANTHROPIC_API_KEY"])

    elif _PROVIDER == "google":
        from openai import OpenAI
        _CLIENT = OpenAI(
            api_key=os.environ["GOOGLE_API_KEY"],
            base_url="https://generativelanguage.googleapis.com/v1beta/openai/",
        )

    elif _PROVIDER == "litellm":
        from openai import OpenAI
        base_url = os.environ.get("LITELLM_BASE_URL", "http://localhost:4001")
        api_key = os.environ.get("LITELLM_API_KEY", "sk-litellm-master-key")
        _CLIENT = OpenAI(base_url=f"{base_url}/v1", api_key=api_key)

    return _CLIENT, _PROVIDER, _DEFAULT_MODEL


def reset():
    """Reset cached client (useful for testing or env changes)."""
    global _PROVIDER, _DEFAULT_MODEL, _CLIENT, _initialized
    _PROVIDER = None
    _DEFAULT_MODEL = None
    _CLIENT = None
    _initialized = False


def is_available() -> bool:
    """Check if an LLM provider is configured."""
    _get_client()
    return _PROVIDER is not None


def get_provider() -> Optional[str]:
    """Return the name of the active provider, or None."""
    _get_client()
    return _PROVIDER


# ---- Response cache helpers ----

def _get_db_path() -> str:
    """Get the database path, matching the pattern in queries.py."""
    _default = os.path.join(
        os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config")),
        "dashboard-native",
        "dashboard.db",
    )
    return os.environ.get("DB_PATH", _default)


def _compute_prompt_hash(
    messages: list[dict],
    model: str,
    max_tokens: int,
    json_mode: bool,
) -> str:
    """Compute a stable SHA-256 hash of the full prompt parameters.

    Deterministic: same messages + model + max_tokens + json_mode -> same hash.
    """
    payload = json.dumps(
        {
            "messages": messages,
            "model": model,
            "max_tokens": max_tokens,
            "json_mode": json_mode,
        },
        sort_keys=True,
        ensure_ascii=True,
    )
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def _cache_lookup(prompt_hash: str) -> Optional[str]:
    """Check the llm_cache table for a cached response.

    Returns the cached response text, or None if not found.
    """
    try:
        db_path = _get_db_path()
        if not os.path.exists(db_path):
            return None
        conn = sqlite3.connect(db_path)
        cur = conn.cursor()
        cur.execute(
            "SELECT response FROM llm_cache WHERE prompt_hash = ?",
            (prompt_hash,),
        )
        row = cur.fetchone()
        cur.close()
        conn.close()
        return row[0] if row else None
    except Exception:
        # Cache lookup should never block the actual call
        return None


def _cache_store(
    prompt_hash: str,
    model: str,
    response: str,
    tokens_used: Optional[int] = None,
) -> None:
    """Store a response in the llm_cache table."""
    try:
        db_path = _get_db_path()
        os.makedirs(os.path.dirname(db_path), exist_ok=True)
        conn = sqlite3.connect(db_path)
        conn.execute(
            """INSERT OR REPLACE INTO llm_cache
               (prompt_hash, model, response, tokens_used)
               VALUES (?, ?, ?, ?)""",
            (prompt_hash, model, response, tokens_used),
        )
        conn.commit()
        conn.close()
    except Exception as e:
        # Cache store should never break the caller
        print(f"LLM cache store warning: {e}")


def complete(
    messages: list[dict],
    model: Optional[str] = None,
    max_tokens: int = 2048,
    json_mode: bool = False,
    skip_cache: bool = False,
) -> Optional[str]:
    """Send a chat completion request to the configured LLM provider.

    Responses are transparently cached in the SQLite ``llm_cache`` table so
    that identical prompts are not re-sent after an app restart.

    Args:
        messages: List of {"role": "user"|"assistant"|"system", "content": "..."}
        model: Override the default model. If None, uses provider default
               or LLM_MODEL env var.
        max_tokens: Maximum tokens in the response.
        json_mode: If True, request JSON output format (where supported).
        skip_cache: If True, bypass the cache and always call the API.

    Returns:
        The response text, or None on error.

    Raises:
        LLMUnavailableError: If no provider is configured.
    """
    client, provider, default_model = _get_client()

    if provider is None:
        raise LLMUnavailableError(
            "No LLM provider configured. Set one of: OPENAI_API_KEY, "
            "ANTHROPIC_API_KEY, GOOGLE_API_KEY, or LITELLM_BASE_URL"
        )

    model = model or os.environ.get("LLM_MODEL") or default_model

    # --- Cache check ---
    prompt_hash = None
    if not skip_cache:
        prompt_hash = _compute_prompt_hash(messages, model, max_tokens, json_mode)
        cached = _cache_lookup(prompt_hash)
        if cached is not None:
            return cached

    # --- Actual API call ---
    try:
        if provider == "anthropic":
            response = _complete_anthropic(client, messages, model, max_tokens)
        else:
            response = _complete_openai(client, messages, model, max_tokens, json_mode)
    except Exception as e:
        print(f"LLM error ({provider}): {e}")
        return None

    # --- Cache store ---
    if response is not None and prompt_hash is not None:
        _cache_store(prompt_hash, model, response)

    return response


def _complete_openai(client, messages, model, max_tokens, json_mode):
    """Complete via OpenAI-compatible API (OpenAI, Google, LiteLLM)."""
    kwargs = {
        "model": model,
        "messages": messages,
        "max_tokens": max_tokens,
    }
    if json_mode:
        kwargs["response_format"] = {"type": "json_object"}

    response = client.chat.completions.create(**kwargs)
    return response.choices[0].message.content


def _complete_anthropic(client, messages, model, max_tokens):
    """Complete via Anthropic API.

    Converts OpenAI-style messages to Anthropic format:
    - system messages become the system parameter
    - user/assistant messages stay as-is
    """
    system_parts = []
    chat_messages = []

    for msg in messages:
        if msg["role"] == "system":
            system_parts.append(msg["content"])
        else:
            chat_messages.append({
                "role": msg["role"],
                "content": msg["content"],
            })

    # Anthropic requires at least one non-system message
    if not chat_messages:
        chat_messages = [{"role": "user", "content": ""}]

    kwargs = {
        "model": model,
        "messages": chat_messages,
        "max_tokens": max_tokens,
    }
    if system_parts:
        kwargs["system"] = "\n\n".join(system_parts)

    response = client.messages.create(**kwargs)
    # Anthropic returns a list of content blocks
    return response.content[0].text
