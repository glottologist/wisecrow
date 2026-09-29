//! Subscription-backed provider: the Claude Code CLI in print mode.
//!
//! [`super::anthropic`] bills the Messages API against Console credits. This
//! provider runs the locally installed `claude` binary with `-p` instead,
//! authenticated by the long-lived OAuth token `claude setup-token` mints, so
//! the calls draw on a Claude subscription. The prompts, the answer shapes and
//! the parsing are the same either way; only the billing route differs.

use std::path::PathBuf;
use std::process::Stdio;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::LlmProvider;
use crate::errors::WisecrowError;

/// Default model when `WISECROW__LLM_MODEL` is unset.
pub const DEFAULT_MODEL: &str = "claude-sonnet-5";

/// The binary print mode runs, resolved on `PATH`.
const BINARY: &str = "claude";

/// Replaces Claude Code's own system prompt.
///
/// Claude Code is a coding assistant and says so at length: measured on one
/// identical call (2026-09-29) its default prompt costs 11 025 cache-creation
/// plus 3 397 cache-read input tokens against 5 116 for this one, and on a
/// subscription every token counts against the usage limits rather than a
/// bill. None of it is wanted either -- each prompt in [`super::prompts`]
/// states its own task and answer shape, and this provider hands the model no
/// tools to be guided about.
const SYSTEM_PROMPT: &str =
    "You answer data requests exactly as asked, with no preamble and no commentary.";

/// Runs prompts through the Claude Code CLI under a subscription.
pub struct ClaudeCliProvider {
    oauth_token: String,
    model: String,
    /// Configuration and working directories forced on the child.
    ///
    /// A subscription run cannot pass `--bare`, which is the flag that would
    /// skip auto-discovery: bare mode never reads `CLAUDE_CODE_OAUTH_TOKEN`.
    /// So the CLI loads whatever `CLAUDE.md`, hooks, settings and MCP servers
    /// it finds in the working directory and the configuration directory, and
    /// an answer would depend on the host it ran on -- a developer's checkout
    /// carries a `CLAUDE.md` that would edit every prompt. Both directories
    /// are ours alone and hold nothing, so no host content reaches a prompt.
    config_dir: PathBuf,
    working_dir: PathBuf,
}

impl ClaudeCliProvider {
    /// `oauth_token` is a `claude setup-token` token, carried in
    /// `WISECROW__LLM_API_KEY`.
    #[must_use]
    pub fn new(oauth_token: impl Into<String>, model: impl Into<String>) -> Self {
        let root = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("wisecrow")
            .join("claude-cli");
        Self {
            oauth_token: oauth_token.into(),
            model: model.into(),
            config_dir: root.join("config"),
            working_dir: root.join("cwd"),
        }
    }

    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }
}

/// The object `--output-format json` prints once the run has finished.
#[derive(Deserialize)]
struct CliResult {
    /// True for a run that failed inside the CLI, such as an expired token or
    /// a spent usage limit. Such a run still exits 0 (measured 2026-09-29 on
    /// 2.1.284, which answered `Not logged in · Please run /login` that way),
    /// so the exit status alone would let an error through as an answer.
    #[serde(default)]
    is_error: bool,
    /// Mirrors the Messages API field, so `max_tokens` here means the answer
    /// was cut mid-sentence. Every prompt asks for JSON, and a cut answer
    /// reaches the parser as a syntax error naming a line and column in text
    /// nobody kept.
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    result: Option<String>,
}

/// The ceiling the CLI is given for one answer.
///
/// On the Messages API a budget the answer overruns comes back as a truncated
/// body with `stop_reason: "max_tokens"`, which the caller can still inspect.
/// Claude Code turns the same overrun into a hard failure -- `API Error:
/// Claude's response exceeded the 2048 output token maximum` -- and the whole
/// call is lost; the first production import on 2026-09-29 lost chunks of two
/// Gaelic grammars that way. The caller's budget is therefore an expectation
/// rather than a wall: the CLI is given four times it, floored at 8 192 and
/// capped at 32 000, and an answer that overruns even that is still reported
/// against the caller's own figure.
fn output_ceiling(max_tokens: u32) -> u32 {
    max_tokens.saturating_mul(4).clamp(8_192, 32_000)
}

/// The one error every caller must be able to tell apart, since a cut answer
/// reaches the parser as a syntax error naming a line and column in text
/// nobody kept.
fn truncated(max_tokens: u32) -> WisecrowError {
    WisecrowError::LlmError(format!(
        "Claude Code CLI stopped at the max_tokens ceiling of {max_tokens}: the answer is \
         truncated, not malformed. Ask for less in one call or raise the budget."
    ))
}

/// Turns a finished run into the model's answer.
///
/// `code` is the child's exit status, `None` when a signal killed it.
///
/// # Errors
///
/// Returns [`WisecrowError::LlmError`] when the CLI printed something other
/// than a result object, when it reported a failure of its own, when the
/// answer was truncated at `max_tokens`, or when the answer is empty.
fn read_answer(
    stdout: &[u8],
    stderr: &[u8],
    code: Option<i32>,
    max_tokens: u32,
) -> Result<String, WisecrowError> {
    let Ok(parsed) = serde_json::from_slice::<CliResult>(stdout) else {
        // A rejected flag, a missing dependency or a crash: the CLI reports
        // these on stderr before the run starts, and stdout is empty or prose.
        let status = code.map_or_else(
            || "killed by signal".to_owned(),
            |code| format!("exit {code}"),
        );
        let detail = String::from_utf8_lossy(stderr);
        let detail = detail.trim();
        let detail = if detail.is_empty() {
            String::from_utf8_lossy(stdout).trim().to_owned()
        } else {
            detail.to_owned()
        };
        return Err(WisecrowError::LlmError(format!(
            "`{BINARY} -p` printed no result object ({status}): {detail}"
        )));
    };

    let answer = parsed.result.unwrap_or_default();
    if parsed.is_error {
        // The CLI reports an overrun of CLAUDE_CODE_MAX_OUTPUT_TOKENS as an
        // API error rather than as a stop reason, so both routes to a cut
        // answer have to be recognised here.
        if answer.contains("output token maximum") {
            return Err(truncated(max_tokens));
        }
        return Err(WisecrowError::LlmError(format!(
            "Claude Code CLI run failed: {}",
            if answer.trim().is_empty() {
                "no reason given".to_owned()
            } else {
                answer
            }
        )));
    }
    if parsed.stop_reason.as_deref() == Some("max_tokens") {
        return Err(truncated(max_tokens));
    }
    if answer.trim().is_empty() {
        return Err(WisecrowError::LlmError(
            "Empty response from the Claude Code CLI".to_owned(),
        ));
    }
    Ok(answer)
}

#[async_trait]
impl LlmProvider for ClaudeCliProvider {
    /// # Errors
    ///
    /// Returns [`WisecrowError::LlmError`] if `claude` cannot be run, if the
    /// prompt cannot be handed to it, or if the run does not produce an
    /// answer.
    async fn generate(&self, prompt: &str, max_tokens: u32) -> Result<String, WisecrowError> {
        for directory in [&self.config_dir, &self.working_dir] {
            tokio::fs::create_dir_all(directory).await.map_err(|e| {
                WisecrowError::LlmError(format!(
                    "Failed to create {} for the Claude Code CLI: {e}",
                    directory.display()
                ))
            })?;
        }

        let mut child = Command::new(BINARY)
            .args([
                "-p",
                "--output-format",
                "json",
                // No built-in tools: these prompts want an answer, not
                // actions, and a session with no tools cannot stall on a
                // permission prompt no one is there to answer.
                "--tools",
                "",
                // `--tools` does not cover MCP tools, and this ignores every
                // MCP configuration outside a `--mcp-config` we never pass.
                "--strict-mcp-config",
                "--disable-slash-commands",
                "--no-session-persistence",
            ])
            .arg("--model")
            .arg(&self.model)
            .arg("--system-prompt")
            .arg(SYSTEM_PROMPT)
            .current_dir(&self.working_dir)
            .env("CLAUDE_CODE_OAUTH_TOKEN", &self.oauth_token)
            .env("CLAUDE_CONFIG_DIR", &self.config_dir)
            // The deployment's service user has no home directory, and the CLI
            // writes its own state beside the credentials it is not storing.
            .env("HOME", &self.config_dir)
            .env(
                "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
                output_ceiling(max_tokens).to_string(),
            )
            // Parity with the Anthropic provider, which disables thinking for
            // the same reason: every prompt here asks for a fixed JSON shape,
            // thinking draws on the same output budget as the answer, and it
            // lengthens a call that already runs for minutes on a document
            // chunk. Measured 2026-09-29 on one grammar prompt: 18 thinking
            // tokens by default against 0 with this set.
            .env("MAX_THINKING_TOKENS", "0")
            .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
            // Both outrank the OAuth token in the CLI's credential order, so
            // leaving either in place would silently bill Console credits --
            // the very thing this provider exists to avoid.
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("ANTHROPIC_AUTH_TOKEN")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                WisecrowError::LlmError(format!(
                    "Failed to run `{BINARY}`: {e}. The claude-cli provider needs Claude Code \
                     installed on this host."
                ))
            })?;

        let mut stdin = child.stdin.take().ok_or_else(|| {
            WisecrowError::LlmError(
                "Claude Code CLI gave no stdin to write the prompt to".to_owned(),
            )
        })?;
        // Written while the child runs rather than before it is waited on: a
        // document chunk outgrows the pipe buffer, and a blocked write against
        // a child nobody is reading from is a deadlock.
        let feed = async move {
            stdin.write_all(prompt.as_bytes()).await?;
            stdin.shutdown().await
        };
        let ((), output) = tokio::try_join!(feed, child.wait_with_output())
            .map_err(|e| WisecrowError::LlmError(format!("Claude Code CLI run failed: {e}")))?;

        read_answer(
            &output.stdout,
            &output.stderr,
            output.status.code(),
            max_tokens,
        )
    }

    fn name(&self) -> &str {
        "claude-cli"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real 2.1.284 run.
    fn result_json(fields: &str) -> Vec<u8> {
        format!("{{\"type\":\"result\",\"subtype\":\"success\",{fields}}}").into_bytes()
    }

    #[test]
    fn answer_comes_from_the_result_field() {
        let stdout = result_json(
            "\"is_error\":false,\"stop_reason\":\"end_turn\",\"result\":\"[\\\"a\\\"]\"",
        );
        assert_eq!(
            read_answer(&stdout, b"", Some(0), 256).expect("a successful run yields its result"),
            "[\"a\"]"
        );
    }

    #[test]
    fn cli_reported_failure_is_an_error_despite_a_zero_exit() {
        let stdout =
            result_json("\"is_error\":true,\"result\":\"Not logged in · Please run /login\"");
        let error = read_answer(&stdout, b"", Some(0), 256).expect_err("is_error must not pass");
        assert!(
            error.to_string().contains("Not logged in"),
            "the CLI's own reason should reach the caller: {error}"
        );
    }

    #[test]
    fn truncation_is_named_rather_than_left_to_the_parser() {
        let stdout =
            result_json("\"is_error\":false,\"stop_reason\":\"max_tokens\",\"result\":\"[\\\"a\"");
        let error =
            read_answer(&stdout, b"", Some(0), 512).expect_err("a cut answer must not pass");
        assert!(
            error.to_string().contains("max_tokens ceiling of 512"),
            "truncation should name the budget: {error}"
        );
    }

    #[test]
    fn an_overrun_reported_as_an_api_error_reads_as_truncation() {
        let stdout = result_json(
            "\"is_error\":true,\"result\":\"API Error: Claude's response exceeded the 8192 output \
             token maximum. To configure this behavior, set the CLAUDE_CODE_MAX_OUTPUT_TOKENS \
             environment variable.\"",
        );
        let error = read_answer(&stdout, b"", Some(0), 2048).expect_err("an overrun must not pass");
        assert!(
            error.to_string().contains("max_tokens ceiling of 2048"),
            "an overrun should read as truncation against the caller's budget: {error}"
        );
    }

    #[test]
    fn the_cli_budget_leaves_room_above_the_caller_s() {
        assert_eq!(output_ceiling(2048), 8_192);
        assert_eq!(output_ceiling(4096), 16_384);
        assert_eq!(output_ceiling(16_384), 32_000);
        assert_eq!(output_ceiling(u32::MAX), 32_000);
    }

    #[test]
    fn empty_answer_is_an_error() {
        let stdout = result_json("\"is_error\":false,\"result\":\"  \"");
        assert!(read_answer(&stdout, b"", Some(0), 256).is_err());
    }

    #[test]
    fn non_json_stdout_reports_the_exit_status_and_stderr() {
        let error = read_answer(b"", b"error: unknown option '--tools'\n", Some(1), 256)
            .expect_err("a rejected flag must not pass");
        let message = error.to_string();
        assert!(message.contains("exit 1"), "message: {message}");
        assert!(message.contains("unknown option"), "message: {message}");
    }

    #[test]
    fn signal_death_is_reported_without_a_status() {
        let error = read_answer(b"", b"", None, 256).expect_err("a killed run must not pass");
        assert!(
            error.to_string().contains("killed by signal"),
            "message: {error}"
        );
    }
}
