use crate::sessions::id;
use crate::{print_json, shown, signed_in, OpenrouterCommand};
use anyhow::Result;
use tumpel_api::types::OpenRouterAuthorizationStart;

pub async fn openrouter(command: OpenrouterCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    match command {
        OpenrouterCommand::Authorize { secret_set_id } => {
            let body = OpenRouterAuthorizationStart {
                secret_set_id: id("secretSetId", &secret_set_id)?,
            };
            let authorization = shown(client.open_router_connections_authorize(&body).await, || {
                format!("{endpoint}/v1/openrouter-connection/authorize")
            })?;
            eprintln!("open authorizeUrl in a browser and approve on OpenRouter within 10 minutes; the key goes into the secret set and is never shown");
            print_json(&authorization)
        }
        OpenrouterCommand::GetAuthorization {
            open_router_authorization_id,
        } => {
            let authorization = id("openRouterAuthorizationId", &open_router_authorization_id)?;
            print_json(&shown(
                client.open_router_connections_get_authorization(&authorization).await,
                || format!("{endpoint}/v1/openrouter-connection/authorizations/{open_router_authorization_id}"),
            )?)
        }
    }
}
