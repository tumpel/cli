use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Output};
use std::thread;

fn scenario(args: &[&str], path: &str, status: u16, body: &str) -> (Output, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let output = thread::scope(|scope| {
        let server = scope.spawn(|| {
            let (mut connection, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                assert_eq!(connection.read(&mut byte).unwrap(), 1);
                request.push(byte[0]);
            }
            let request = String::from_utf8(request).unwrap();
            let method = match args[1] {
                "archive" => "POST",
                "delete" => "DELETE",
                _ => "GET",
            };
            assert!(request.starts_with(&format!("{method} {path} HTTP/1.1\r\n")), "{request}");
            write!(connection, "HTTP/1.1 {status} Stub\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        });
        let output = Command::new(env!("CARGO_BIN_EXE_tumpel"))
            .args(args)
            .env("TUMPEL_ENDPOINT", &endpoint)
            .env("TUMPEL_TOKEN", "scenario-token")
            .env("TUMPEL_ENV_FILE", "/dev/null")
            .env_remove("HTTP_PROXY")
            .env_remove("HTTPS_PROXY")
            .env("NO_PROXY", "127.0.0.1")
            .output()
            .unwrap();
        server.join().unwrap();
        output
    });
    let notice = format!("the server sent a response this CLI doesn't recognise, please update the CLI: curl -fsSL {endpoint}/v1/releases/install.sh | sh\n");
    (output, notice)
}

fn session() -> Value {
    json!({
        "sessionId": "session-1", "tenantId": "tenant-1", "principalId": "principal-1",
        "hostId": "host-1", "boxId": "box-1", "personaId": "persona-1",
        "repository": "https://github.com/tumpel/cli", "origin": {"kind": "Prompt"},
        "startedBy": {"kind": "Person", "principalId": "principal-1"},
        "tags": [], "harness": "ClaudeCode", "harnessVersion": "1", "model": "sonnet",
        "state": "Idle", "lastTurnId": "turn-1", "lastTurnState": "Completed",
        "lastTurnAt": "2026-11-21T00:00:00Z", "boxRetention": "Warm",
        "lastTurn": {
            "turnId": "turn-1", "tenantId": "tenant-1", "sessionId": "session-1", "boxId": "box-1",
            "prompt": "build", "state": "Completed", "lineCount": 0, "createdAt": 1, "updatedAt": 2
        },
        "createdAt": 1, "updatedAt": 2
    })
}

#[test]
fn session_list_preserves_known_and_unknown_rows_with_one_update_message() {
    let known = session();
    let mut future = known.clone();
    future["state"] = json!("FutureState");
    future["extra"] = json!({"future": true});
    let body = json!([known, future, {"state": "AnotherFutureState"}]);
    let (output, notice) = scenario(&["session", "list"], "/v1/sessions", 200, &body.to_string());
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stderr).unwrap(), notice);
    assert_eq!(serde_json::from_slice::<Value>(&output.stdout).unwrap(), body);
}

#[test]
fn session_get_preserves_an_unknown_enum_or_field_shape_with_one_update_message() {
    for changed in [json!("FutureState"), json!({"future": "shape"})] {
        let mut body = session();
        body["state"] = changed;
        let (output, notice) = scenario(
            &["session", "get", "session-1"],
            "/v1/sessions/session-1",
            200,
            &body.to_string(),
        );
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(String::from_utf8(output.stderr).unwrap(), notice);
        assert_eq!(serde_json::from_slice::<Value>(&output.stdout).unwrap(), body);
    }
}

#[test]
fn sessions_fail_with_code_three_and_only_the_update_message_on_unreadable_responses() {
    for (args, path) in [
        (vec!["session", "list"], "/v1/sessions"),
        (vec!["session", "get", "session-1"], "/v1/sessions/session-1"),
    ] {
        for (status, body) in [
            (299, "{}"),
            (503, "<html>future response</html>"),
            (200, "not JSON"),
            (200, "null"),
        ] {
            let (output, notice) = scenario(&args, path, status, body);
            assert_eq!(output.status.code(), Some(3));
            assert_eq!(String::from_utf8(output.stderr).unwrap(), notice);
            assert_eq!(output.stdout, b"");
        }
    }
}

#[test]
fn recognised_session_responses_do_not_request_an_update() {
    let mut body = session();
    body["extra"] = json!({"future": true});
    let (output, _) = scenario(
        &["session", "get", "session-1"],
        "/v1/sessions/session-1",
        200,
        &body.to_string(),
    );
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty(), "{}", String::from_utf8_lossy(&output.stderr));
    body.as_object_mut().unwrap().remove("extra");
    assert_eq!(serde_json::from_slice::<Value>(&output.stdout).unwrap(), body);
}

#[test]
fn session_get_preserves_an_unknown_failure_reason() {
    let mut body = session();
    body["lastTurn"]["state"] = json!("Failed");
    body["lastTurn"]["reason"] = json!("FutureFailureReason");
    body["lastTurn"]["detail"] = json!("future failure");
    body["lastTurn"]["failedAt"] = json!("2026-11-21T00:00:00Z");
    let (output, notice) = scenario(
        &["session", "get", "session-1"],
        "/v1/sessions/session-1",
        200,
        &body.to_string(),
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stderr).unwrap(), notice);
    assert_eq!(serde_json::from_slice::<Value>(&output.stdout).unwrap(), body);
}

#[test]
fn an_unknown_session_refusal_prints_only_the_update_message() {
    let body = r#"{"code":"FutureRefusal","title":"New refusal","detail":"future detail","fix":"future fix"}"#;
    let (output, notice) = scenario(&["session", "get", "session-1"], "/v1/sessions/session-1", 400, body);
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(String::from_utf8(output.stderr).unwrap(), notice);
    assert_eq!(output.stdout, b"");
}

#[test]
fn session_delete_and_bulk_archive_stop_on_unrecognised_responses() {
    for (args, path) in [
        (vec!["session", "delete", "session-1"], "/v1/sessions/session-1"),
        (
            vec!["session", "archive", "session-1", "session-2"],
            "/v1/sessions/session-1/archive",
        ),
    ] {
        let body = r#"{"code":"FutureRefusal","title":"New refusal","detail":"future detail","fix":"future fix"}"#;
        let (output, notice) = scenario(&args, path, 400, body);
        assert_eq!(output.status.code(), Some(3));
        assert_eq!(String::from_utf8(output.stderr).unwrap(), notice);
        assert_eq!(output.stdout, b"");
    }
}

#[test]
fn a_known_session_refusal_keeps_its_explanation_and_exit_code() {
    let body = r#"{"code":"GitHubAppPermissionMissing","title":"Permission missing","detail":"cannot read the repository","fix":"grant access"}"#;
    let (output, _) = scenario(&["session", "get", "session-1"], "/v1/sessions/session-1", 400, body);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "tumpel: Permission missing: cannot read the repository. grant access\n"
    );
    assert_eq!(output.stdout, b"");
}
