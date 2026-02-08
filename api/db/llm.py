"""Unified LLM client with auto-detected provider support.

Supports multiple providers with automatic fallback:
1. OPENAI_API_KEY -> OpenAI (gpt-4o-mini default)
2. ANTHROPIC_API_KEY -> Anthropic (claude-sonnet-4-5-20250929 default)
3. GOOGLE_API_KEY -> Google Gemini (gemini-2.5-flash default)
4. LITELLM_BASE_URL -> LiteLLM proxy (backwards compat)
5. None set -> AI features disabled
"""
import os
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


def complete(
    messages: list[dict],
    model: Optional[str] = None,
    max_tokens: int = 2048,
    json_mode: bool = False,
) -> Optional[str]:
    """Send a chat completion request to the configured LLM provider.

    Args:
        messages: List of {"role": "user"|"assistant"|"system", "content": "..."}
        model: Override the default model. If None, uses provider default
               or LLM_MODEL env var.
        max_tokens: Maximum tokens in the response.
        json_mode: If True, request JSON output format (where supported).

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

    try:
        if provider == "anthropic":
            return _complete_anthropic(client, messages, model, max_tokens)
        else:
            return _complete_openai(client, messages, model, max_tokens, json_mode)
    except Exception as e:
        print(f"LLM error ({provider}): {e}")
        return None


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
