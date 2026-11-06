# Qwen Code

Craft Anything can use the installed Qwen Code CLI (`qwen`) as a coding agent for chats, analysis
and builds. Messages, thinking summaries, tool calls and their results arrive as chat events, the
same way Claude Code's and Codex's do. Qwen Code's own sign-in (its Qwen/Qwen account, Coding Plan
or another provider) is used; Craft Anything never asks for or stores an API key.

## Setup

1. Install Qwen Code using its
   [installation guide](https://github.com/QwenLM/qwen-code#installation). The standalone
   installer puts it in `~/.local/bin` on macOS and Linux, and in
   `%LOCALAPPDATA%\qwen-code\bin` on Windows; both are searched even when the app was started
   before `PATH` was updated.
2. In your own terminal, run `qwen` once, then `/auth` to sign in or choose a provider. Qwen Code
   keeps its credentials. A headless run that isn't signed in stops with the reason instead of
   waiting.
3. In Settings → AI, detect and add **Qwen Code**, then choose it for chat or **Build with this**.
   If it is installed elsewhere, use **Choose...** to select `qwen`.

The equivalent `config.toml` entry is:

```toml
[[ai.backends]]
name = "Qwen Code"
kind = "agent_cli"
model = "qwen-code"
enabled = true
```

The agent id is `qwen-code`; `qwen` is accepted as an alias.

## Chats and builds

Each message is one headless run:

```text
qwen -p "" --output-format stream-json --include-partial-messages \
  --approval-mode yolo --mcp-config <inline JSON> --exclude-tools …
```

The message itself goes on stdin (`-p ""` appends stdin to it), so a long prompt doesn't hit
Windows' command-line limit. `--output-format stream-json` gives partial answer text, complete
assistant messages (text, thinking and tool calls), tool results, a session id to continue, and
usage. Craft Anything passes the saved session back with `--resume`.

Craft Anything's MCP server is supplied for that run only through `--mcp-config`, with `trust`
set: nothing in `~/.qwen/settings.json` is rewritten. The server enforces Clean room or Deep mode
and the build approval policy.

## Permissions

A headless Qwen run cannot answer a confirmation, so the adapter asks for `--approval-mode yolo`
unless the backend already chose an approval mode or `--yolo`. Safety then comes from refusing
tools by name:

- `ask_user_question` is always excluded; nothing can answer it.
- A chat or analysis run also excludes Qwen's own shell, edit, write, notebook, web, subagent,
  schedule, team and memory tools. A Deep-mode chat keeps its read tools (`read_file`,
  `grep_search`, `glob`, `list_directory`).
- A Clean room chat excludes the read tools too, so the original is reached only through Craft
  Anything's tools.
- A builder keeps its own file and shell tools, like the other agent CLIs. In Clean room its
  reads are not checked by the approval policy; this is the same gap the roadmap records for every
  agent that never asks us (R2.2).

## Models

The adapter does not fetch a Qwen catalogue. Qwen Code uses its own configured model; setting
`cli_model` passes `-m <model>` for the run. Thinking/reasoning options are Qwen's own settings,
not a separate Craft Anything control.

## Verification and limits

- Unit tests cover argument construction and exclusions in each access mode, partial-message
  deduplication, tool calls and results, session ids, usage, subagent filtering, denied Craft
  Anything tools and typed failures.
- Live on Windows with Qwen Code 0.26.9: detection found the standalone install, a headless run
  started, and its `result` event was parsed as an error because Qwen's OAuth free tier ended on
  2026-04-15. The message and the `/auth` fix were shown; no live answer or build was completed on
  this account. Chat with a Coding Plan, Qwen account or other provider has not been run here.
- Qwen's result envelope currently carries only an error message, not a typed `error.type`; an
  unclassified failure is therefore not retried rather than guessed from English. A future CLI
  that sends `error.type` is read directly.
- `--include-partial-messages` is requested but not required: if an older CLI ignores it, the
  complete assistant message is shown instead.
- The `qwen --acp` ACP mode remains available as an opt-in transport; the native stream-json
  adapter above is the default.