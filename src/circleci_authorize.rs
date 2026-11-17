use std::io::{BufRead, IsTerminal, Read};
use std::time::Duration;

use crate::sessions::{id, parsed};
use crate::{answered, print_json};
use anyhow::{anyhow, bail, Context, Result};
use reqwest::Url;
use tokio::net::{TcpListener, TcpStream};
use tumpel_api::types::{
    CircleCiAuthorization, CircleCiAuthorizationChoose, CircleCiAuthorizationComplete, CircleCiAuthorizationStatus,
    CircleCiOrganization, EntityId,
};
use tumpel_api::Client;

const REQUEST_HEAD_LIMIT: usize = 8192;
const LANDED_PAGE: &str = "<!doctype html><title>tumpel</title><p>CircleCI answered tumpel. Return to your terminal; you can close this tab.</p>";

pub async fn authorize(client: &Client, url: &str, organization_id: Option<String>) -> Result<()> {
    let started = answered(client.circle_ci_connections_authorize().await, || {
        format!("{url}/authorize")
    })?;
    let CircleCiAuthorizationStatus::AwaitingRedirect(waiting) = &started.status else {
        bail!(
            "the control plane answered a new CircleCI authorization that is not AwaitingRedirect: {:?}",
            started.status
        );
    };
    let redirect = Url::parse(&waiting.redirect_uri)
        .with_context(|| format!("the redirect address {} is not a URL", waiting.redirect_uri))?;
    let port = redirect
        .port()
        .ok_or_else(|| anyhow!("the redirect address {} names no port", waiting.redirect_uri))?;
    let listener = TcpListener::bind(("127.0.0.1", port)).await.with_context(|| {
        format!(
            "listening on 127.0.0.1:{port} for CircleCI's redirect; free the port, or connect CircleCI from the dashboard, which takes the address the browser lands on"
        )
    })?;
    eprintln!(
        "Open this address, choose Write or Admin, then Allow:\n\n  {}\n\nWaiting for CircleCI to send the browser back to {} until {}.",
        waiting.authorize_url, waiting.redirect_uri, waiting.redirect_due_at
    );
    let remaining = (waiting.redirect_due_at.with_timezone(&chrono::Utc) - chrono::Utc::now())
        .to_std()
        .unwrap_or(Duration::ZERO);
    let state = state_of(&waiting.authorize_url)?;
    let target = tokio::time::timeout(remaining, landed(&listener, &redirect, &state))
        .await
        .map_err(|_| {
            anyhow!(
                "CircleCI did not send the browser back to {} by {}; run tumpel circleci authorize again and choose Allow within that time",
                waiting.redirect_uri,
                waiting.redirect_due_at
            )
        })??;
    let landed_url = redirect
        .join(&target)
        .context("joining the redirect address and the request target")?;
    let completed = complete(client, url, &started.circle_ci_authorization_id, landed_url.to_string()).await?;
    settle(client, url, completed, organization_id).await
}

fn state_of(address: &str) -> Result<String> {
    let url = Url::parse(address).with_context(|| format!("{address} is not a URL"))?;
    url.query_pairs()
        .find(|(name, _)| name == "state")
        .map(|(_, value)| value.into_owned())
        .ok_or_else(|| anyhow!("the consent address {address} carries no state"))
}

async fn landed(listener: &TcpListener, redirect: &Url, state: &str) -> Result<String> {
    let path = redirect.path();
    loop {
        let (stream, _) = listener.accept().await.context("accepting CircleCI's redirect")?;
        let head = read_head(&stream).await?;
        let target = head
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("GET "))
            .and_then(|rest| rest.split(' ').next())
            .unwrap_or("")
            .to_string();
        let ours = (target == path || target.starts_with(&format!("{path}?")))
            && redirect
                .join(&target)
                .ok()
                .and_then(|landed| state_of(landed.as_str()).ok())
                .is_some_and(|landed| landed == state);
        let reply = if ours {
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{LANDED_PAGE}",
                LANDED_PAGE.len()
            )
        } else {
            "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_string()
        };
        write_all(&stream, reply.as_bytes()).await?;
        if ours {
            return Ok(target);
        }
    }
}

async fn read_head(stream: &TcpStream) -> Result<String> {
    let mut head = Vec::new();
    let mut buffer = [0u8; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        if head.len() > REQUEST_HEAD_LIMIT {
            bail!("the request to the redirect listener has a head over {REQUEST_HEAD_LIMIT} bytes");
        }
        stream.readable().await.context("reading the redirect request")?;
        match stream.try_read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => head.extend_from_slice(&buffer[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error).context("reading the redirect request"),
        }
    }
    Ok(String::from_utf8_lossy(&head).into_owned())
}

async fn write_all(stream: &TcpStream, mut bytes: &[u8]) -> Result<()> {
    while !bytes.is_empty() {
        stream.writable().await.context("answering the browser")?;
        match stream.try_write(bytes) {
            Ok(written) => bytes = &bytes[written..],
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error).context("answering the browser"),
        }
    }
    Ok(())
}

async fn complete(
    client: &Client,
    url: &str,
    authorization: &EntityId,
    redirected_url: String,
) -> Result<CircleCiAuthorization> {
    let body = CircleCiAuthorizationComplete {
        circle_ci_authorization_id: authorization.clone(),
        redirected_url: redirected_url
            .try_into()
            .map_err(|error| anyhow!("the redirect address is not usable: {error}"))?,
    };
    answered(client.circle_ci_connections_complete(&body).await, || {
        format!("{url}/complete")
    })
}

async fn choose(
    client: &Client,
    url: &str,
    authorization: &EntityId,
    organization_id: &str,
) -> Result<CircleCiAuthorization> {
    let body = CircleCiAuthorizationChoose {
        circle_ci_authorization_id: authorization.clone(),
        organization_id: parsed("--organization-id", organization_id)?,
    };
    answered(client.circle_ci_connections_choose(&body).await, || {
        format!("{url}/choose")
    })
}

fn pick(authorization: &EntityId, organizations: &[CircleCiOrganization]) -> Result<String> {
    let listed = organizations
        .iter()
        .enumerate()
        .map(|(index, organization)| {
            format!(
                "  {}. {} ({}, {})",
                index + 1,
                *organization.name,
                organization.slug,
                *organization.organization_id
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    if !std::io::stdin().is_terminal() {
        bail!(
            "the token reads several CircleCI organizations:\n{listed}\npick one with tumpel circleci choose --circleci-authorization-id {} --organization-id <id>",
            **authorization
        );
    }
    eprintln!(
        "The token reads several CircleCI organizations:\n{listed}\nWhich one bills the builds? Type its number:"
    );
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .context("reading the organization number")?;
    let chosen = line
        .trim()
        .parse::<usize>()
        .ok()
        .and_then(|number| number.checked_sub(1))
        .and_then(|index| organizations.get(index))
        .ok_or_else(|| anyhow!("{:?} is not a number from 1 to {}", line.trim(), organizations.len()))?;
    Ok(chosen.organization_id.to_string())
}

async fn settle(
    client: &Client,
    url: &str,
    authorization: CircleCiAuthorization,
    organization_id: Option<String>,
) -> Result<()> {
    let chosen = if let CircleCiAuthorizationStatus::ChoosingOrganization(choosing) = &authorization.status {
        let id = &authorization.circle_ci_authorization_id;
        let organization = match organization_id {
            Some(organization) => organization,
            None => pick(id, &choosing.organizations)?,
        };
        choose(client, url, id, &organization).await?
    } else {
        authorization
    };
    if let CircleCiAuthorizationStatus::Failed(failed) = &chosen.status {
        bail!("the CircleCI authorization failed {}: {}", failed.reason, failed.detail);
    }
    print_json(&chosen)
}

pub async fn complete_from_stdin(client: &Client, url: &str, authorization: &str, stdin: bool) -> Result<()> {
    if !stdin {
        bail!("the redirect address carries a one-time code and is read only from stdin; pass --stdin and pipe the address in");
    }
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .context("reading the redirect address from stdin")?;
    let address = text.trim();
    if address.is_empty() {
        bail!("stdin held no address; pipe in the address the browser landed on after CircleCI's consent page");
    }
    let id = id("--circleci-authorization-id", authorization)?;
    print_json(&complete(client, url, &id, address.to_string()).await?)
}

pub async fn choose_organization(client: &Client, url: &str, authorization: &str, organization_id: &str) -> Result<()> {
    let id = id("--circleci-authorization-id", authorization)?;
    print_json(&choose(client, url, &id, organization_id).await?)
}
