use crate::sessions::id;
use crate::{print_json, shown, signed_in, EventsCommand};
use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tumpel_api::types::{EntityId, Sequence, TenantEventType};
use tumpel_api::{types::TenantEvent, ContainsUnrecognised};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const RECONNECT_AFTER: Duration = Duration::from_secs(1);

pub async fn events(command: EventsCommand) -> Result<()> {
    match command {
        EventsCommand::Wait {
            after_event_id,
            event,
            session_id,
            turn_id,
            timeout_seconds,
        } => wait(after_event_id, event, session_id, turn_id, timeout_seconds).await,
        EventsCommand::Follow {
            after_event_id,
            event,
            session_id,
        } => follow(after_event_id, &event, &session_id).await,
    }
}

fn server_session(session_ids: &[String]) -> Result<Option<EntityId>> {
    let mut sessions = session_ids
        .iter()
        .map(|session| id("sessionId", session))
        .collect::<Result<Vec<_>>>()?;
    Ok(if sessions.len() == 1 { sessions.pop() } else { None })
}

fn names_session(event: &serde_json::Value, session_ids: &[String]) -> bool {
    session_ids.is_empty()
        || event
            .get("sessionId")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|session| session_ids.iter().any(|wanted| wanted == session))
}

fn names_turn(event: &serde_json::Value, turn_id: Option<&str>) -> bool {
    turn_id.is_none_or(|turn| event.get("turnId").and_then(serde_json::Value::as_str) == Some(turn))
}

async fn wait(
    after_event_id: Option<i64>,
    event: Vec<TenantEventType>,
    session_id: Vec<String>,
    turn_id: Option<String>,
    timeout_seconds: u64,
) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    let session = server_session(&session_id)?;
    let types = (!event.is_empty()).then_some(event);
    let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
    let mut cursor = after_event_id;
    loop {
        let after = cursor.map(Sequence);
        let page = shown(
            client
                .events_list(after.as_ref(), types.as_ref(), session.as_ref(), None)
                .await,
            || format!("{endpoint}/v1/events"),
        )?;
        let events = page
            .get("events")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let Some(found) = events
            .iter()
            .find(|event| names_session(event, &session_id) && names_turn(event, turn_id.as_deref()))
        {
            return print_json(found);
        }
        cursor = page.get("lastEventId").and_then(serde_json::Value::as_i64);
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            bail!(
                "No matching event happened within {timeout_seconds} seconds. Run the command again to keep waiting."
            );
        }
        tokio::time::sleep(POLL_INTERVAL.min(left)).await;
    }
}

fn follow_url(
    endpoint: &str,
    after_event_id: Option<i64>,
    event: &[TenantEventType],
    session_id: Option<&str>,
) -> Result<String> {
    let mut url = reqwest::Url::parse(&format!("{endpoint}/v1/events"))
        .context("the stored endpoint is not a URL; run tumpel login")?;
    let scheme = if url.scheme() == "http" { "ws" } else { "wss" };
    url.set_scheme(scheme)
        .map_err(|()| anyhow::anyhow!("the stored endpoint {endpoint} cannot carry a WebSocket; run tumpel login"))?;
    {
        let mut query = url.query_pairs_mut();
        if let Some(after) = after_event_id {
            query.append_pair("afterEventId", &after.to_string());
        }
        for kind in event {
            query.append_pair("event", &kind.to_string());
        }
        if let Some(session) = session_id {
            query.append_pair("sessionId", session);
        }
    }
    Ok(url.into())
}

async fn follow(after_event_id: Option<i64>, event: &[TenantEventType], session_id: &[String]) -> Result<()> {
    let (credentials, _) = signed_in()?;
    let session = server_session(session_id)?;
    let mut cursor = after_event_id;
    loop {
        let mut request = follow_url(
            &credentials.endpoint,
            cursor,
            event,
            session.as_deref().map(String::as_str),
        )?
        .into_client_request()?;
        let mut authorization = HeaderValue::from_str(&format!("Bearer {}", *credentials.token))
            .context("the stored token is not a valid header value; run tumpel login")?;
        authorization.set_sensitive(true);
        request.headers_mut().insert("authorization", authorization);
        let (mut socket, _) = match tokio_tungstenite::connect_async(request).await {
            Ok(connected) => connected,
            Err(WsError::Http(response)) => bail!(
                "The Tumpel service refused to stream events with status {}: {}",
                response.status().as_u16(),
                String::from_utf8_lossy(response.body().as_deref().unwrap_or_default())
            ),
            Err(error) => return Err(error).context("connecting to the event stream"),
        };
        while let Some(message) = socket.next().await {
            let text = match message {
                Ok(Message::Text(text)) => text,
                Ok(Message::Close(_)) | Err(_) => break,
                Ok(_) => continue,
            };
            let event: serde_json::Value =
                serde_json::from_str(&text).map_err(|_| crate::unrecognised_response(&credentials.endpoint))?;
            if serde_json::from_value::<TenantEvent>(event.clone())
                .map_or(true, |decoded| decoded.contains_unrecognised())
            {
                crate::update_message(&crate::unrecognised_response(&credentials.endpoint));
            }
            cursor = event.get("eventId").and_then(serde_json::Value::as_i64).or(cursor);
            if names_session(&event, session_id) {
                println!("{text}");
            }
        }
        tokio::time::sleep(RECONNECT_AFTER).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_follow_url_turns_https_into_wss_and_carries_every_filter() -> Result<()> {
        let url = follow_url(
            "https://tumpel.example",
            Some(7),
            &[TenantEventType::TurnCompleted, TenantEventType::TurnFailed],
            Some("s1"),
        )?;
        assert_eq!(
            url,
            "wss://tumpel.example/v1/events?afterEventId=7&event=TurnCompleted&event=TurnFailed&sessionId=s1"
        );
        Ok(())
    }

    #[test]
    fn a_session_filter_keeps_only_events_of_those_sessions() {
        let sessions = ["s1".to_string(), "s2".to_string()];
        assert!(names_session(&serde_json::json!({ "sessionId": "s2" }), &sessions));
        assert!(!names_session(&serde_json::json!({ "sessionId": "s3" }), &sessions));
        assert!(!names_session(
            &serde_json::json!({ "event": "HostOffline" }),
            &sessions
        ));
        assert!(names_session(&serde_json::json!({ "event": "HostOffline" }), &[]));
    }

    #[test]
    fn a_turn_filter_keeps_only_events_of_that_turn() {
        let event = serde_json::json!({ "event": "TurnCompleted", "turnId": "t1" });
        assert!(names_turn(&event, Some("t1")));
        assert!(!names_turn(&event, Some("t2")));
        assert!(names_turn(&serde_json::json!({ "event": "HostOffline" }), None));
    }
}
