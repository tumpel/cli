mod support;

use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;
use tokio_tungstenite::tungstenite::Message;

const ID: &str = "00000000-0000-4000-8000-000000000001";
const OTHER_ID: &str = "00000000-0000-4000-8000-000000000002";

async fn cli(endpoint: &str, directory: &Path, args: &[&str], code: i32) -> String {
    let credentials = directory.join("env");
    std::fs::write(&credentials, "TUMPEL_TOKEN=scenario-credential\n").unwrap();
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        Command::new(env!("CARGO_BIN_EXE_tumpel"))
            .env_clear()
            .env("TUMPEL_ENDPOINT", endpoint)
            .env("TUMPEL_ENV_FILE", credentials)
            .current_dir(directory)
            .args(args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("CLI scenario timed out")
    .unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(code), "{args:?}: {stderr}");
    assert_eq!(
        stderr,
        format!("the server sent a response this CLI doesn't recognise, please update the CLI: curl -fsSL {endpoint}/v1/releases/install.sh | sh\n"),
    );
    String::from_utf8(output.stdout).unwrap()
}

async fn http(args: &[&str], request: &str, body: &Value, code: i32) -> String {
    let body = body.to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let (endpoint, served) = support::serve(vec![response]).await;
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("tenant.json"),
        r#"{"personas":[],"secretSets":[],"harnessPins":[],"hosts":[]}"#,
    )
    .unwrap();
    let stdout = cli(&endpoint, directory.path(), args, code).await;
    assert_eq!(served.await.unwrap(), [request]);
    stdout
}

#[tokio::test]
async fn session_list_preserves_known_and_future_rows_in_order() {
    let body = json!([
        {
            "state": "Idle", "sessionId": ID, "tenantId": ID, "principalId": ID,
            "personaId": ID, "boxId": ID, "hostId": ID, "harness": "Codex", "harnessVersion": "1.0",
            "model": "test-model", "origin": {"kind": "Prompt"}, "tags": [],
            "startedBy": {"kind": "Person", "principalId": ID},
            "repository": "https://github.com/example/project", "boxRetention": "Warm",
            "lastTurnId": ID, "lastTurnAt": "2026-11-24T00:00:00Z", "lastTurnState": "Offered",
            "lastTurn": {"state": "Offered", "turnId": ID, "sessionId": ID, "tenantId": ID,
                         "boxId": ID, "prompt": "Review", "lineCount": 0, "createdAt": 1, "updatedAt": 1},
            "createdAt": 1, "updatedAt": 1
        },
        {"state": "FutureSession", "sessionId": OTHER_ID, "extra": {"ready": true}}
    ]);
    let mut wire = body.clone();
    wire[0]["extra"] = json!("ignored on a known row");
    let stdout = http(&["session", "list"], "GET /v1/sessions?limit=50 HTTP/1.1", &wire, 0).await;
    assert_eq!(serde_json::from_str::<Value>(&stdout).unwrap(), body);
}

#[tokio::test]
async fn session_transcript_stops_on_a_future_page_form() {
    let stdout = http(
        &["session", "transcript", ID],
        &format!("GET /v1/sessions/{ID}/transcript?after=0&compact=false HTTP/1.1"),
        &json!({"form": "FutureTranscript", "lines": [], "cursor": 0, "finished": true}),
        3,
    )
    .await;
    assert!(stdout.is_empty(), "{stdout}");
}

#[tokio::test]
async fn events_wait_prints_a_future_event() {
    let event = json!({"event": "FutureEvent", "eventId": 2, "tenantId": ID, "createdAt": 1, "extra": true});
    let stdout = http(
        &["events", "wait", "--timeout-seconds", "2"],
        "GET /v1/events HTTP/1.1",
        &json!({"events": [event], "lastEventId": 2}),
        0,
    )
    .await;
    assert_eq!(serde_json::from_str::<Value>(&stdout).unwrap(), event);
}

async fn websocket(connections: Vec<(&'static str, Vec<Message>)>) -> String {
    use futures_util::SinkExt;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let served = tokio::spawn(async move {
        for (uri, frames) in connections {
            let (stream, _) = listener.accept().await.unwrap();
            #[expect(
                clippy::result_large_err,
                reason = "the WebSocket handshake callback fixes the error type"
            )]
            let mut websocket = tokio_tungstenite::accept_hdr_async(stream, |request: &Request, response: Response| {
                assert_eq!(request.uri(), uri);
                Ok(response)
            })
            .await
            .unwrap();
            for frame in frames {
                websocket.send(frame).await.unwrap();
            }
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let stdout = cli(&endpoint, directory.path(), &["events", "follow"], 3).await;
    served.await.unwrap();
    stdout
}

#[tokio::test]
async fn events_follow_resumes_from_the_cursor_after_server_close() {
    let known = json!({"event": "HostOffline", "eventId": 1, "tenantId": ID, "hostId": ID, "createdAt": 1});
    let future = json!({"event": "FutureEvent", "eventId": 2, "tenantId": ID, "createdAt": 1, "extra": true});
    let stdout = websocket(vec![
        (
            "/v1/events",
            vec![Message::text(known.to_string()), Message::Close(None)],
        ),
        (
            "/v1/events?afterEventId=1",
            vec![Message::text(future.to_string()), Message::text("{")],
        ),
    ])
    .await;
    assert_eq!(stdout, format!("{known}\n{future}\n"));
}

#[tokio::test]
async fn events_follow_prints_a_changed_shape_and_continues_until_malformed_json() {
    let changed = json!({"event": "HostOffline", "eventId": 1, "tenantId": ID, "hostId": {"id": ID}, "createdAt": 1});
    let known = json!({"event": "HostOffline", "eventId": 2, "tenantId": ID, "hostId": ID, "createdAt": 1});
    let stdout = websocket(vec![(
        "/v1/events",
        vec![
            Message::text(changed.to_string()),
            Message::text(known.to_string()),
            Message::text("{"),
        ],
    )])
    .await;
    assert_eq!(stdout, format!("{changed}\n{known}\n"));
}

#[tokio::test]
async fn host_get_prints_a_future_presence() {
    let body = json!({"presence": "FuturePresence", "hostId": ID, "extra": true});
    let stdout = http(&["host", "get", ID], &format!("GET /v1/hosts/{ID} HTTP/1.1"), &body, 0).await;
    assert_eq!(serde_json::from_str::<Value>(&stdout).unwrap(), body);
}

#[tokio::test]
async fn config_plan_prints_the_description_of_a_future_action() {
    let stdout = http(
        &["config", "plan", "tenant.json"],
        "POST /v1/tenant-config/plan HTTP/1.1",
        &json!({"changes": [
            {"action": "Update", "resource": "Persona", "name": "Build", "description": "Update Build"},
            {"action": "FutureAction", "resource": "Persona", "name": "Review", "description": "Migrate Review"}
        ]}),
        0,
    )
    .await;
    assert_eq!(stdout, "Update Build\nMigrate Review\n");
}

#[tokio::test]
async fn config_apply_stops_when_a_live_persona_file_cannot_be_used_for_a_write() {
    let stdout = http(
        &["config", "apply", "tenant.json"],
        "GET /v1/personas HTTP/1.1",
        &json!([{"personaId": ID, "tenantId": ID, "secretSetId": ID, "name": "Review", "harness": "Codex",
                 "boxRetention": "Off", "connectorIds": [], "files": [{"kind": "FutureFile", "extra": true}],
                 "tumpelMcp": false, "createdAt": 1, "updatedAt": 1}]),
        3,
    )
    .await;
    assert!(stdout.is_empty(), "{stdout}");
}
