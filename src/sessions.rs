use crate::{
    answered, print_json, problem_text, response, shown, signed_in, ApiResponse, SessionCommand, SnapshotCommand,
    TranscriptArgs, TranscriptCommand, TurnCommand,
};
use anyhow::{bail, Result};
use chrono::{DateTime, FixedOffset};
use std::num::NonZeroU32;
use std::str::FromStr;
use std::time::Duration;
use tumpel_api::types::{
    BoxRetention, EntityId, Problem, Session, SessionStart, SessionStartFromIssue, SessionState, SessionTag,
    SessionTags, SessionTagsPatch, StartedSession, StartedSessionFromIssue, SteerCreate, TranscriptPage, Turn,
    TurnFailureReason, TurnSend,
};
use tumpel_api::{Client, Error, ResponseValue};

const FIRST_BACKOFF: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(5);
const PAGE_LINES: NonZeroU32 = NonZeroU32::new(1000).expect("1000 is not zero");

pub(crate) fn parsed<T>(flag: &str, value: &str) -> Result<T>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    value
        .parse()
        .map_err(|error| anyhow::anyhow!("{flag} {value:?} is not valid: {error}"))
}

pub(crate) fn retention(value: Option<&str>) -> Result<Option<BoxRetention>> {
    value.map(|value| parsed("--box-retention", value)).transpose()
}

pub(crate) fn id(flag: &str, value: &str) -> Result<EntityId> {
    parsed(flag, value)
}

pub(crate) fn tags(flag: &str, values: &[String]) -> Result<Vec<SessionTag>> {
    values.iter().map(|value| parsed(flag, value)).collect()
}

fn session_parts<'a>(
    session: &'a Session,
    endpoint: &str,
) -> Result<(&'a EntityId, &'a Turn), crate::UnrecognisedResponse> {
    Ok(match session {
        Session::StartingSession(session) => (&session.session_id, &session.last_turn),
        Session::IdleSession(session) => (&session.session_id, &session.last_turn),
        Session::RunningSession(session) => (&session.session_id, &session.last_turn),
        Session::StoppingSession(session) => (&session.session_id, &session.last_turn),
        Session::StoppedSession(session) => (&session.session_id, &session.last_turn),
        Session::ArchivedSession(session) => (&session.session_id, &session.last_turn),
        Session::FailedSession(session) => (&session.session_id, &session.last_turn),
        Session::UnknownSession(session) => (&session.session_id, &session.last_turn),
        Session::Unrecognised(_) => return Err(crate::unrecognised_response(endpoint)),
    })
}

trait SessionOutput: serde::Serialize {
    fn sessions(&self) -> &[Session];
}

impl SessionOutput for Session {
    fn sessions(&self) -> &[Session] {
        std::slice::from_ref(self)
    }
}

impl SessionOutput for Vec<Session> {
    fn sessions(&self) -> &[Session] {
        self
    }
}

impl SessionOutput for StartedSession {
    fn sessions(&self) -> &[Session] {
        std::slice::from_ref(&self.session)
    }
}

impl SessionOutput for StartedSessionFromIssue {
    fn sessions(&self) -> &[Session] {
        std::slice::from_ref(&self.session)
    }
}

fn shown_sessions<T: SessionOutput>(
    outcome: Result<ResponseValue<T>, Error<Problem>>,
    call: impl FnOnce() -> String,
) -> Result<serde_json::Value> {
    let call = call();
    let outcome = response(outcome, || call.clone())?;
    session_output(outcome, &call)
}

fn session_output<T: SessionOutput>(outcome: ApiResponse<T, Problem>, call: &str) -> Result<serde_json::Value> {
    if let ApiResponse::Answer(answer) = &outcome {
        for session in answer.sessions() {
            match session_parts(session, call) {
                Err(response) => crate::update_message(&response),
                Ok((_, turn)) => {
                    if matches!(turn, Turn::Unrecognised(_))
                        || matches!(turn, Turn::FailedTurn(turn) if matches!(turn.reason, TurnFailureReason::Unrecognised(_)))
                    {
                        crate::update_message(&crate::unrecognised_response(call));
                    }
                }
            }
        }
    }
    outcome.shown()
}

async fn set_tags(client: &Client, endpoint: &str, session_id: &str, values: &[String]) -> Result<()> {
    let session = id("sessionId", session_id)?;
    let body = SessionTags {
        tags: tags("tag", values)?,
    };
    print_json(&shown_sessions(
        client.sessions_set_tags(&session, &body).await,
        || format!("{endpoint}/v1/sessions/{session_id}/tags"),
    )?)
}

async fn patch_tags(
    client: &Client,
    endpoint: &str,
    session_id: &str,
    add: &[String],
    remove: &[String],
) -> Result<()> {
    let session = id("sessionId", session_id)?;
    let body = SessionTagsPatch {
        add: tags("--add", add)?,
        remove: tags("--remove", remove)?,
    };
    print_json(&shown_sessions(
        client.sessions_patch_tags(&session, &body).await,
        || format!("{endpoint}/v1/sessions/{session_id}/tags"),
    )?)
}

pub async fn session(command: SessionCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    match command {
        SessionCommand::Start {
            repository,
            model,
            persona_id,
            prompt,
            box_retention,
            tag,
        } => {
            let body = SessionStart {
                box_retention: retention(box_retention.as_deref())?,
                model: parsed("--model", &model)?,
                persona_id: id("--persona-id", &persona_id)?,
                prompt: parsed("--prompt", &prompt)?,
                repository: parsed("--repository", &repository)?,
                tags: tags("--tag", &tag)?,
            };
            print_json(&shown_sessions(client.sessions_start(&body).await, || {
                format!("{endpoint}/v1/sessions")
            })?)
        }
        SessionCommand::StartFromIssue {
            owner,
            name,
            issue_number,
            model,
            persona_id,
            box_retention,
            tag,
        } => {
            let body = SessionStartFromIssue {
                box_retention: retention(box_retention.as_deref())?,
                issue_number: parsed("--issue-number", &issue_number)?,
                model: parsed("--model", &model)?,
                persona_id: id("--persona-id", &persona_id)?,
                owner: parsed("--owner", &owner)?,
                name: parsed("--name", &name)?,
                tags: tags("--tag", &tag)?,
            };
            print_json(&shown_sessions(client.sessions_start_from_issue(&body).await, || {
                format!("{endpoint}/v1/sessions/from-issue")
            })?)
        }
        SessionCommand::Get { session_id } => {
            let session = id("sessionId", &session_id)?;
            print_json(&shown_sessions(client.sessions_get(&session).await, || {
                format!("{endpoint}/v1/sessions/{session_id}")
            })?)
        }
        SessionCommand::SetTags { session_id, tags } => set_tags(&client, &endpoint, &session_id, &tags).await,
        SessionCommand::PatchTags {
            session_id,
            add,
            remove,
        } => patch_tags(&client, &endpoint, &session_id, &add, &remove).await,
        SessionCommand::StopFailures { session_id } => {
            let session = id("sessionId", &session_id)?;
            print_json(&shown(client.sessions_list_stop_failures(&session).await, || {
                format!("{endpoint}/v1/sessions/{session_id}/stop-failures")
            })?)
        }
        SessionCommand::Stop {
            session_id,
            cancel_turn,
        } => {
            let session = id("sessionId", &session_id)?;
            print_json(&shown_sessions(
                client.sessions_stop(&session, cancel_turn.then_some(true)).await,
                || format!("{endpoint}/v1/sessions/{session_id}/stop"),
            )?)
        }
        SessionCommand::Archive {
            session_ids,
            state,
            last_turn_before,
        } => archive(&client, &endpoint, &session_ids, state, last_turn_before).await,
        SessionCommand::Delete { session_id } => delete(&client, &endpoint, &session_id).await,
        SessionCommand::Handovers { session_id } => {
            let session = id("sessionId", &session_id)?;
            print_json(&shown(
                client.sessions_list_credential_handovers(&session).await,
                || format!("{endpoint}/v1/sessions/{session_id}/credential-handovers"),
            )?)
        }
        SessionCommand::Transcript { session_id, read } => {
            let session = id("sessionId", &session_id)?;
            print_transcript(&client, &endpoint, session, None, read).await
        }
        SessionCommand::List {
            state,
            host_id,
            last_turn_before,
            tag,
            trigger,
        } => list(&client, &endpoint, state, host_id, last_turn_before, &tag, trigger).await,
    }
}

async fn list(
    client: &Client,
    endpoint: &str,
    state: Option<String>,
    host_id: Option<String>,
    last_turn_before: Option<String>,
    tag: &[String],
    trigger: Option<String>,
) -> Result<()> {
    let state = state
        .map(|value| parsed::<SessionState>("--state", &value))
        .transpose()?;
    let host = host_id.map(|value| id("--host-id", &value)).transpose()?;
    let before = last_turn_before
        .map(|value| parsed::<DateTime<FixedOffset>>("--last-turn-before", &value))
        .transpose()?;
    let tags = tags("--tag", tag)?;
    let trigger = trigger.map(|value| id("--trigger", &value)).transpose()?;
    print_json(&shown_sessions(
        client
            .sessions_list(
                host.as_ref(),
                before.as_ref(),
                state,
                (!tags.is_empty()).then_some(&tags),
                trigger.as_ref(),
            )
            .await,
        || format!("{endpoint}/v1/sessions"),
    )?)
}

pub async fn turn(command: TurnCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    match command {
        TurnCommand::Send {
            session_id,
            prompt,
            goal,
        } => {
            let session = id("sessionId", &session_id)?;
            let body = TurnSend {
                prompt: parsed("--prompt", &prompt)?,
                goal: goal.map(|value| parsed("--goal", &value)).transpose()?,
            };
            print_json(&shown(client.turns_send(&session, &body).await, || {
                format!("{endpoint}/v1/sessions/{session_id}/turns")
            })?)
        }
        TurnCommand::Get { turn_id } => {
            let turn = id("turnId", &turn_id)?;
            print_json(&shown(client.turns_get(&turn).await, || {
                format!("{endpoint}/v1/turns/{turn_id}")
            })?)
        }
        TurnCommand::List { session_id } => {
            let session = id("sessionId", &session_id)?;
            print_json(&shown(client.turns_list(&session).await, || {
                format!("{endpoint}/v1/sessions/{session_id}/turns")
            })?)
        }
        TurnCommand::Cancel { turn_id } => {
            let turn = id("turnId", &turn_id)?;
            print_json(&shown(client.turns_cancel(&turn).await, || {
                format!("{endpoint}/v1/turns/{turn_id}/cancel")
            })?)
        }
        TurnCommand::Steer { turn_id, prompt } => {
            let turn = id("turnId", &turn_id)?;
            let body = SteerCreate {
                prompt: parsed("--prompt", &prompt)?,
            };
            print_json(&shown(client.turns_steer(&turn, &body).await, || {
                format!("{endpoint}/v1/turns/{turn_id}/steers")
            })?)
        }
        TurnCommand::ListSteers { turn_id } => {
            let turn = id("turnId", &turn_id)?;
            print_json(&shown(client.turns_list_steers(&turn).await, || {
                format!("{endpoint}/v1/turns/{turn_id}/steers")
            })?)
        }
        TurnCommand::Failure { turn_id } => {
            let turn = id("turnId", &turn_id)?;
            print_json(&shown(client.turns_get_failure(&turn).await, || {
                format!("{endpoint}/v1/turns/{turn_id}/failure")
            })?)
        }
    }
}

pub async fn snapshot(command: SnapshotCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    match command {
        SnapshotCommand::List { session_id } => {
            let session = id("sessionId", &session_id)?;
            print_json(&shown(client.snapshots_list(&session).await, || {
                format!("{endpoint}/v1/sessions/{session_id}/snapshots")
            })?)
        }
        SnapshotCommand::Get { snapshot_id } => {
            let snapshot = id("snapshotId", &snapshot_id)?;
            print_json(&shown(client.snapshots_get(&snapshot).await, || {
                format!("{endpoint}/v1/snapshots/{snapshot_id}")
            })?)
        }
    }
}

async fn delete(client: &Client, endpoint: &str, session_id: &str) -> Result<()> {
    let session = id("sessionId", session_id)?;
    answered(client.sessions_delete(&session).await, || {
        format!("{endpoint}/v1/sessions/{session_id}")
    })?;
    eprintln!("deleted session {session_id}");
    Ok(())
}

async fn archive(
    client: &Client,
    endpoint: &str,
    session_ids: &[String],
    state: Option<String>,
    last_turn_before: Option<String>,
) -> Result<()> {
    if let [session_id] = session_ids {
        let session = id("sessionId", session_id)?;
        return print_json(&shown_sessions(client.sessions_archive(&session).await, || {
            format!("{endpoint}/v1/sessions/{session_id}/archive")
        })?);
    }
    let sessions = if session_ids.is_empty() {
        let state = state
            .map(|value| parsed::<SessionState>("--state", &value))
            .transpose()?;
        let before = last_turn_before
            .map(|value| parsed::<DateTime<FixedOffset>>("--last-turn-before", &value))
            .transpose()?;
        answered(
            client.sessions_list(None, before.as_ref(), state, None, None).await,
            || format!("{endpoint}/v1/sessions"),
        )?
        .into_iter()
        .map(|session| Ok(session_parts(&session, endpoint)?.0.clone()))
        .collect::<Result<Vec<_>>>()?
    } else {
        session_ids
            .iter()
            .map(|value| id("sessionId", value))
            .collect::<Result<Vec<_>>>()?
    };
    let mut archived = Vec::new();
    let mut refused = Vec::new();
    for session in &sessions {
        match response(client.sessions_archive(session).await, || {
            format!("{endpoint}/v1/sessions/{}/archive", **session)
        })? {
            ApiResponse::Refusal(problem) => {
                refused.push(format!("session {}: {}", **session, problem_text(&problem)));
            }
            outcome => archived.push(session_output(outcome, endpoint)?),
        }
    }
    print_json(&archived)?;
    if !refused.is_empty() {
        bail!(
            "{} of {} sessions were not archived:\n{}",
            refused.len(),
            sessions.len(),
            refused.join("\n")
        );
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum Spent {
    Unlimited,
    Left(NonZeroU32),
    Done,
}

fn spend(remaining: Option<NonZeroU32>, available: usize) -> (usize, Spent) {
    let Some(left) = remaining else {
        return (available, Spent::Unlimited);
    };
    let wanted = usize::try_from(left.get()).unwrap_or(usize::MAX);
    let taken = available.min(wanted);
    let rest = u32::try_from(wanted - taken).unwrap_or(u32::MAX);
    (taken, NonZeroU32::new(rest).map_or(Spent::Done, Spent::Left))
}

pub async fn transcript(command: TranscriptCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    let TranscriptCommand::Read { turn_id, read } = command;
    let turn = id("turnId", &turn_id)?;
    let session = match answered(client.turns_get(&turn).await, || {
        format!("{endpoint}/v1/turns/{turn_id}")
    })? {
        Turn::OfferedTurn(read) => read.session_id,
        Turn::RunningTurn(read) => read.session_id,
        Turn::CompletedTurn(read) => read.session_id,
        Turn::FailedTurn(read) => read.session_id,
        Turn::UnknownTurn(read) => read.session_id,
        Turn::Unrecognised(_) => return Err(crate::unrecognised_response(&endpoint).into()),
    };
    print_transcript(&client, &endpoint, session, Some(turn), read).await
}

fn headed_lines(
    texts: impl IntoIterator<Item = (EntityId, String)>,
    compact: bool,
    headed: &mut Option<EntityId>,
) -> Vec<String> {
    let mut lines = Vec::new();
    for (turn_id, text) in texts {
        if compact && headed.as_ref() != Some(&turn_id) {
            lines.push(format!("turn {}", *turn_id));
            *headed = Some(turn_id);
        }
        lines.push(text);
    }
    lines
}

async fn print_transcript(
    client: &Client,
    endpoint: &str,
    session: EntityId,
    turn: Option<EntityId>,
    read: TranscriptArgs,
) -> Result<()> {
    let mut cursor = read.after.into();
    let mut backoff = FIRST_BACKOFF;
    let mut remaining = read.limit;
    let mut headed: Option<EntityId> = None;
    loop {
        let page = answered(
            client
                .transcripts_read(
                    &session,
                    Some(&cursor),
                    Some(read.compact),
                    remaining.map(|left| left.min(PAGE_LINES)),
                    read.max_bytes,
                    turn.as_ref(),
                )
                .await,
            || format!("{endpoint}/v1/sessions/{}/transcript", *session),
        )?;
        let (texts, page_cursor, finished) = match page {
            TranscriptPage::Unrecognised(_) => return Err(crate::unrecognised_response(endpoint).into()),
            TranscriptPage::RawTranscriptPage(raw) => (
                raw.lines
                    .into_iter()
                    .map(|line| (line.turn_id, line.text))
                    .collect::<Vec<_>>(),
                raw.cursor,
                raw.finished,
            ),
            TranscriptPage::CompactTranscriptPage(compact) => (
                compact
                    .turns
                    .into_iter()
                    .flat_map(|turn| {
                        let turn_id = turn.turn_id;
                        turn.lines.into_iter().map(move |line| (turn_id.clone(), line.text))
                    })
                    .collect(),
                compact.cursor,
                compact.finished,
            ),
        };
        let (printed, spent) = spend(remaining, texts.len());
        for line in headed_lines(
            texts
                .into_iter()
                .take(printed)
                .map(|(turn, text)| (turn, text.to_string())),
            read.compact,
            &mut headed,
        ) {
            println!("{line}");
        }
        remaining = match spent {
            Spent::Done => return Ok(()),
            Spent::Left(left) => Some(left),
            Spent::Unlimited => None,
        };
        let advanced = page_cursor != cursor;
        cursor = page_cursor;
        if finished {
            if !read.follow {
                return Ok(());
            }
            let last = if let Some(turn) = turn {
                answered(client.turns_get(&turn).await, || {
                    format!("{endpoint}/v1/turns/{}", *turn)
                })?
            } else {
                let answer = answered(client.sessions_get(&session).await, || {
                    format!("{endpoint}/v1/sessions/{}", *session)
                })?;
                session_parts(&answer, endpoint)?.1.clone()
            };
            if let Some(warning) = turn_outcome(last, endpoint)? {
                eprintln!("{warning}");
            }
            return Ok(());
        }
        if advanced {
            backoff = FIRST_BACKOFF;
            continue;
        }
        if !read.follow {
            return Ok(());
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

fn turn_outcome(last: Turn, endpoint: &str) -> Result<Option<String>> {
    match last {
        Turn::Unrecognised(_) => Err(crate::unrecognised_response(endpoint).into()),
        Turn::FailedTurn(failed) if matches!(failed.reason, TurnFailureReason::Unrecognised(_)) => {
            Err(crate::unrecognised_response(endpoint).into())
        }
        Turn::FailedTurn(failed) => bail!(
            "turn {} failed with {}: {}",
            *failed.turn_id,
            failed.reason,
            failed.detail
        ),
        Turn::CompletedTurn(completed) => Ok(completed
            .warning
            .map(|warning| format!("turn {} completed with a warning: {}", *completed.turn_id, *warning))),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::{headed_lines, spend, turn_outcome, Spent};
    use std::num::NonZeroU32;
    use tumpel_api::types::EntityId;

    fn lines(count: u32) -> NonZeroU32 {
        NonZeroU32::new(count).expect("a test limit is not zero")
    }

    #[test]
    fn a_limit_is_the_total_printed_across_pages() {
        assert_eq!(spend(Some(lines(3)), 1000), (3, Spent::Done));
        assert_eq!(spend(Some(lines(3)), 2), (2, Spent::Left(lines(1))));
        assert_eq!(spend(Some(lines(1)), 1), (1, Spent::Done));
        assert_eq!(spend(Some(lines(5)), 0), (0, Spent::Left(lines(5))));
    }

    #[test]
    fn no_limit_prints_every_line_of_every_page() {
        assert_eq!(spend(None, 1000), (1000, Spent::Unlimited));
    }

    fn turn(id: &str) -> EntityId {
        id.parse().expect("a test turn id is valid")
    }

    #[test]
    fn a_compact_session_read_names_each_turn_once_above_its_lines_across_pages() {
        let mut headed = None;
        let first_page = headed_lines(
            [
                (turn("01J00000000000000000000001"), "read the issue".to_owned()),
                (turn("01J00000000000000000000001"), "opened a pull request".to_owned()),
                (turn("01J00000000000000000000002"), "fixed the review".to_owned()),
            ],
            true,
            &mut headed,
        );
        let second_page = headed_lines(
            [
                (turn("01J00000000000000000000002"), "pushed the fix".to_owned()),
                (turn("01J00000000000000000000003"), "nothing left to do".to_owned()),
            ],
            true,
            &mut headed,
        );
        assert_eq!(
            first_page,
            [
                "turn 01J00000000000000000000001",
                "read the issue",
                "opened a pull request",
                "turn 01J00000000000000000000002",
                "fixed the review",
            ]
        );
        assert_eq!(
            second_page,
            [
                "pushed the fix",
                "turn 01J00000000000000000000003",
                "nothing left to do"
            ]
        );
    }

    #[test]
    fn a_followed_transcript_ends_with_a_completed_turn_warning_and_fails_on_a_failed_turn() {
        let turn = |extra: serde_json::Value| -> tumpel_api::types::Turn {
            let mut read = serde_json::json!({
                "turnId": "turn-1", "tenantId": "tenant-1", "sessionId": "session-1", "boxId": "box-1",
                "prompt": "build", "lineCount": 1, "createdAt": 1, "updatedAt": 2,
            });
            read.as_object_mut()
                .expect("an object")
                .extend(extra.as_object().expect("an object").clone());
            serde_json::from_value(read).expect("a turn")
        };
        let warning = "the box disk of 32096 MiB is full, 12 MiB free; give the persona a larger boxDiskMib";
        assert_eq!(
            turn_outcome(
                turn(serde_json::json!({ "state": "Completed", "warning": warning })),
                "https://example.test"
            )
            .expect("completed"),
            Some(format!("turn turn-1 completed with a warning: {warning}"))
        );
        assert_eq!(
            turn_outcome(
                turn(serde_json::json!({ "state": "Completed" })),
                "https://example.test"
            )
            .expect("completed"),
            None
        );
        let failed = turn_outcome(
            turn(serde_json::json!({
                "state": "Failed", "reason": "BoxDiskFull", "detail": warning, "failedAt": "2026-11-18T10:00:00+00:00",
            })),
            "https://example.test",
        )
        .expect_err("a failed turn fails the read");
        assert_eq!(
            failed.to_string(),
            format!("turn turn-1 failed with BoxDiskFull: {warning}")
        );
    }
}
