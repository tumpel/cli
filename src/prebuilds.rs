use crate::sessions::{id, parsed};
use crate::{print_json, shown, signed_in, PrebuildCommand, PrebuildTemplateCommand};
use anyhow::{Context, Result};
use tumpel_api::types::{
    PrebuildPreviewOfRepository, PrebuildPreviewOfRepositorySource, PrebuildPreviewOfSupplied,
    PrebuildPreviewOfSuppliedSource, PrebuildPreviewRequest, PrebuildStart, PrebuildTemplateId,
    PrebuildTemplatePullRequestOpen, PrebuildTemplateVersionNumber, RepositoryUrl,
};

pub async fn prebuild(command: PrebuildCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    match command {
        PrebuildCommand::Start {
            repository,
            persona_id,
            rebuild,
        } => {
            let body = PrebuildStart {
                repository: parsed("--repository", &repository)?,
                persona_id: id("--persona-id", &persona_id)?,
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
        PrebuildCommand::Preview {
            repository,
            persona_id,
            config,
        } => {
            let repository: RepositoryUrl = parsed("--repository", &repository)?;
            let persona_id = id("--persona-id", &persona_id)?;
            let body: PrebuildPreviewRequest = match config {
                None => PrebuildPreviewOfRepository {
                    source: PrebuildPreviewOfRepositorySource::RepositoryFile,
                    repository,
                    persona_id,
                }
                .into(),
                Some(path) => {
                    let text = std::fs::read_to_string(&path)
                        .with_context(|| format!("reading {}, the --config file", path.display()))?;
                    PrebuildPreviewOfSupplied {
                        source: PrebuildPreviewOfSuppliedSource::Supplied,
                        repository,
                        persona_id,
                        devcontainer: parsed("--config", &text)?,
                    }
                    .into()
                }
            };
            print_json(&shown(client.prebuilds_preview(&body).await, || {
                format!("{endpoint}/v1/prebuilds/preview")
            })?)
        }
    }
}

pub async fn prebuild_template(command: PrebuildTemplateCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    match command {
        PrebuildTemplateCommand::List => print_json(&shown(client.prebuild_templates_list().await, || {
            format!("{endpoint}/v1/prebuild-templates")
        })?),
        PrebuildTemplateCommand::Get { template, version } => {
            let template_id: PrebuildTemplateId = parsed("--template", &template)?;
            let version_number: PrebuildTemplateVersionNumber = parsed("--version", &version)?;
            print_json(&shown(
                client.prebuild_templates_get(&template_id, &version_number).await,
                || format!("{endpoint}/v1/prebuild-templates/{template}/versions/{version}"),
            )?)
        }
        PrebuildTemplateCommand::OpenPullRequest {
            repository,
            template,
            version,
            replace,
        } => {
            let body = PrebuildTemplatePullRequestOpen {
                repository: parsed("--repository", &repository)?,
                prebuild_template_id: parsed("--template", &template)?,
                prebuild_template_version: parsed("--version", &version)?,
                replace,
            };
            print_json(&shown(
                client.prebuild_templates_open_pull_request(&body).await,
                || format!("{endpoint}/v1/prebuild-templates/pull-request"),
            )?)
        }
        PrebuildTemplateCommand::GetPullRequest { repository } => {
            let repository_url: RepositoryUrl = parsed("--repository", &repository)?;
            print_json(&shown(
                client.prebuild_templates_get_pull_request(&repository_url).await,
                || format!("{endpoint}/v1/prebuild-templates/pull-request?repository={repository}"),
            )?)
        }
    }
}
