mod support;

use serde_json::{json, Value};
use std::time::Duration;
use tokio::process::Command;

const TENANT: &str = "00000000-0000-4000-8000-000000000000";

fn session(number: usize) -> Value {
    let id = format!("00000000-0000-4000-8000-{number:012}");
    json!({
        "state": "Idle", "sessionId": id, "tenantId": TENANT, "principalId": TENANT,
        "personaId": TENANT, "boxId": TENANT, "hostId": TENANT, "harness": "Codex", "harnessVersion": "1.0",
        "model": "test-model", "origin": {"kind": "Prompt"}, "tags": [],
        "startedBy": {"kind": "Person", "principalId": TENANT},
        "repository": "https://github.com/example/project", "boxRetention": "Warm",
        "lastTurnId": id, "lastTurnAt": "2026-11-24T00:00:00Z", "lastTurnState": "Offered",
        "lastTurn": {"state": "Offered", "turnId": id, "sessionId": id, "tenantId": TENANT,
                     "boxId": TENANT, "prompt": "Review", "lineCount": 0, "createdAt": 1, "updatedAt": 1},
        "createdAt": number, "updatedAt": number
    })
}

#[tokio::test]
async fn session_list_pages_through_every_session_fifty_at_a_time() {
    let sessions: Vec<Value> = (1..=120).map(session).collect();
    let answers = sessions
        .chunks(50)
        .map(|page| {
            let body = Value::from(page).to_string();
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
        })
        .collect();
    let (endpoint, served) = support::serve(answers).await;
    let directory = tempfile::tempdir().unwrap();
    let credentials = directory.path().join("env");
    std::fs::write(&credentials, "TUMPEL_TOKEN=scenario-credential\n").unwrap();
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        Command::new(env!("CARGO_BIN_EXE_tumpel"))
            .env_clear()
            .env("TUMPEL_ENDPOINT", &endpoint)
            .env("TUMPEL_ENV_FILE", credentials)
            .current_dir(directory.path())
            .args(["session", "list"])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("CLI scenario timed out")
    .unwrap();
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        served.await.unwrap(),
        [
            "GET /v1/sessions?limit=50 HTTP/1.1",
            "GET /v1/sessions?after=00000000-0000-4000-8000-000000000050&limit=50 HTTP/1.1",
            "GET /v1/sessions?after=00000000-0000-4000-8000-000000000100&limit=50 HTTP/1.1",
        ]
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        Value::from(sessions)
    );
}
