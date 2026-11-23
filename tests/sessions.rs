use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant};

fn scenario(args: &[&str], path: &str, status: u16, body: &str) -> (Output, String) {
    responses(args, &[(path, status, body)])
}

fn responses(args: &[&str], replies: &[(&str, u16, &str)]) -> (Output, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let endpoint = format!("http://{address}");
    let env_file = std::env::temp_dir().join(format!("tumpel-{}-{}.env", std::process::id(), address.port()));
    let output = thread::scope(|scope| {
        let server = scope.spawn(move || {
            for (path, status, body) in replies {
                let deadline = Instant::now() + Duration::from_secs(5);
                let (mut connection, _) = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                        {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => panic!("waiting for {path}: {error}"),
                    }
                };
                connection.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    assert_eq!(connection.read(&mut byte).unwrap(), 1);
                    request.push(byte[0]);
                }
                let request = String::from_utf8(request).unwrap();
                let length = request.lines().find_map(|line| {
                    line.to_ascii_lowercase().strip_prefix("content-length: ").map(|length| length.parse::<usize>().unwrap())
                }).unwrap_or(0);
                connection.read_exact(&mut vec![0; length]).unwrap();
                let method = match args.get(1).copied().unwrap_or("login") {
                    "archive" | "login" | "plan" | "upload-file" => "POST",
                    "delete" => "DELETE",
                    _ => "GET",
                };
                assert!(request.starts_with(&format!("{method} {path} HTTP/1.1\r\n")), "{request}");
                write!(connection, "HTTP/1.1 {status} Stub\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let output = Command::new(env!("CARGO_BIN_EXE_tumpel"))
            .args(args)
            .env("TUMPEL_ENDPOINT", &endpoint)
            .env("TUMPEL_TOKEN", "scenario-token")
            .env("TUMPEL_ENV_FILE", &env_file)
            .env_remove("TUMPEL_PROOF_CLIENT_ID")
            .env_remove("TUMPEL_PROOF_CLIENT_SECRET")
            .env_remove("HTTP_PROXY")
            .env_remove("HTTPS_PROXY")
            .env("NO_PROXY", "127.0.0.1")
            .output()
            .unwrap();
        server.join().unwrap();
        output
    });
    if env_file.exists() {
        std::fs::remove_file(env_file).unwrap();
    }
    let notice = format!("the server sent a response this CLI doesn't recognise, please update the CLI: curl -fsSL {endpoint}/v1/releases/install.sh | sh\n");
    (output, notice)
}

#[test]
fn session_list_preserves_known_and_unknown_rows_with_one_update_message() {
    let known = json!({
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
    });
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
        let mut body = json!({
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
        });
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
        for (status, body) in [(299, "{}"), (503, "<html>future response</html>"), (200, "not JSON")] {
            let (output, notice) = scenario(&args, path, status, body);
            assert_eq!(output.status.code(), Some(3));
            assert_eq!(String::from_utf8(output.stderr).unwrap(), notice);
            assert_eq!(output.stdout, b"");
        }
    }
}

#[test]
fn recognised_session_responses_do_not_request_an_update() {
    let mut body = json!({
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
    });
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
    let mut body = json!({
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
    });
    body["lastTurn"]["state"] = json!("Failed");
    body["lastTurn"]["reason"] = json!("FutureFailureReason");
    body["lastTurn"]["detail"] = json!("future failure");
    body["lastTurn"]["failedAt"] = json!("2026-11-21T00:00:00Z");
    let tumpel_api::types::Session::IdleSession(decoded) = serde_json::from_value(body.clone()).unwrap() else {
        panic!("expected a known session");
    };
    assert!(
        matches!(decoded.last_turn, tumpel_api::types::Turn::FailedTurn(turn) if matches!(turn.reason, tumpel_api::types::TurnFailureReason::Unrecognised(_)))
    );
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

#[test]
fn login_start_stops_on_unknown_and_unreadable_responses() {
    for (status, body) in [
        (400, r#"{"error":"future_error","error_description":"future refusal"}"#),
        (200, r#"{"device_code":{"future":true}}"#),
    ] {
        let (output, notice) = scenario(&["login"], "/v1/device/code", status, body);
        assert_eq!(output.status.code(), Some(3));
        assert_eq!(String::from_utf8(output.stderr).unwrap(), notice);
        assert_eq!(output.stdout, b"");
    }
}

#[test]
fn login_poll_stops_on_unknown_and_unreadable_responses() {
    for (status, body) in [
        (400, r#"{"error":"future_error","error_description":"future refusal"}"#),
        (200, r#"{"access_token":"token","token_type":"FutureToken"}"#),
        (200, r#"{"access_token":{"future":true}}"#),
    ] {
        let (output, notice) = responses(
            &["login"],
            &[
                (
                    "/v1/device/code",
                    200,
                    r#"{"device_code":"device","user_code":"code","verification_uri":"https://example.test/login","verification_uri_complete":"https://example.test/login?code=code","expires_in":60,"interval":1}"#,
                ),
                ("/v1/device/token", status, body),
            ],
        );
        assert_eq!(output.status.code(), Some(3));
        assert_eq!(String::from_utf8(output.stderr).unwrap(), format!("Open https://example.test/login and enter the code code\n(or open https://example.test/login?code=code)\n{notice}"));
        assert_eq!(output.stdout, b"");
    }
}

#[test]
fn bundle_upload_stops_on_unknown_and_unreadable_responses() {
    let file = std::env::temp_dir().join(format!("tumpel-{}-upload.txt", std::process::id()));
    std::fs::write(&file, "persona file").unwrap();
    for (status, body) in [
        (
            400,
            r#"{"code":"FutureRefusal","title":"New refusal","detail":"future detail","fix":"future fix"}"#,
        ),
        (200, r#"{"uploadUrl":{"future":true}}"#),
    ] {
        let (output, notice) = scenario(
            &["persona", "upload-file", file.to_str().unwrap()],
            "/v1/personas/file-uploads",
            status,
            body,
        );
        assert_eq!(output.status.code(), Some(3));
        assert_eq!(String::from_utf8(output.stderr).unwrap(), notice);
        assert_eq!(output.stdout, b"");
    }
    std::fs::remove_file(file).unwrap();
}

#[test]
fn config_plan_keeps_descriptions_of_unknown_changes_and_stops_on_unreadable_responses() {
    let file = std::env::temp_dir().join(format!("tumpel-{}-plan.json", std::process::id()));
    std::fs::write(&file, r#"{"personas":[],"secretSets":[],"harnessPins":[],"hosts":[]}"#).unwrap();
    let body = json!({"changes": [
        {"action":"Create","resource":"Persona","name":"known","description":"create known persona"},
        {"action":"FutureAction","resource":"FutureResource","name":"future","description":"future change"}
    ]});
    let (output, notice) = scenario(
        &["config", "plan", file.to_str().unwrap()],
        "/v1/tenant-config/plan",
        200,
        &body.to_string(),
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stderr).unwrap(), notice);
    assert_eq!(output.stdout, b"create known persona\nfuture change\n");
    for (status, body) in [
        (
            400,
            r#"{"code":"FutureRefusal","title":"New refusal","detail":"future detail","fix":"future fix"}"#,
        ),
        (200, r#"{"changes":{"future":true}}"#),
    ] {
        let (output, notice) = scenario(
            &["config", "plan", file.to_str().unwrap()],
            "/v1/tenant-config/plan",
            status,
            body,
        );
        assert_eq!(output.status.code(), Some(3));
        assert_eq!(String::from_utf8(output.stderr).unwrap(), notice);
        assert_eq!(output.stdout, b"");
    }
    std::fs::remove_file(file).unwrap();
}
