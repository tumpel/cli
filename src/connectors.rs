use crate::resources::{entity_id, read_stdin};
use crate::sessions::parsed;
use crate::{answered, print_json, shown, signed_in, ConnectorCommand};
use anyhow::Result;
use tumpel_api::types::{
    Connector, ConnectorUpdate, EntityId, OAuthConnectorCreate, StaticConnectorCreate, StaticConnectorTokenWrite,
};
use tumpel_api::Client;

pub async fn connector(command: ConnectorCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    run(&client, &credentials.endpoint, command).await
}

/// The `updatedAt` of the connector as the control plane holds it now. A write sends it back so the
/// control plane refuses with `ConnectorChanged` (409) when another write landed in between. The caller
/// shows that refusal and stops; it never re-reads and retries.
async fn read_updated_at(client: &Client, endpoint: &str, connector_id: &str, id: &EntityId) -> Result<i64> {
    let connector = answered(client.connectors_get(id).await, || {
        format!("{endpoint}/v1/connectors/{connector_id}")
    })?;
    Ok(match connector {
        Connector::StaticConnector(connector) => connector.updated_at,
        Connector::OAuthConnector(connector) => connector.updated_at,
    })
}

async fn run(client: &Client, endpoint: &str, command: ConnectorCommand) -> Result<()> {
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
            let updated_at = read_updated_at(client, endpoint, &connector_id, &id).await?;
            print_json(&shown(client.connectors_update(&id, updated_at, &body).await, || {
                format!("{endpoint}/v1/connectors/{connector_id}")
            })?)
        }
        ConnectorCommand::Delete { connector_id } => {
            let id = entity_id(&connector_id, "connector")?;
            let updated_at = read_updated_at(client, endpoint, &connector_id, &id).await?;
            answered(client.connectors_delete(&id, updated_at).await, || {
                format!("{endpoint}/v1/connectors/{connector_id}")
            })?;
            eprintln!("deleted connector {connector_id}");
            Ok(())
        }
        ConnectorCommand::Authorize { connector_id } => {
            let id = entity_id(&connector_id, "connector")?;
            let updated_at = read_updated_at(client, endpoint, &connector_id, &id).await?;
            print_json(&shown(
                client.o_auth_connectors_authorize(&id, updated_at).await,
                || format!("{endpoint}/v1/oauth-connectors/{connector_id}/authorize"),
            )?)
        }
        ConnectorCommand::SetToken { connector_id, stdin } => {
            let id = entity_id(&connector_id, "connector")?;
            let body = StaticConnectorTokenWrite {
                value: read_stdin(stdin, "the connector token")?,
            };
            let updated_at = read_updated_at(client, endpoint, &connector_id, &id).await?;
            print_json(&shown(
                client.static_connectors_set_token(&id, updated_at, &body).await,
                || format!("{endpoint}/v1/static-connectors/{connector_id}/token"),
            )?)
        }
        ConnectorCommand::ClearCredential { connector_id } => {
            let id = entity_id(&connector_id, "connector")?;
            let updated_at = read_updated_at(client, endpoint, &connector_id, &id).await?;
            print_json(&shown(
                client.connectors_clear_credential(&id, updated_at).await,
                || format!("{endpoint}/v1/connectors/{connector_id}/credential"),
            )?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::run;
    use crate::ConnectorCommand;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tumpel_api::Client;

    const CONNECTOR_ID: &str = "00000000-0000-4000-8000-000000000001";

    /// Answers each request in turn and returns the request line of each.
    async fn serve(answers: Vec<String>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let mut lines = Vec::new();
            for answer in answers {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") {
                    head.push(stream.read_u8().await.unwrap());
                }
                lines.push(String::from_utf8(head).unwrap().lines().next().unwrap().to_string());
                stream.write_all(answer.as_bytes()).await.unwrap();
            }
            lines
        });
        (url, task)
    }

    #[tokio::test]
    async fn a_connector_write_sends_the_updated_at_it_just_read() {
        let body = format!(
            r#"{{"auth":"Static","connectorId":"{CONNECTOR_ID}","createdAt":1,"name":"docs","status":{{"state":"NotConnected"}},"tenantId":"{CONNECTOR_ID}","updatedAt":1234,"url":"https://example.com/mcp"}}"#
        );
        let read = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        let deleted = "HTTP/1.1 204 No Content\r\nconnection: close\r\n\r\n".to_string();
        let (url, task) = serve(vec![read, deleted]).await;

        run(
            &Client::new(&url).unwrap(),
            &url,
            ConnectorCommand::Delete {
                connector_id: CONNECTOR_ID.to_string(),
            },
        )
        .await
        .unwrap();

        assert_eq!(
            task.await.unwrap(),
            vec![
                format!("GET /v1/connectors/{CONNECTOR_ID} HTTP/1.1"),
                format!("DELETE /v1/connectors/{CONNECTOR_ID}?updatedAt=1234 HTTP/1.1"),
            ]
        );
    }
}
