# Dohee (도회) - Local-First AI Coding Agent

Dohee (도회) is a lightweight, high-performance local AI coding agent written in Rust. Unlike general agent wrappers, Dohee compiles its inference engine **directly in-process** using statically linked `llama.cpp` bindings, requiring no background subprocesses or HTTP servers. It features a borderless true-color terminal workbench (Ratatui), OS-level Landlock LSM sandboxing, dynamic GBNF grammar constraints, and SQLite session persistence.

---

## Key Features

- **In-Process Inference**: Zero networking. It links with `llama.cpp` statically at compile time. Model loading is handled directly within the main executable.
- **GPU (Vulkan) / CPU Hybrid Offloading**: Direct Vulkan driver support for full/partial GPU offloading.
- **Layered TOML Configuration**: Overrides stack: Built-in Defaults $\rightarrow$ Global Config (`~/.config/dohee/config.toml`) $\rightarrow$ Project Local Config (`.dohee.toml`) $\rightarrow$ CLI Flags.
- **Landlock LSM Sandboxing**: Spawns terminal commands inside a child-process sandbox, restricting filesystem reads to read-only and writes exclusively to the project workspace directory.
- **Structured GBNF Grammar**: Constrains raw model token selection to prevent malformed or half-written tool XML tags.
- **SQLite Session Store**: Automatically records and lists conversational histories, allowing you to resume active agent sessions after a process interrupt.
- **Premium Codex-Style TUI**:
  - **Borderless Engineering Workbench**: Designed with a sleek, borderless layout and modern Catppuccin Mocha colors.
  - **Tab Completion & Suggestions**: Cycle through slash command matches by pressing `Tab` inside the composer. Suggestions are rendered directly in the input bar.
  - **Component-Based MVU Engine**: Built with a decoupled component architecture (Header, Transcript, Composer, StatusBar, Popups) with an RAII terminal guard.
  - **Vim Navigation & Visual Mode**: Navigate message transcripts using Vim bindings (`j`/`k`, `g`/`G`) and visual select/yank text directly to your clipboard.

---

## How to Build & Run

### Prerequisites
- **Rust Toolchain** (Rust 1.75+ or Cargo).
- **GPU Drivers & SDKs**:
  - **CUDA Toolkit** (Recommended for NVIDIA GPUs for maximum performance).
  - **Vulkan Drivers & Headers** (Recommended for AMD/Intel GPUs or vendor-agnostic setups).

### 1. Build from Source
Build the workspace binary targeting your hardware:
```bash
# Compile with CUDA acceleration (NVIDIA GPUs - Recommended)
cargo build --release --workspace --features cuda

# Compile with Vulkan acceleration (AMD/Intel/Cross-platform GPUs)
cargo build --release --workspace --features vulkan

# Compile for CPU-only execution
cargo build --release --workspace
```

### 2. Configure Your Settings
Create a global config file in `~/.config/dohee/config.toml` or a local one in your project directory called `.dohee.toml`:
```toml
# Example Config
backend = "cuda"          # Use "cuda", "vulkan", or "cpu"
ctx_size = 2048
temperature = 0.2
seed = 1234
gpu_layers = 99
sandbox_policy = "WorkspaceWrite"
chat_template = "chatml" # Custom template .jinja path, or builtin name (e.g. "chatml")
```

---

## CLI Usage Reference

### Commands Overview
- `dohee [prompt]`: Launches the interactive Terminal User Interface (TUI). If a prompt is supplied, it starts the session automatically. Specify `--chat-template <path/builtin>` to override templates.
- `dohee run <prompt>`: Executes a single-shot prompt in the terminal, showing streamed outputs and tool calls. Specify `--chat-template <path/builtin>` to override templates.
- `dohee config-show`: Renders the active merged configurations.
- `dohee models-list`: Lists GGUF files in your local `models/` directory.
- `dohee sessions-list`: Displays session IDs and timestamps from the SQLite database.
- `dohee sessions-resume <id>`: Reloads the session history and resumes the agent loop.
- `dohee doctor`: Performs system hardware, model path, Vulkan compatibility, and sandbox support diagnostics.
- `dohee index [path]`: Asynchronously builds or refreshes the workspace AST symbol index (defaults to the current directory).

---

## Interactive TUI Slash Commands

Inside the chat composer, type `/` to access built-in tools:
* `/help`                  - Show TUI helper documentation.
* `/config`                - Show current merged configurations.
* `/config set <key> <val>`- Update configuration parameters (e.g. `temperature`, `seed`, `ctx_size`, `sandbox`) dynamically.
* `/models`                - List GGUF models in your local `models/` directory.
* `/sessions`              - List past chat sessions from the database.
* `/resume <session_id>`   - Resume a past chat session.
* `/doctor`                - Run diagnostics (acceleration support, sandbox check, config values).
* `/index`                 - Build or refresh the workspace AST symbol index in the background.

---

## Crate Layout & Architecture

- `dohee-cli`: Main executable binary entrypoint.
- `dohee-prompt`: Dynamic template-driven prompt renderer utilizing cached compiled MiniJinja environments.
- `dohee-core`: Handles the agent state machine and turn loops.
- `dohee-infer`: Links `llama-cpp-2` statically and implements Detokenization/Sampling.
- `dohee-tools`: Unified `Tool` trait and built-in filesystem read/write/edit/shell tools.
- `dohee-sandbox`: Linux Landlock LSM file system isolation rules.
- `dohee-context`: Pruning strategies and token length calculators.
- `dohee-store`: SQLite session logs persistence.
- `dohee-tui`: Interactive drawing loops using `ratatui` and `crossterm` designed in a Component-based MVU pattern.
