//! Google Antigravity (`agy`), driven over ACP with our tool server rather than its own file and
//! shell tools. Sign-in, credentials and session storage stay in Antigravity itself: a person signs
//! in once with their Google account (or enterprise account) in the CLI, and no API key is
//! involved unless they choose one there.
//!
//! The native `agy -p` run can't be handed a per-run MCP server without changing the person's
//! global `mcp_config.json`, so Antigravity runs the way Copilot does: one ACP process, `session/new`
//! carrying Craft Anything's server, permission requests decided by our approval policy, and
//! `session/load` to continue a saved conversation.

use super::*;

pub struct Antigravity;

impl AgentAdapter for Antigravity {
    fn ids(&self) -> &'static [&'static str] {
        &["antigravity", "agy"]
    }

    /// The native runner has no way to add our tool server to one run, and a build without our
    /// tools can't do the job. An explicit `acp = false` or an SSH host is refused with the fix,
    /// not run without tools.
    fn requires_acp(&self) -> bool {
        true
    }

    /// The native runner refuses `requires_acp` before it asks for arguments.
    fn args(&self, _cx: &ArgsCx) -> Vec<String> {
        Vec::new()
    }

    /// The models this account can run come from a fresh ACP session, not a fixed list.
    fn has_model_catalog(&self) -> bool {
        true
    }

    fn model_catalog(
        &self,
        config: &BackendConfig,
        options: &BackendOptions,
        directory: Option<&Path>,
        stop: &Stop,
    ) -> Result<Vec<models::AgentModel>, ToolError> {
        let metadata = super::super::acp_model_metadata(config, options, directory, stop)?;
        Ok(catalogue(&metadata))
    }

    fn validate_model(&self, config: &BackendConfig, options: &BackendOptions, directory: Option<&Path>, stop: &Stop) -> Result<(), ToolError> {
        stop.check()?;
        self.validate_options(options)?;
        if options.cli_model.is_none() {
            return Ok(());
        }
        models::validate(options, &self.model_catalog(config, options, directory, stop)?)
    }

    fn model_choices(&self, config: &BackendConfig, options: &BackendOptions) -> Result<Vec<String>, ToolError> {
        let metadata = super::super::acp_model_metadata(config, options, None, &Stop::default())?;
        Ok(catalogue(&metadata).into_iter().map(|model| model.id).collect())
    }

    /// The model names Antigravity reports already carry their thinking level
    /// (`gemini-3.8-flash-medium`), so there is no separate depth setting to offer.
    fn supports_model_tuning(&self) -> bool {
        false
    }

    fn model_in_use(&self, options: Option<&BackendOptions>) -> Option<String> {
        Some(options.and_then(|options| options.cli_model.clone()).unwrap_or_else(|| "Antigravity default".into()))
    }

    /// A session can answer a question about a build: ACP permission requests go through the
    /// read-only approver, so its writes, commands and fetches are refused.
    fn answers_questions(&self) -> bool {
        true
    }

    fn plain_fallback(&self) -> bool {
        false
    }

    fn check_tool_server(&self, stop: &CancelToken) -> Result<(), ToolError> {
        cli_can_start(craft_anything_platform::self_cli_path().as_deref(), stop)
    }

    /// Startup arguments before `acp`: the chosen model must come before the subcommand.
    fn acp_options(&self, options: &BackendOptions, _access: Access) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(model) = options.cli_model.as_deref().map(str::trim).filter(|model| !model.is_empty()) {
            args.extend(["--model".into(), model.into()]);
        }
        args
    }

    /// Extra arguments could auto-approve tools (`--dangerously-skip-permissions`) and bypass the
    /// approval policy, so they're refused; settings the app understands are on the backend.
    fn validate_options(&self, options: &BackendOptions) -> Result<(), ToolError> {
        if options.args.is_empty() {
            Ok(())
        } else {
            Err(ToolError::invalid("Extra Antigravity arguments could bypass Craft Anything's approvals.")
                .with_remediation("Remove this backend's extra arguments. Choose its model in Settings → AI, and leave approvals to the app."))
        }
    }

    fn sign_in_fix(&self) -> Option<String> {
        Some("Sign in to Antigravity with your Google account: open a terminal and run `agy`, then try again. Antigravity keeps its credentials.".into())
    }
}

/// The selectable models a fresh session reports, in the CLI's own order. Duplicates and empty ids
/// are dropped: the running agent decides what this account may use.
fn catalogue(metadata: &super::super::acp::ModelMetadata) -> Vec<models::AgentModel> {
    let mut seen = std::collections::BTreeSet::new();
    metadata
        .models
        .iter()
        .filter(|id| !id.trim().is_empty() && seen.insert((*id).clone()))
        .map(|id| models::AgentModel { id: id.clone(), name: id.clone(), thinking: Vec::new() })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn antigravity_is_acp_only_and_never_runs_native() {
        assert!(Antigravity.requires_acp() && !Antigravity.plain_fallback());
        assert!(Antigravity.answers_questions());
        assert!(AgentAdapters::built_in().for_agent("antigravity").ids().contains(&"antigravity"));
        assert!(AgentAdapters::built_in().for_agent("agy").ids().contains(&"agy"));
        assert_eq!(super::super::acp::acp_args("antigravity"), Some(["acp"].as_slice()));
        assert_eq!(super::super::acp::acp_args("agy"), Some(["acp"].as_slice()));
    }

    #[test]
    fn the_persons_model_goes_before_the_acp_subcommand_and_extra_arguments_are_refused() {
        let options = BackendOptions { cli_model: Some("gemini-3.8-flash-medium".into()), ..Default::default() };
        assert_eq!(Antigravity.acp_options(&options, Access::Build), ["--model", "gemini-3.8-flash-medium"]);
        assert!(Antigravity.validate_options(&options).is_ok());
        let extra = BackendOptions { args: vec!["--dangerously-skip-permissions".into()], ..Default::default() };
        assert_eq!(Antigravity.validate_options(&extra).unwrap_err().code, craft_anything_evidence::ErrorCode::BadInput);
    }

    #[test]
    fn the_catalogue_keeps_the_sessions_order_and_drops_duplicates() {
        let metadata = super::super::acp::ModelMetadata {
            models: vec!["gemini-3.8-flash-high".into(), "claude-sonnet-4-6".into(), "gemini-3.8-flash-high".into(), " ".into()],
            current_model: "gemini-3.8-flash-high".into(),
            config_options: Vec::new(),
        };
        let models = catalogue(&metadata);
        assert_eq!(models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(), ["gemini-3.8-flash-high", "claude-sonnet-4-6"]);
        assert!(models.iter().all(|model| model.thinking.is_empty()));
    }

    #[test]
    fn signing_in_stays_in_antigravity() {
        let fix = Antigravity.sign_in_fix().unwrap();
        assert!(fix.contains("agy") && fix.contains("Google") && !fix.to_ascii_lowercase().contains("api key"));
    }
}
