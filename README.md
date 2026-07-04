# Dohee (도회) - Local AI Coding Agent

Dohee (도회) is a lightweight, high-performance local AI coding agent written in Rust. It is custom-built to interact with local LLMs running via `llama.cpp` or `Ollama`. 

Dohee can autonomously list directories, read files, write files, and run terminal commands in your workspace, prompting you for permission before executing any action.

## Features

- **Automated llama-server Lifecycle**: Pass a local GGUF model path and Dohee will start the server, wait for it to load the model, run your chat session, and terminate the server when you exit.
- **CPU/GPU Offloading Controls**: Specify how many layers to offload to the GPU using `--gpu-layers` (defaults to `99` for full GPU offloading with CPU fallback) and threads with `--threads`.
- **Interactive REPL**: Streams completions directly to the terminal with ANSI colors.
- **Agentic Loop with Tool Execution**: Parses LLM outputs for special XML tags to perform actions:
  - `<list_dir>/path/to/dir</list_dir>`
  - `<read_file>/path/to/file</read_file>`
  - `<write_file path="/path/to/file">content</write_file>`
  - `<run_command>command</run_command>`
- **Token Memory Truncation**: Automatically monitors conversation history size and trims older turns to prevent context window overflow while preserving the system prompt.
- **Slash Commands**:
  - `/exit` - Exit the agent session.
  - `/clear` - Clear chat history (retaining system prompt).
  - `/history` - Display context length and estimated token stats.
  - `/system` - Print the active system prompt.
  - `/help` - Print the help menu.

---

## How to Build & Run

### Prerequisites

1. **Rust Toolchain**: Install Rust (Cargo).
2. **llama-server**: Ensure you have compiled `llama.cpp` (available in `/home/scott/Projects/agi/llama.cpp/server` or on your system `PATH`).
3. **Local GGUF Model**: You have models available in `/home/scott/Projects/agi/models/`.

### Run Dohee with Auto-launched local llama-server

You can run Dohee by pointing it to one of your local GGUF models. It will launch the backend server on port `8080` (with full GPU offloading by default):

```bash
cargo run --release -- --local-model /home/scott/Projects/agi/models/gemma-4-12b-it-Q4_K_M.gguf
```

Options:
- `-g <layers>`: Set number of GPU layers to offload (default `99`).
- `-t <threads>`: Set number of CPU threads (default `6`).
- `-p <port>`: Change local port (default `8080`).

### Run Dohee against an existing Ollama endpoint

If you already have Ollama running:

```bash
cargo run --release -- --ollama --model gemma
```

### Run Dohee against a manually launched server

If you manually started `llama-server` on `http://localhost:8080`:

```bash
cargo run --release -- --api-url http://localhost:8080/v1
```

---

## Command Line Arguments Reference

```
Usage: dohee [OPTIONS]

Options:
  -o, --ollama
          Use Ollama endpoint instead of llama.cpp server
      --api-url <API_URL>
          URL of the OpenAI-compatible API server [default: http://localhost:8080/v1]
  -m, --model <MODEL>
          Model name to request (required for Ollama, ignored by default llama.cpp server) [default: gemma-4-12b-it-Q4_K_M.gguf]
  -l, --local-model <LOCAL_MODEL>
          Path to a local GGUF model file to launch a background llama-server automatically
      --server-path <SERVER_PATH>
          Path to llama-server binary. If omitted, checks standard paths & Projects/agi/llama.cpp/server
  -g, --gpu-layers <GPU_LAYERS>
          Number of GPU layers to offload (used when spawning local server) [default: 99]
  -t, --threads <THREADS>
          Number of threads to use (used when spawning local server) [default: 6]
  -p, --port <PORT>
          Port to run the local server on [default: 8080]
      --max-turns <MAX_TURNS>
          Max consecutive tool loop iterations in a single turn [default: 10]
  -s, --system-prompt-file <SYSTEM_PROMPT_FILE>
          Custom system prompt file path
  -d, --cwd <CWD>
          Working directory for the workspace [default: .]
  -h, --help
          Print help
```
