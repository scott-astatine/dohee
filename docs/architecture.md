# Dohee (도회) Architecture

Dohee is a local-first coding assistant agent.

## Crate Responsibilities

- **`dohee-core`**: Manages the main agentic REPL loop and the state machine of the coding session. It coordinates model prompts, parsed tool invocations, user approval gates, and the feedback loop.
- **`dohee-infer`**: Encapsulates the in-process llama.cpp bindings (via `llama-cpp-2`). It is responsible for model loading, VRAM/RAM allocation, context updates, KV cache management, and token stream generation.
- **`dohee-tools`**: Implements the core action set accessible to the AI agent. This includes filesystem directory listing, file reading, file editing (diff/regex patching), grep searching, and local shell execution.
- **`dohee-sandbox`**: Provides process-level sandboxing on Linux using Landlock LSM. It limits the capabilities of spawned shell tools to prevent access to the host outside the project directory and block unauthorized network requests.
- **`dohee-context`**: Handles token accounting, conversation compaction, and tool output pruning. It manages context windows by replacing old output logs with truncated summaries and running local LLM summarization.
- **`dohee-store`**: Implements session persistence using SQLite (via `rusqlite`). It saves active chat histories, model profiles, and token logs to allow resuming past coding sessions after process restarts.
- **`dohee-mcp`**: Implements a client for the Model Context Protocol (MCP). It allows the local agent to connect to third-party MCP servers, exposing external databases, tools, or resources as standard agent tools.
- **`dohee-config`**: Manages the configuration schemas and loading hierarchy for the project. It merges built-in settings with global config files (`~/.config/dohee/config.toml`) and project-specific files (`.dohee.toml`).
- **`dohee-tui`**: Provides an interactive terminal user interface built on `ratatui`. It displays session history panels, live token usage meters, and interactive approval menus for agent tool invocations.
- **`dohee-cli`**: The binary entrypoint for the `dohee` executable. It handles CLI arguments (via `clap`), initializes the required configurations, spins up selected interfaces (CLI or TUI), and drives the session.
