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

## Baseline Performance Benchmarks

Measured on an Intel Iris Xe + NVIDIA GeForce GTX 1650 (Optimus) laptop using the **Qwen 2.5 1.5B Instruct (Q4_K_M)** model:

### Generation Speed (1-Paragraph Generation, ~160 tokens)
*   **Vulkan GPU Backend (`-b vulkan -g 99`)**:
    *   Total Execution Time: **76.89 seconds**
    *   Approximate Throughput: **2.1 tokens/sec** (including Vulkan driver initialization and GGUF loading overhead)
*   **CPU-only Backend (`-b cpu`)**:
    *   Total Execution Time: **91.41 seconds**
    *   Approximate Throughput: **1.7 tokens/sec**

### Resource Utilization
*   **Vulkan VRAM footprint (1.5B Q4_K_M)**: ~934 MiB
*   **Context limits tested**: up to 2048 context length
*   **Sandboxing overhead**: < 5ms startup delay during Landlock policy restriction application.

