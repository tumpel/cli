use crate::{answered, print_json, shown, signed_in, SessionCommand, SnapshotCommand, TurnCommand};
use anyhow::{bail, Result};
use reqwest::StatusCode;
use std::num::NonZeroU32;
use std::str::FromStr;
use std::time::Duration;
use tumpel_api::types::{EntityId, SessionStart, SessionStartFromIssue, SessionState, TranscriptPage, TurnSend};
use tumpel_api::Error;

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
        SessionCommand::Stop { session_id } => {
            let session = id("sessionId", &session_id)?;
            print_json(&shown(client.sessions_stop(&session).await, || {
                format!("{endpoint}/v1/sessions/{session_id}/stop")
            })?)
        }
        SessionCommand::Archive { session_id } => {
            let session = id("sessionId", &session_id)?;
            print_json(&shown(client.sessions_archive(&session).await, || {
                format!("{endpoint}/v1/sessions/{session_id}/archive")
            })?)
        }
        SessionCommand::Handovers { session_id } => {
            let session = id("sessionId", &session_id)?;
            print_json(&shown(
                client.sessions_list_credential_handovers(&session).await,
                || format!("{endpoint}/v1/sessions/{session_id}/credential-handovers"),
            )?)
        }
        SessionCommand::List { state, host_id } => {
            let state = state
                .map(|value| parsed::<SessionState>("--state", &value))
                .transpose()?;
            let host = host_id.map(|value| id("--host-id", &value)).transpose()?;
            print_json(&shown(client.sessions_list(host.as_ref(), state).await, || {
                format!("{endpoint}/v1/sessions")
            })?)
        }
    }
}

pub async fn turn(command: TurnCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    match command {
        TurnCommand::Send { session_id, prompt } => {
            let session = id("sessionId", &session_id)?;
            let body = TurnSend {
                prompt: parsed("--prompt", &prompt)?,
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

pub struct TranscriptRead {
    pub turn_id: String,
    pub follow: bool,
    pub after: i64,
    pub limit: Option<NonZeroU32>,
    pub max_bytes: Option<NonZeroU32>,
    pub compact: bool,
}

pub async fn transcript(read: TranscriptRead) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    let turn_id = read.turn_id;
    let turn = id("turnId", &turn_id)?;
    let session = answered(client.turns_get(&turn).await, || {
        format!("{endpoint}/v1/turns/{turn_id}")
    })?
    .session_id;
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
                    Some(&turn),
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
        for (turn_id, text) in texts.into_iter().take(printed) {
            if read.compact && headed.as_ref() != Some(&turn_id) {
                println!("turn {}", *turn_id);
                headed = Some(turn_id);
            }
            println!("{text}");
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
            return match client.turns_get_failure(&turn).await {
                Err(Error::ErrorResponse(problem)) if problem.status() == StatusCode::NOT_FOUND => Ok(()),
                outcome => {
                    let failure = answered(outcome, || format!("{endpoint}/v1/turns/{turn_id}/failure"))?;
                    bail!("turn {turn_id} failed with {}: {}", failure.reason, failure.detail)
                }
            };
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
    use super::{spend, Spent};
    use std::num::NonZeroU32;

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
}
