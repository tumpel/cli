use crate::resources::read_stdin;
use crate::sessions::{id, parsed};
use crate::{
    answered, print_json, shown, signed_in, CircleciCommand, CircleciProjectCommand, ConnectionCommand,
    GithubActionsCommand, NamespaceCommand, OauthGrantCommand,
};
use anyhow::Result;
use tumpel_api::types::{
    CircleCiConnectionStart, CircleCiProjectAdd, GitHubActionsConnectionStart, NamespaceConnectionStart,
};

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

pub async fn github_actions(command: GithubActionsCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let url = format!("{}/v1/github-actions-connection", credentials.endpoint);
    match command {
        GithubActionsCommand::Get => print_json(&shown(client.git_hub_actions_connections_get().await, || url)?),
        GithubActionsCommand::Start {
            account_label,
            caller_workflow,
        } => {
            let body = GitHubActionsConnectionStart {
                account_label: parsed("--account-label", &account_label)?,
                caller_workflow: parsed("--caller-workflow", &caller_workflow)?,
            };
            print_json(&shown(client.git_hub_actions_connections_start(&body).await, || {
                format!("{url}/start")
            })?)
        }
        GithubActionsCommand::Delete => {
            answered(client.git_hub_actions_connections_delete().await, || url)?;
            eprintln!("disconnected the GitHub Actions build provider");
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

pub async fn circleci(command: CircleciCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let url = format!("{}/v1/circleci-connection", credentials.endpoint);
    let projects = format!("{}/v1/circleci-projects", credentials.endpoint);
    match command {
        CircleciCommand::Get => print_json(&shown(client.circle_ci_connections_get().await, || url)?),
        CircleciCommand::Authorize { organization_id } => {
            crate::circleci_authorize::authorize(&client, &url, organization_id).await
        }
        CircleciCommand::Complete {
            circle_ci_authorization_id,
            stdin,
        } => crate::circleci_authorize::complete_from_stdin(&client, &url, &circle_ci_authorization_id, stdin).await,
        CircleciCommand::Choose {
            circle_ci_authorization_id,
            organization_id,
        } => {
            crate::circleci_authorize::choose_organization(&client, &url, &circle_ci_authorization_id, &organization_id)
                .await
        }
        CircleciCommand::Start {
            account_label,
            organization_id,
            token_stdin,
        } => {
            let body = CircleCiConnectionStart {
                account_label: parsed("--account-label", &account_label)?,
                organization_id: parsed("--organization-id", &organization_id)?,
                api_token: read_stdin(token_stdin, "the CircleCI API token")?,
            };
            print_json(&shown(client.circle_ci_connections_start(&body).await, || {
                format!("{url}/start")
            })?)
        }
        CircleciCommand::Delete => {
            answered(client.circle_ci_connections_delete().await, || url)?;
            eprintln!("disconnected the CircleCI build provider and deleted its project bindings");
            Ok(())
        }
        CircleciCommand::Project { command } => match command {
            CircleciProjectCommand::List => print_json(&shown(client.circle_ci_projects_list().await, || projects)?),
            CircleciProjectCommand::Add {
                repository,
                project_slug,
                pipeline_definition_id,
            } => {
                let body = CircleCiProjectAdd {
                    repository: parsed("--repository", &repository)?,
                    project_slug: parsed("--project-slug", &project_slug)?,
                    pipeline_definition_id: parsed("--pipeline-definition-id", &pipeline_definition_id)?,
                };
                print_json(&shown(client.circle_ci_projects_add(&body).await, || projects)?)
            }
            CircleciProjectCommand::Delete { circle_ci_project_id } => {
                let id = id("circle_ci_project_id", &circle_ci_project_id)?;
                answered(client.circle_ci_projects_delete(&id).await, || {
                    format!("{projects}/{circle_ci_project_id}")
                })?;
                eprintln!("deleted the CircleCI project binding {circle_ci_project_id}");
                Ok(())
            }
        },
    }
}
