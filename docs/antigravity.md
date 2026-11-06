# Google Antigravity

Craft Anything can use Google Antigravity's CLI (`agy`) as a coding agent for chats, analysis and
builds. Antigravity runs over the Agent Client Protocol (ACP), so its answers, thinking, tool calls
and permission requests arrive as chat events. Sign-in stays in Antigravity: a Google account, a
Gemini Enterprise account, or another provider the person chose there. Craft Anything does not ask
for an API key or read Antigravity's credential store.

## Setup

1. Install the Antigravity CLI with Google's installer:

   ```sh
   # macOS, Linux, Googlebook
   curl -fsSL https://antigravity.google/cli/install.sh | bash
   ```

   On Windows, run the PowerShell installer from the same
   [CLI guide](https://antigravity.google/docs/getting-started?tab=cli). It places `agy.exe` in
   `%LOCALAPPDATA%\agy\bin`, which Craft Anything searches.

2. In your own terminal, run `agy` once and complete the Google sign-in it asks for (or choose the
   provider you want). A headless run without cached credentials stops with an `authentication
   required` error rather than hanging.

3. In Settings → AI, detect and add **Antigravity**, then choose it for chat or **Build with this**.
   If it is installed elsewhere, use **Choose...** to select `agy`.

The equivalent `config.toml` entry is:

```toml
[[ai.backends]]
name = "Antigravity"
kind = "agent_cli"
model = "antigravity"
enabled = true
```

The agent id is `antigravity`; `agy` is accepted as an alias.

## Why ACP

Antigravity's native print mode (`agy -p`) reads MCP servers from the person's global
`~/.gemini/config/mcp_config.json`. Adding Craft Anything there would change their setup and would
be shared by concurrent runs. ACP instead takes the tool server per session (`session/new`), which
keeps the person's configuration untouched, works the same locally, and routes permission requests
through Craft Anything's approval policy. Antigravity is therefore ACP-only:

- `acp = false` is refused, with the way to fix it, rather than falling back to a run without
  Craft Anything's tools.
- An SSH host is refused: the ACP process is local.
- Extra `args` are refused because they could auto-approve tools (for example
  `--dangerously-skip-permissions`) and bypass approvals. Use the model picker instead.

## Models

A fresh ACP session reports the models this account can use, for example
`gemini-3.8-flash-high`, `gemini-3.8-flash-medium`, `gemini-3.8-flash-low`, `claude-sonnet-4-6`,
`claude-opus-4-6-thinking` and `gpt-oss-120b-medium`. Settings → AI offers exactly that list; a
choice is checked against a live session before a run. The model names already carry their thinking
level, so there is no separate depth control. The chosen slug is passed as `--model <slug>` before
the `acp` subcommand.

With no choice, Antigravity uses its own saved model. The Settings entry shows **Antigravity
default**.

## Tools and permissions

- A chat or build opens a session with Craft Anything's MCP server. Antigravity names its tools
  `mcp_craft-anything_<tool>`; the feed and the approval policy see the tool's own name.
- In a build, Antigravity's edit, command and web requests come to the project's approval policy
  (inbox and Auto mode), the same as Claude Code's and Copilot's. Read requests that Antigravity
  doesn't ask about are not held by our policy; this is the Clean room gap the roadmap records for
  agents that never ask us (R2.2).
- A chat in Clean room uses the read-only approver: Antigravity's writes, commands and fetches are
  refused. A Deep-mode chat may read the project it is discussing.
- A question about a build opens a fresh session with no MCP server and does not load or change the
  builder's conversation.
- Antigravity advertises `session/load`; a saved conversation is continued when it still exists on
  this computer. If it doesn't, a new session starts rather than failing the message.

## Verification and limits

- Live on Windows with Antigravity CLI 1.5.2 (Google AI Plus), driven exactly as the adapter does:
  `agy --model gemini-3.8-flash-low acp`, `initialize`, `session/new` carrying a synthetic stdio
  MCP server, then `session/prompt`. Antigravity listed the server's `hello` tool as
  `mcp_craft-test_hello`, called it, and returned its text. A second prompt asked to run a shell
  command: Antigravity sent `session/request_permission` with `kind: "execute"`, and the denied
  decision produced a failed tool call while the run still ended cleanly.
- Unit tests cover ACP arguments, the ACP registry rows, the model catalogue, refusal of extra
  arguments and native runs, sign-in guidance, and the `mcp_craft-anything_*` tool-name mapping.
- No authenticated build, full chat turn through the Craft Anything UI, or cross-computer session
  resume was run. The model list is account-specific; other accounts can differ.
- ACP reports no attributable usage totals here, so the activity stream leaves tokens and cost
  unknown instead of inventing them. Consult Antigravity (`/usage`) for credits and quota.
- The CLI used for verification was downloaded to a temporary folder, not installed; the app will
  offer Antigravity once the installer above is run.