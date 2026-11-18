use crate::resources::{entity_id, read_stdin};
use crate::sessions::parsed;
use crate::{answered, print_json, shown, signed_in, ConnectorCommand};
use anyhow::Result;
use tumpel_api::types::{ConnectorUpdate, OAuthConnectorCreate, StaticConnectorCreate, StaticConnectorTokenWrite};

pub async fn connector(command: ConnectorCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    match command {
        ConnectorCommand::List => print_json(&shown(client.connectors_list().await, || {
            format!("{endpoint}/v1/connectors")
        })?),
        ConnectorCommand::Get { connector_id } => {
            let id = entity_id(&connector_id, "connector")?;
            print_json(&shown(client.connectors_get(&id).await, || {
                format!("{endpoint}/v1/connectors/{connector_id}")
            })?)
        }
        ConnectorCommand::CreateStatic { name, url } => {
            let body = StaticConnectorCreate {
                name: parsed("--name", &name)?,
                url: parsed("--url", &url)?,
            };
            print_json(&shown(client.static_connectors_create(&body).await, || {
                format!("{endpoint}/v1/static-connectors")
            })?)
        }
        ConnectorCommand::CreateOauth { name, url } => {
            let body = OAuthConnectorCreate {
                name: parsed("--name", &name)?,
                url: parsed("--url", &url)?,
            };
            print_json(&shown(client.o_auth_connectors_create(&body).await, || {
                format!("{endpoint}/v1/oauth-connectors")
            })?)
        }
        ConnectorCommand::Update { connector_id, name } => {
            let id = entity_id(&connector_id, "connector")?;
            let body = ConnectorUpdate {
                name: parsed("--name", &name)?,
            };
            print_json(&shown(client.connectors_update(&id, &body).await, || {
                format!("{endpoint}/v1/connectors/{connector_id}")
            })?)
        }
        ConnectorCommand::Delete { connector_id } => {
            let id = entity_id(&connector_id, "connector")?;
            answered(client.connectors_delete(&id).await, || {
                format!("{endpoint}/v1/connectors/{connector_id}")
            })?;
            eprintln!("deleted connector {connector_id}");
            Ok(())
        }
        ConnectorCommand::Authorize { connector_id } => {
            let id = entity_id(&connector_id, "connector")?;
            print_json(&shown(client.o_auth_connectors_authorize(&id).await, || {
                format!("{endpoint}/v1/oauth-connectors/{connector_id}/authorize")
            })?)
        }
        ConnectorCommand::SetToken { connector_id, stdin } => {
            let id = entity_id(&connector_id, "connector")?;
            let body = StaticConnectorTokenWrite {
                value: read_stdin(stdin, "the connector token")?,
            };
            print_json(&shown(client.static_connectors_set_token(&id, &body).await, || {
                format!("{endpoint}/v1/static-connectors/{connector_id}/token")
            })?)
        }
        ConnectorCommand::ClearCredential { connector_id } => {
            let id = entity_id(&connector_id, "connector")?;
            print_json(&shown(client.connectors_clear_credential(&id).await, || {
                format!("{endpoint}/v1/connectors/{connector_id}/credential")
            })?)
        }
    }
}
