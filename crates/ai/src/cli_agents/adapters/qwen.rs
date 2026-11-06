//! Qwen Code (`qwen --output-format stream-json`), one headless run per message. Its stream is
//! Anthropic-shaped, so answers, thinking, tool calls, usage and the session to continue arrive as
//! [`AgentEvent`]s, the way Claude Code's do.
//!
//! The message goes in on stdin: `-p ""` appends stdin to it, so a long one stays under Windows'
//! command-line limit (#402). Our tool server is handed over per run with `--mcp-config`, an inline
//! JSON document, so nothing in the person's own Qwen settings changes.
//!
//! Qwen asks before changing a file or running a command, and a headless run has nobody to answer.
//! `--approval-mode yolo` approves everything except `ask_user_question`, which is excluded; what a
//! run must not do is refused by name with `--exclude-tools`. Sign-in stays in Qwen Code itself
//! (its Google/Qwen account or its own provider settings); we never see the credentials.

use super::*;

pub struct QwenCode;

/// Qwen's own tools a run that isn't a build must not use: they change files, run something, reach
/// the web, start subagents, or drive Qwen's schedules, teams and memory. A name the installed
/// version doesn't know is ignored, so this may list tools a newer CLI added.
const CHANGING: &[&str] = &[
    "run_shell_command",
    "exec",
    "monitor",
    "edit",
    "write_file",
    "notebook_edit",
    "web_fetch",
    "web_search",
    "image_gen",
    "agent",
    "skill",
    "workflow",
    "artifact",
    "record_artifact",
    "record_source",
    "report_findings",
    "autofix_pr",
    "cron_create",
    "cron_delete",
    "team_create",
    "team_delete",
    "request_shutdown",
    "loop_wakeup",
    "send_message",
    "create_sub_session",
    "task_create",
    "task_update",
    "task_stop",
    "enter_worktree",
    "exit_worktree",
    "enter_plan_mode",
    "exit_plan_mode",
    "propose_goal",
    "update_goal",
    "todo_write",
    "save_memory",
    "manage_memory",
];

/// Qwen's own tools that read files, refused outright in Clean room (#564) so a chat reaches the
/// original only through our tool server. In Deep mode a chat keeps them: it may read the project
/// it is talking about.
const READING: &[&str] = &["read_file", "grep_search", "glob", "list_directory", "zoom_image", "read_mcp_resource", "lsp", "search_memory", "display_image"];

impl AgentAdapter for QwenCode {
    fn ids(&self) -> &'static [&'static str] {
        &["qwen-code", "qwen"]
    }

    fn prompt_on_stdin(&self) -> bool {
        true
    }

    fn stdin_text(&self, preamble: &str, prompt: &str) -> String {
        format!("{preamble}{prompt}")
    }

    fn check_tool_server(&self, stop: &CancelToken) -> Result<(), ToolError> {
        cli_can_start(craft_anything_platform::self_cli_path().as_deref(), stop)
    }

    fn args(&self, cx: &ArgsCx) -> Vec<String> {
        let mcp_json = json!({"mcpServers": {MCP_NAME: {
            "command": cx.mcp.command,
            "args": cx.mcp.args,
            // Our tools ask the person themselves where it matters, so Qwen needn't gate them.
            "trust": true,
        }}})
        .to_string();
        let mut a = cx.options.args.clone();
        // An empty `-p` keeps the run headless; stdin is appended to it (see `stdin_text`).
        a.extend(["-p".into(), String::new(), "--output-format".into(), "stream-json".into(), "--include-partial-messages".into()]);
        if let Some(m) = &cx.options.cli_model {
            a.extend(["-m".into(), m.clone()]);
        }
        if let Some(r) = cx.resume {
            a.extend(["--resume".into(), r.to_string()]);
        }
        // Nothing can answer a confirmation in a headless run. A person's own choice of approval
        // mode or `--yolo` is kept; both together are refused by Qwen itself.
        let approval_set = a.iter().any(|x| x == "--approval-mode" || x.starts_with("--approval-mode=") || x == "--yolo" || x == "-y");
        if !approval_set {
            a.extend(["--approval-mode".into(), "yolo".into()]);
        }
        a.extend(["--mcp-config".into(), mcp_json]);
        let mut excluded: Vec<&str> = vec!["ask_user_question"];
        if !cx.builder {
            excluded.extend(CHANGING);
            if cx.reads_checked {
                excluded.extend(READING);
            }
        }
        a.push("--exclude-tools".into());
        a.extend(excluded.into_iter().map(str::to_string));
        a
    }

    fn required_flags(&self) -> (&'static [&'static str], &'static [&'static str]) {
        (&["--help"], &["--output-format", "stream-json"])
    }

    /// Plain `qwen <prompt>` is one-shot, but it prints text we can't turn into a chat, and an
    /// older CLI without `--output-format` may open its terminal interface instead. A failed
    /// flag check says so and tries the adapter anyway.
    fn plain_fallback(&self) -> bool {
        false
    }

    fn resumes(&self) -> bool {
        true
    }

    /// Qwen's stream-json: partial answers when the CLI sends them, then the complete assistant
    /// message (tool calls, thinking, and text the partials didn't already show), tool results,
    /// and a final `result` with usage and a failed turn in plain words.
    fn parse(&self, p: &mut StreamParser, v: &Value) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        if let Some(id) = v.get("session_id").and_then(Value::as_str) {
            p.session_id = Some(id.to_string());
        }
        // A subagent's own messages stay out of the feed: its `agent` call reports the outcome.
        if v.get("parent_tool_use_id").is_some_and(|id| !id.is_null()) {
            return out;
        }
        match s("type").as_str() {
            "system" if matches!(s("subtype").as_str(), "session_start" | "init") => {
                // Some releases name the servers they started; a failed Craft Anything server
                // means no tool of ours can run, which the run must say rather than discover later.
                let failed = v.get("mcp_servers").and_then(Value::as_array).and_then(|servers| {
                    servers.iter().find(|server| {
                        server.get("name").and_then(Value::as_str) == Some(MCP_NAME)
                            && matches!(server.get("status").and_then(Value::as_str), Some("failed" | "needs-auth"))
                    })
                });
                if let Some(server) = failed {
                    p.failure = Some(TurnFailure::new(FailureKind::ToolsUnavailable));
                    let status = server.get("status").and_then(Value::as_str).unwrap_or_default();
                    out.push(AgentEvent::Error {
                        message: format!("Craft Anything's tools did not start in Qwen Code (server status: {status})."),
                        remediation: Some(
                            "Check the Craft Anything CLI and the agent failure log, then start a new build attempt after fixing the tool connection.".into(),
                        ),
                        log: None,
                    });
                }
            }
            "stream_event" => {
                let delta = v.pointer("/event/delta");
                let kind = delta.and_then(|d| d.get("type")).and_then(Value::as_str).unwrap_or_default();
                if kind == "text_delta"
                    && let Some(text) = delta.and_then(|d| d.get("text")).and_then(Value::as_str).filter(|text| !text.is_empty())
                {
                    p.streamed_text = true;
                    out.push(AgentEvent::TextDelta { text: text.to_string() });
                }
                // Thinking is summarised once from the completed block, not per fragment.
            }
            "assistant" => {
                for b in v.pointer("/message/content").and_then(Value::as_array).cloned().unwrap_or_default() {
                    match b.get("type").and_then(Value::as_str).unwrap_or_default() {
                        // With partial messages the same text already arrived in fragments.
                        "text" if !p.streamed_text => {
                            out.push(AgentEvent::TextDelta { text: b.get("text").and_then(Value::as_str).unwrap_or_default().to_string() })
                        }
                        "thinking" => {
                            if let Some(text) = b.get("thinking").and_then(Value::as_str).filter(|text| !text.trim().is_empty()) {
                                out.push(AgentEvent::Status { text: summary_of(text) });
                            }
                        }
                        "tool_use" => {
                            let id = b.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                            let name = b.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
                            out.push(p.started(&id, &name, b.get("input").cloned().unwrap_or(Value::Null)));
                        }
                        _ => {}
                    }
                }
            }
            "user" => {
                for b in v.pointer("/message/content").and_then(Value::as_array).cloned().unwrap_or_default() {
                    if b.get("type").and_then(Value::as_str) == Some("tool_result") {
                        let id = b.get("tool_use_id").and_then(Value::as_str).unwrap_or_default().to_string();
                        let err = b.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                        let text = content_text(b.get("content").unwrap_or(&Value::Null));
                        out.push(p.finished(&id, !err, &text));
                    }
                }
            }
            "result" => {
                p.saw_result = true;
                if let Some(u) = v.get("usage") {
                    p.input_tokens = u
                        .get("input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0)
                        .saturating_add(u.get("cache_read_input_tokens").and_then(Value::as_u64).unwrap_or(0));
                    p.output_tokens = u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
                }
                let denied: Vec<String> = v
                    .get("permission_denials")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|d| d.get("tool_name").and_then(Value::as_str))
                    .filter(|name| name.starts_with(&format!("mcp__{MCP_NAME}__")))
                    .map(str::to_string)
                    .collect();
                if !denied.is_empty() {
                    p.failure = Some(TurnFailure::new(FailureKind::ToolsUnavailable));
                    out.push(AgentEvent::Error {
                        message: format!("Qwen Code refused to run Craft Anything's tools ({}).", denied.join(", ")),
                        remediation: Some("Check the Craft Anything CLI and this agent's settings, then start a new attempt.".into()),
                        log: None,
                    });
                } else if v.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
                    let failure = qwen_failure(v.pointer("/error/type").and_then(Value::as_str), &s("subtype"));
                    p.failure = Some(failure);
                    let why = v.pointer("/error/message").and_then(Value::as_str).unwrap_or("the run failed");
                    let remediation = match failure.kind {
                        FailureKind::Authentication => self.sign_in_fix(),
                        FailureKind::Billing => Some("Check the account's credits, plan and spending limit before trying again.".into()),
                        FailureKind::Configuration => Some("Check Qwen Code's model and settings before trying again.".into()),
                        // The free OAuth tier's retirement, and most setup problems, arrive as a
                        // plain message: send the person to the CLI's own sign-in screen.
                        FailureKind::Unknown => Some("Open `qwen` in a terminal and run `/auth` to check its sign-in, model and plan.".into()),
                        _ => None,
                    };
                    out.push(AgentEvent::Error { message: format!("Qwen Code stopped: {why}"), remediation, log: None });
                }
            }
            _ => {}
        }
        out
    }

    fn sign_in_fix(&self) -> Option<String> {
        Some("Sign in to Qwen Code with your account: open a terminal and run `qwen`, then try again. Qwen Code keeps its credentials.".into())
    }
}

/// A failed Qwen run's typed reason. The current CLI gives its result envelope only a message, so
/// an unclassified failure stays [`FailureKind::Unknown`] (not retried) rather than guessing from
/// the words in it; a newer CLI's `error.type` is read when present.
fn qwen_failure(kind: Option<&str>, subtype: &str) -> TurnFailure {
    TurnFailure::new(match kind.unwrap_or_default() {
        "authentication_error" | "authentication_failed" | "permission_error" | "invalid_api_key" => FailureKind::Authentication,
        "billing_error" | "insufficient_quota" | "quota_exceeded" => FailureKind::Billing,
        "rate_limit_error" | "rate_limit_exceeded" | "rate_limit" => FailureKind::RateLimited,
        "overloaded_error" | "api_error" | "server_error" | "timeout" => FailureKind::Transient,
        "invalid_request_error" | "model_not_found" | "run_budget_exceeded" => FailureKind::Configuration,
        _ if subtype == "error_max_turns" => FailureKind::Configuration,
        _ => FailureKind::Unknown,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(builder: bool, reads_checked: bool, options: &BackendOptions) -> Vec<String> {
        let mcp = McpCommand { command: "craft-anything-cli".into(), args: vec!["mcp".into()], notes: None };
        QwenCode.args(&ArgsCx { options, prompt: "make a game", preamble: "PRE ", mcp: &mcp, resume: None, builder, reads_checked, question: false })
    }

    fn excluded(argv: &[String]) -> Vec<String> {
        let at = argv.iter().position(|a| a == "--exclude-tools").expect("exclude-tools");
        argv[at + 1..].to_vec()
    }

    #[test]
    fn the_prompt_goes_on_stdin_and_the_tool_server_travels_with_the_run() {
        let argv = args(true, false, &BackendOptions::default());
        assert_eq!(argv[..3], ["-p", "", "--output-format"]);
        assert!(argv.contains(&"--include-partial-messages".to_string()));
        let mcp = argv.iter().position(|a| a == "--mcp-config").expect("mcp config");
        let config: Value = serde_json::from_str(&argv[mcp + 1]).unwrap();
        assert_eq!(config["mcpServers"][MCP_NAME]["command"], "craft-anything-cli");
        assert_eq!(config["mcpServers"][MCP_NAME]["args"], json!(["mcp"]));
        assert_eq!(config["mcpServers"][MCP_NAME]["trust"], true);
        assert_eq!(argv.iter().filter(|a| *a == "--approval-mode").count(), 1);
        assert!(argv.windows(2).any(|pair| pair == ["--approval-mode", "yolo"]));
        assert_eq!(excluded(&argv), ["ask_user_question"]);
        assert!(AgentAdapters::built_in().for_agent("qwen").ids().contains(&"qwen-code"));
        assert!(AgentAdapters::built_in().for_agent("qwen-code").ids().contains(&"qwen"));
        assert!(QwenCode.resumes() && !QwenCode.plain_fallback());
        assert!(QwenCode.prompt_on_stdin());
    }

    #[test]
    fn a_chat_can_only_read_and_a_clean_room_chat_cannot_read_the_original() {
        let deep = excluded(&args(false, false, &BackendOptions::default()));
        for tool in ["run_shell_command", "edit", "write_file", "monitor", "notebook_edit", "agent", "web_fetch"] {
            assert!(deep.contains(&tool.to_string()), "{tool} is switched off in a chat");
        }
        assert!(!deep.contains(&"read_file".to_string()), "a Deep chat reads the project it discusses");
        assert!(!deep.contains(&"grep_search".to_string()));

        let clean = excluded(&args(false, true, &BackendOptions::default()));
        for tool in ["read_file", "grep_search", "glob", "list_directory"] {
            assert!(clean.contains(&tool.to_string()), "{tool} is switched off in Clean room");
        }
        assert!(clean.contains(&"run_shell_command".to_string()));
    }

    #[test]
    fn a_builder_keeps_its_tools_and_keeps_a_persons_own_approval_choice() {
        let argv = args(true, true, &BackendOptions::default());
        assert_eq!(excluded(&argv), ["ask_user_question"], "a builder keeps its file and shell tools");
        let options = BackendOptions { cli_model: Some("qwen3-coder-plus".into()), args: vec!["--approval-mode".into(), "auto".into()], ..Default::default() };
        let argv = args(true, false, &options);
        assert_eq!(argv.iter().filter(|a| *a == "--approval-mode").count(), 1, "not twice");
        assert!(argv.windows(2).any(|pair| pair == ["--approval-mode", "auto"]));
        assert!(argv.windows(2).any(|pair| pair == ["-m", "qwen3-coder-plus"]));
    }

    #[test]
    fn answers_tool_calls_and_usage_from_the_anthropic_shaped_stream() {
        let mut p = StreamParser::new(Arc::new(QwenCode));
        let lines = [
            json!({"type":"system","subtype":"session_start","session_id":"s-1","mcp_servers":[{"name":MCP_NAME,"status":"connected"}]}),
            json!({"type":"stream_event","session_id":"s-1","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello "}}}),
            json!({"type":"stream_event","session_id":"s-1","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"there"}}}),
            json!({"type":"assistant","session_id":"s-1","message":{"content":[{"type":"text","text":"Hello there"},{"type":"thinking","thinking":"It is a greeting."},{"type":"tool_use","id":"tool-1","name":"mcp__craft-anything__core_finding_add","input":{"text":"it greets"}}]}}),
            json!({"type":"user","session_id":"s-1","message":{"content":[{"type":"tool_result","tool_use_id":"tool-1","content":[{"type":"text","text":"added ev_0123456789abcdef"}]}]}}),
            json!({"type":"result","session_id":"s-1","subtype":"success","is_error":false,"usage":{"input_tokens":10,"cache_read_input_tokens":3,"output_tokens":5}}),
        ];
        let events: Vec<_> = lines.iter().flat_map(|v| p.line(&v.to_string())).collect();
        let text: String = events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::TextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "Hello there", "partial fragments and the complete message are not shown twice");
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Status { text } if text.contains("greeting"))));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolStarted { tool, .. } if tool == "core_finding_add")));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolFinished { ok: true, tool, .. } if tool == "core_finding_add")));
        assert_eq!((p.session_id.as_deref(), p.input_tokens, p.output_tokens), (Some("s-1"), 13, 5));
        assert!(p.saw_result);
    }

    #[test]
    fn the_complete_message_is_used_when_no_partial_messages_arrive() {
        let mut p = StreamParser::new(Arc::new(QwenCode));
        let events = p.line(&json!({"type":"assistant","message":{"content":[{"type":"text","text":"An answer"}]}}).to_string());
        assert!(matches!(events.as_slice(), [AgentEvent::TextDelta { text }] if text == "An answer"));
    }

    #[test]
    fn a_failed_result_is_typed_and_says_how_to_sign_in() {
        let mut p = StreamParser::new(Arc::new(QwenCode));
        let events = p.line(
            &json!({"type":"result","subtype":"error_during_execution","is_error":true,"error":{"type":"authentication_error","message":"login required"},"usage":{}})
                .to_string(),
        );
        assert!(
            matches!(events.as_slice(), [AgentEvent::Error { message, remediation: Some(fix), .. }] if message.contains("login required") && fix.contains("qwen"))
        );
        assert_eq!(p.failure, Some(TurnFailure::new(FailureKind::Authentication)));
        assert!(!p.failure.unwrap().retryable());
        assert_eq!(qwen_failure(None, "error_during_execution").kind, FailureKind::Unknown, "not guessed from English");
        assert!(qwen_failure(Some("rate_limit_error"), "error_during_execution").retryable());
        assert_eq!(qwen_failure(None, "error_max_turns").kind, FailureKind::Configuration);
        let mut p = StreamParser::new(Arc::new(QwenCode));
        let events = p.line(&json!({"type":"result","subtype":"error_during_execution","is_error":true,"error":{"message":"OAuth tier retired"}}).to_string());
        assert!(
            matches!(events.as_slice(), [AgentEvent::Error { remediation: Some(fix), .. }] if fix.contains("/auth")),
            "an unclassified setup failure points at Qwen's own sign-in"
        );
    }

    #[test]
    fn a_denied_craft_anything_tool_is_reported_as_our_tools_missing() {
        let mut p = StreamParser::new(Arc::new(QwenCode));
        p.line(
            &json!({"type":"result","subtype":"success","is_error":false,"permission_denials":[{"tool_name":"mcp__craft-anything__core_finding_add"}]})
                .to_string(),
        );
        assert_eq!(p.failure, Some(TurnFailure::new(FailureKind::ToolsUnavailable)));
    }

    #[test]
    fn a_failed_tool_server_is_reported() {
        let mut p = StreamParser::new(Arc::new(QwenCode));
        let events =
            p.line(&json!({"type":"system","subtype":"session_start","session_id":"s-2","mcp_servers":[{"name":MCP_NAME,"status":"failed"}]}).to_string());
        assert!(matches!(events.as_slice(), [AgentEvent::Error { message, .. }] if message.contains("did not start")));
        assert_eq!(p.failure, Some(TurnFailure::new(FailureKind::ToolsUnavailable)));
        assert_eq!(p.session_id.as_deref(), Some("s-2"));
    }

    #[test]
    fn a_subagents_messages_stay_out_of_the_feed() {
        let mut p = StreamParser::new(Arc::new(QwenCode));
        let events = p.line(&json!({"type":"assistant","parent_tool_use_id":"agent-1","message":{"content":[{"type":"text","text":"internal"}]}}).to_string());
        assert!(events.is_empty());
    }
}
