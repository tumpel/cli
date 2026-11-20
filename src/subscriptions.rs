use crate::sessions::{id, parsed, tags};
use crate::{answered, print_json, shown, signed_in, SubscriptionCommand};
use anyhow::Result;
use chrono::{DateTime, FixedOffset};
use tumpel_api::types::{SubscriptionCreate, SubscriptionPause, SubscriptionState};

pub async fn subscription(command: SubscriptionCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let url = format!("{}/v1/subscriptions", credentials.endpoint);
    match command {
        SubscriptionCommand::Create {
            session_id,
            prompt,
            goal,
            event,
            crew_tag,
            expires_at,
        } => {
            let body = SubscriptionCreate {
                crew_tags: tags("--crew-tag", &crew_tag)?,
                events: event
                    .iter()
                    .map(|value| parsed("--event", value))
                    .collect::<Result<_>>()?,
                expires_at: parsed::<DateTime<FixedOffset>>("--expires-at", &expires_at)?,
                goal: parsed("--goal", &goal)?,
                prompt: parsed("--prompt", &prompt)?,
                session_id: id("--session-id", &session_id)?,
            };
            print_json(&shown(client.subscriptions_create(&body).await, || url)?)
        }
        SubscriptionCommand::List { session_id, state } => {
            let session = session_id.map(|value| id("--session-id", &value)).transpose()?;
            let state = state
                .map(|value| parsed::<SubscriptionState>("--state", &value))
                .transpose()?;
            print_json(&shown(
                client.subscriptions_list(session.as_ref(), state).await,
                || url,
            )?)
        }
        SubscriptionCommand::Get { subscription_id } => {
            let subscription = id("subscriptionId", &subscription_id)?;
            print_json(&shown(client.subscriptions_get(&subscription).await, || {
                format!("{url}/{subscription_id}")
            })?)
        }
        SubscriptionCommand::Pause {
            subscription_id,
            message,
        } => {
            let subscription = id("subscriptionId", &subscription_id)?;
            let body = SubscriptionPause {
                message: parsed("--message", &message)?,
            };
            print_json(&shown(client.subscriptions_pause(&subscription, &body).await, || {
                format!("{url}/{subscription_id}/pause")
            })?)
        }
        SubscriptionCommand::Resume { subscription_id } => {
            let subscription = id("subscriptionId", &subscription_id)?;
            print_json(&shown(client.subscriptions_resume(&subscription).await, || {
                format!("{url}/{subscription_id}/resume")
            })?)
        }
        SubscriptionCommand::Delete { subscription_id } => {
            let subscription = id("subscriptionId", &subscription_id)?;
            answered(client.subscriptions_delete(&subscription).await, || {
                format!("{url}/{subscription_id}")
            })?;
            eprintln!("deleted subscription {subscription_id}");
            Ok(())
        }
        SubscriptionCommand::Complete { subscription_id } => {
            let subscription = id("subscriptionId", &subscription_id)?;
            print_json(&shown(client.subscriptions_complete(&subscription).await, || {
                format!("{url}/{subscription_id}/complete")
            })?)
        }
    }
}
