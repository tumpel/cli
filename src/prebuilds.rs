use crate::sessions::{id, parsed};
use crate::{print_json, shown, signed_in, PrebuildCommand};
use anyhow::Result;
use tumpel_api::types::PrebuildStart;

pub async fn prebuild(command: PrebuildCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    match command {
        PrebuildCommand::Start {
            repository,
            persona_id,
            provider,
            rebuild,
        } => {
            let body = PrebuildStart {
                repository: parsed("--repository", &repository)?,
                persona_id: id("--persona-id", &persona_id)?,
                provider: parsed("--provider", &provider)?,
                rebuild: Some(rebuild),
            };
            print_json(&shown(client.prebuilds_start(&body).await, || {
                format!("{endpoint}/v1/prebuilds")
            })?)
        }
        PrebuildCommand::List { repository } => {
            let repository_url: Option<tumpel_api::types::RepositoryUrl> = repository
                .as_deref()
                .map(|value| parsed("--repository", value))
                .transpose()?;
            print_json(&shown(client.prebuilds_list(repository_url.as_ref()).await, || {
                repository.as_ref().map_or_else(
                    || format!("{endpoint}/v1/prebuilds"),
                    |value| format!("{endpoint}/v1/prebuilds?repository={value}"),
                )
            })?)
        }
        PrebuildCommand::ListLatest => print_json(&shown(client.prebuilds_list_latest().await, || {
            format!("{endpoint}/v1/prebuilds/latest")
        })?),
        PrebuildCommand::Get { prebuild_id } => {
            let prebuild = id("prebuildId", &prebuild_id)?;
            print_json(&shown(client.prebuilds_get(&prebuild).await, || {
                format!("{endpoint}/v1/prebuilds/{prebuild_id}")
            })?)
        }
        PrebuildCommand::Failure { prebuild_id } => {
            let prebuild = id("prebuildId", &prebuild_id)?;
            print_json(&shown(client.prebuilds_get_failure(&prebuild).await, || {
                format!("{endpoint}/v1/prebuilds/{prebuild_id}/failure")
            })?)
        }
    }
}
