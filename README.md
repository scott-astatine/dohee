# Dohee (도회) - Local-First AI Coding Agent

Dohee (도회) is a lightweight, high-performance local AI coding agent written in Rust. Unlike general agent wrappers, Dohee compiles its inference engine **directly in-process** using statically linked `llama.cpp` bindings, requiring no background subprocesses or HTTP servers. It features terminal raw-mode UI (Ratatui), OS-level Landlock LSM sandboxing, dynamic GBNF grammar constraints, and SQLite session persistence.

---

## Key Features

- **In-Process Inference**: Zero networking. It links with `llama.cpp` statically at compile time. Model load is handled directly within the main executable.
- **GPU (Vulkan) / CPU Hybrid Offloading**: Direct Vulkan drivers support for full/partial GPU offloading.
- **Layered TOML Configuration**: Overrides stack: Built-in Defaults $\rightarrow$ Global Config (`~/.config/dohee/config.toml`) $\rightarrow$ Project Local Config (`.dohee.toml`) $\rightarrow$ CLI Flags.
- **Landlock LSM Sandboxing**: Spawns terminal commands inside a child-process sandbox, restricting filesystem reads to read-only and writes exclusively to the project workspace directory.
- **Structured GBNF Grammar**: Constrains raw model token selection to prevent malformed or half-written tool XML tags.
- **SQLite Session Store**: Automatically records and lists conversational histories, allowing you to resume active agent sessions after a process interrupt (`Ctrl+C`).
- **Interactive TUI Chat**: Custom terminal layout with separate panels for streaming text, token meters, sandbox policies, and interactive tool confirmation prompts.

---

## How to Build & Run

### Prerequisites
- **Rust Toolchain** (Rust 1.75+ or Cargo).
- **Vulkan Drivers & Headers** (if building with GPU acceleration).

### 1. Build from Source
Build the workspace binary:
```bash
# Compile with Vulkan acceleration
cargo build --release --workspace --features vulkan

# Compile for CPU-only fallback
cargo build --release --workspace
```

### 2. Configure Your Settings
Create a global config file in `~/.config/dohee/config.toml` or a local one in your project directory called `.dohee.toml`:
```toml
# Example Config
backend = "vulkan"
ctx_size = 2048
temperature = 0.2
seed = 1234
gpu_layers = 99
sandbox_policy = "WorkspaceWrite"
```

---

## CLI Usage Reference

### Commands Overview
- `dohee chat [prompt]`: Launches the interactive Terminal User Interface (TUI). If a prompt is supplied, it starts the session automatically.
- `dohee run <prompt>`: Executes a single-shot prompt in the terminal, showing streamed outputs and tool calls.
- `dohee config-show`: Renders the active merged configurations.
- `dohee models-list`: Lists GGUF files in your local `models/` directory.
- `dohee sessions-list`: Displays session IDs and timestamps from the SQLite database.
- `dohee sessions-resume <id>`: Reloads the session history and resumes the agent loop.

### Command Examples

```bash
# Run TUI chat mode
cargo run --release --features vulkan -- chat

# Run a query in single-shot mode
cargo run --release --features vulkan -- -m models/qwen2.5-1.5b-instruct-q4_k_m.gguf "write a python hello world script"

# Resume session from SQLite
cargo run --release --features vulkan -- sessions-resume session-1719999999
```

---

## Crate Layout & Architecture

- `dohee-cli`: Main executable binary entrypoint.
- `dohee-core`: Handles the agent state machine, prompt templates, and turn loops.
- `dohee-infer`: Links `llama-cpp-2` statically and implements Detokenization/Sampling.
- `dohee-tools`: Unified `Tool` trait and built-in filesystem read/write/edit/shell tools.
- `dohee-sandbox`: Linux Landlock LSM file system isolation rules.
- `dohee-context`: Pruning strategies and token length calculators.
- `dohee-store`: SQLite session logs persistence.
- `dohee-tui`: Interactive drawing loops using `ratatui` and `crossterm`.
