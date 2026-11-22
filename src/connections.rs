use crate::sessions::{id, parsed};
use crate::{answered, print_json, shown, signed_in, ConnectionCommand, NamespaceCommand, OauthGrantCommand};
use anyhow::Result;
use tumpel_api::types::NamespaceConnectionStart;

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

pub async fn namespace(command: NamespaceCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let url = format!("{}/v1/namespace-connection", credentials.endpoint);
    match command {
        NamespaceCommand::Get => print_json(&shown(client.namespace_connections_get().await, || url)?),
        NamespaceCommand::Start {
            account_label,
            workspace_id,
        } => {
            let body = NamespaceConnectionStart {
                account_label: parsed("--account-label", &account_label)?,
                workspace_id: parsed("--workspace-id", &workspace_id)?,
            };
            print_json(&shown(client.namespace_connections_start(&body).await, || {
                format!("{url}/start")
            })?)
        }
        NamespaceCommand::Delete => {
            answered(client.namespace_connections_delete().await, || url)?;
            eprintln!("disconnected the Namespace build provider");
            Ok(())
        }
    }
}
