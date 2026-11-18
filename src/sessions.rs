use crate::{
    answered, print_json, problem_text, shown, signed_in, SessionCommand, SnapshotCommand, TranscriptArgs,
    TranscriptCommand, TurnCommand,
};
use anyhow::{bail, Result};
use chrono::{DateTime, FixedOffset};
use std::num::NonZeroU32;
use std::str::FromStr;
use std::time::Duration;
use tumpel_api::types::{EntityId, SessionStart, SessionStartFromIssue, SessionState, TranscriptPage, Turn, TurnSend};
use tumpel_api::{Client, Error};

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

pub(crate) fn id(flag: &str, value: &str) -> Result<EntityId> {
    parsed(flag, value)
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
        } => {
            let body = SessionStart {
                model: parsed("--model", &model)?,
                persona_id: id("--persona-id", &persona_id)?,
                prompt: parsed("--prompt", &prompt)?,
                repository: parsed("--repository", &repository)?,
            };
            print_json(&shown(client.sessions_start(&body).await, || {
                format!("{endpoint}/v1/sessions")
            })?)
        }
        SessionCommand::StartFromIssue {
            owner,
            name,
            issue_number,
            model,
            persona_id,
        } => {
            let body = SessionStartFromIssue {
                issue_number: parsed("--issue-number", &issue_number)?,
                model: parsed("--model", &model)?,
                persona_id: id("--persona-id", &persona_id)?,
                owner: parsed("--owner", &owner)?,
                name: parsed("--name", &name)?,
            };
            print_json(&shown(client.sessions_start_from_issue(&body).await, || {
                format!("{endpoint}/v1/sessions/from-issue")
            })?)
        }
        SessionCommand::Get { session_id } => {
            let session = id("sessionId", &session_id)?;
            print_json(&shown(client.sessions_get(&session).await, || {
                format!("{endpoint}/v1/sessions/{session_id}")
            })?)
        }
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
            print_json(&shown(
                client.sessions_stop(&session, cancel_turn.then_some(true)).await,
                || format!("{endpoint}/v1/sessions/{session_id}/stop"),
            )?)
        }
        SessionCommand::Archive {
            session_ids,
            state,
            last_turn_before,
        } => archive(&client, &endpoint, &session_ids, state, last_turn_before).await,
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
        } => {
            let state = state
                .map(|value| parsed::<SessionState>("--state", &value))
                .transpose()?;
            let host = host_id.map(|value| id("--host-id", &value)).transpose()?;
            let before = last_turn_before
                .map(|value| parsed::<DateTime<FixedOffset>>("--last-turn-before", &value))
                .transpose()?;
            print_json(&shown(
                client.sessions_list(host.as_ref(), before.as_ref(), state).await,
                || format!("{endpoint}/v1/sessions"),
            )?)
        }
    }
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
        TurnCommand::Failure { turn_id } => {
            let turn = id("turnId", &turn_id)?;
            print_json(&shown(client.turns_get_failure(&turn).await, || {
                format!("{endpoint}/v1/turns/{turn_id}/failure")
            })?)
        }
        TurnCommand::Wait {
            turn_ids,
            timeout_seconds,
        } => {
            let turns = turn_ids
                .iter()
                .map(|turn_id| id("turnId", turn_id))
                .collect::<Result<Vec<_>>>()?;
            print_json(&shown(client.turns_wait(timeout_seconds, &turns).await, || {
                format!("{endpoint}/v1/turns/wait")
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

async fn archive(
    client: &Client,
    endpoint: &str,
    session_ids: &[String],
    state: Option<String>,
    last_turn_before: Option<String>,
) -> Result<()> {
    if let [session_id] = session_ids {
        let session = id("sessionId", session_id)?;
        return print_json(&shown(client.sessions_archive(&session).await, || {
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
        answered(client.sessions_list(None, before.as_ref(), state).await, || {
            format!("{endpoint}/v1/sessions")
        })?
        .into_iter()
        .map(|session| session.session_id)
        .collect()
    } else {
        session_ids
            .iter()
            .map(|value| id("sessionId", value))
            .collect::<Result<Vec<_>>>()?
    };
    let mut archived = Vec::new();
    let mut refused = Vec::new();
    for session in &sessions {
        match client.sessions_archive(session).await {
            Err(Error::ErrorResponse(problem)) => {
                refused.push(format!("session {}: {}", **session, problem_text(&problem)));
            }
            outcome => archived.push(shown(outcome, || {
                format!("{endpoint}/v1/sessions/{}/archive", **session)
            })?),
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
        for line in headed_lines(texts.into_iter().take(printed), read.compact, &mut headed) {
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
            let last = match turn {
                Some(turn) => answered(client.turns_get(&turn).await, || {
                    format!("{endpoint}/v1/turns/{}", *turn)
                })?,
                None => {
                    answered(client.sessions_get(&session).await, || {
                        format!("{endpoint}/v1/sessions/{}", *session)
                    })?
                    .last_turn
                }
            };
            if let Turn::FailedTurn(failed) = last {
                bail!(
                    "turn {} failed with {}: {}",
                    *failed.turn_id,
                    failed.reason,
                    failed.detail
                )
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

#[cfg(test)]
mod tests {
    use super::{headed_lines, spend, Spent};
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
}
