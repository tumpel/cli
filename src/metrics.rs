use crate::sessions::{id, parsed};
use crate::{print_json, shown, signed_in, MetricWindowArgs, MetricsCommand};
use anyhow::Result;
use chrono::{DateTime, FixedOffset};
use tumpel_api::types::{MetricBucket, MetricName, Summary};

struct Window {
    from: DateTime<FixedOffset>,
    to: DateTime<FixedOffset>,
    filter: Option<Vec<Summary>>,
}

fn window(args: &MetricWindowArgs) -> Result<Window> {
    Ok(Window {
        from: parsed("--from", &args.from)?,
        to: parsed("--to", &args.to)?,
        filter: (!args.filter.is_empty())
            .then(|| {
                args.filter
                    .iter()
                    .map(|value| parsed("--filter", value))
                    .collect::<Result<_>>()
            })
            .transpose()?,
    })
}

pub async fn metrics(command: MetricsCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    match command {
        MetricsCommand::Catalog { window: args } => {
            let window = window(&args)?;
            print_json(&shown(
                client
                    .metrics_catalog(window.filter.as_ref(), &window.from, &window.to)
                    .await,
                || format!("{endpoint}/v1/metrics"),
            )?)
        }
        MetricsCommand::Query {
            metric,
            window: args,
            bucket,
            group_by,
        } => {
            let name: MetricName = parsed("metric", &metric)?;
            let bucket: MetricBucket = parsed("--bucket", &bucket)?;
            let window = window(&args)?;
            let group_by = (!group_by.is_empty())
                .then(|| {
                    group_by
                        .iter()
                        .map(|value| parsed("--group-by", value))
                        .collect::<Result<Vec<_>>>()
                })
                .transpose()?;
            print_json(&shown(
                client
                    .metrics_query(
                        &name,
                        bucket,
                        window.filter.as_ref(),
                        &window.from,
                        group_by.as_ref(),
                        &window.to,
                    )
                    .await,
                || format!("{endpoint}/v1/metrics/{metric}/series"),
            )?)
        }
        MetricsCommand::Turns {
            metric,
            window: args,
            session_id,
            cursor,
        } => {
            let name: MetricName = parsed("metric", &metric)?;
            let window = window(&args)?;
            let cursor = cursor.as_deref().map(|value| parsed("--cursor", value)).transpose()?;
            let session = session_id
                .as_deref()
                .map(|value| id("--session-id", value))
                .transpose()?;
            print_json(&shown(
                client
                    .metrics_turns(
                        &name,
                        cursor.as_ref(),
                        window.filter.as_ref(),
                        &window.from,
                        session.as_ref(),
                        &window.to,
                    )
                    .await,
                || format!("{endpoint}/v1/metrics/{metric}/turns"),
            )?)
        }
    }
}
