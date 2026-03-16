"""Configuration — mirrors src/config/ from the Rust implementation.

All settings are read from environment variables (or .env file).
"""

from __future__ import annotations

import os
from typing import Literal

from pydantic import Field
from pydantic_settings import BaseSettings, SettingsConfigDict


class LlmConfig(BaseSettings):
    """LLM provider configuration."""

    model_config = SettingsConfigDict(env_prefix="LLM_", extra="ignore")

    backend: Literal[
        "openai", "anthropic", "openai_compatible", "ollama"
    ] = "anthropic"
    model: str = "claude-sonnet-4-6"
    base_url: str | None = None
    api_key: str | None = None
    max_tokens: int = 4096
    temperature: float = 0.0


class AgentConfig(BaseSettings):
    """Core agent settings."""

    model_config = SettingsConfigDict(env_prefix="AGENT_", extra="ignore")

    name: str = "IronClaw"
    max_parallel_jobs: int = Field(default=5, ge=1)
    job_timeout_seconds: int = 300
    max_iterations: int = 50
    max_tokens_per_job: int = 0  # 0 = unlimited
    use_planning: bool = False
    session_idle_timeout_seconds: int = 3600
    allow_local_tools: bool = True
    auto_approve_tools: bool = False
    # Cost guard
    max_cost_per_day_cents: int | None = None
    max_actions_per_hour: int | None = None
    # Tool nudge
    enable_tool_intent_nudge: bool = True
    max_tool_intent_nudges: int = 2
    # Default timezone
    default_timezone: str = "UTC"


class SafetyConfig(BaseSettings):
    """Safety layer configuration."""

    model_config = SettingsConfigDict(env_prefix="SAFETY_", extra="ignore")

    injection_check_enabled: bool = True
    max_output_length: int = 100_000


class DatabaseConfig(BaseSettings):
    """Database connection settings."""

    model_config = SettingsConfigDict(env_prefix="DATABASE_", extra="ignore")

    url: str | None = None


class ChannelConfig(BaseSettings):
    """Channel enablement flags."""

    model_config = SettingsConfigDict(extra="ignore")

    repl_enabled: bool = Field(default=True, alias="REPL_ENABLED")
    http_enabled: bool = Field(default=False, alias="HTTP_ENABLED")
    http_port: int = Field(default=8080, alias="HTTP_PORT")
    http_secret: str | None = Field(default=None, alias="HTTP_SECRET")


class Config(BaseSettings):
    """Root configuration — composes all subsystem configs."""

    model_config = SettingsConfigDict(
        env_file=".env",
        env_file_encoding="utf-8",
        extra="ignore",
    )

    llm: LlmConfig = Field(default_factory=LlmConfig)
    agent: AgentConfig = Field(default_factory=AgentConfig)
    safety: SafetyConfig = Field(default_factory=SafetyConfig)
    database: DatabaseConfig = Field(default_factory=DatabaseConfig)
    channels: ChannelConfig = Field(default_factory=ChannelConfig)

    @classmethod
    def load(cls) -> "Config":
        """Load config from environment, with .env fallback."""
        return cls()
