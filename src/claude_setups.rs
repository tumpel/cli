use std::io::Read;

use crate::sessions::id;
use crate::{print_json, shown, signed_in, ClaudeSetupCommand};
use anyhow::{bail, Context, Result};
use tumpel_api::types::{ClaudeSetupCodeSubmit, ClaudeSetupStart};

fn code_from_stdin() -> Result<ClaudeSetupCodeSubmit> {
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .context("reading the sign-in code from stdin")?;
    let code = text.trim();
    if code.is_empty() {
        bail!("stdin held no sign-in code; pipe the code the sign-in page showed into tumpel claude-setup submit-code --stdin");
    }
    let code = code.to_string().try_into().map_err(|_| {
        anyhow::anyhow!(
            "the sign-in code on stdin is longer than 2000 characters; paste only the code the sign-in page showed"
        )
    })?;
    Ok(ClaudeSetupCodeSubmit { code })
}

pub async fn claude_setup(command: ClaudeSetupCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    match command {
        ClaudeSetupCommand::Start { secret_set_id } => {
            let body = ClaudeSetupStart {
                harness: None,
                secret_set_id: id("secretSetId", &secret_set_id)?,
            };
            print_json(&shown(client.claude_setups_start(&body).await, || {
                format!("{endpoint}/v1/claude-setups")
            })?)
        }
        ClaudeSetupCommand::List { secret_set_id } => {
            let set = id("secretSetId", &secret_set_id)?;
            print_json(&shown(client.claude_setups_list(&set).await, || {
                format!("{endpoint}/v1/claude-setups?secretSetId={secret_set_id}")
            })?)
        }
        ClaudeSetupCommand::Get { claude_setup_id } => {
            let setup = id("claudeSetupId", &claude_setup_id)?;
            print_json(&shown(client.claude_setups_get(&setup).await, || {
                format!("{endpoint}/v1/claude-setups/{claude_setup_id}")
            })?)
        }
        ClaudeSetupCommand::SubmitCode { claude_setup_id, stdin } => {
            if !stdin {
                bail!("the sign-in code is a secret and is read only from stdin; pass --stdin and pipe the code in");
            }
            let setup = id("claudeSetupId", &claude_setup_id)?;
            let body = code_from_stdin()?;
            print_json(&shown(client.claude_setups_submit_code(&setup, &body).await, || {
                format!("{endpoint}/v1/claude-setups/{claude_setup_id}/code")
            })?)
        }
    }
}
