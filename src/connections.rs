use crate::sessions::id;
use crate::{answered, print_json, shown, signed_in, ConnectionCommand, OauthGrantCommand};
use anyhow::Result;

pub async fn connection(command: ConnectionCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    match command {
        ConnectionCommand::List => print_json(&shown(client.connections_list().await, || {
            format!("{endpoint}/v1/connections")
        })?),
    }
}

pub async fn oauth_grant(command: OauthGrantCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    match command {
        OauthGrantCommand::List => print_json(&shown(client.o_auth_grants_list().await, || {
            format!("{endpoint}/v1/oauth-grants")
        })?),
        OauthGrantCommand::Revoke { grant_id } => {
            let id = id("grant_id", &grant_id)?;
            answered(client.o_auth_grants_revoke(&id).await, || {
                format!("{endpoint}/v1/oauth-grants/{grant_id}")
            })?;
            eprintln!("revoked {grant_id}");
            Ok(())
        }
    }
}
