use crate::sessions::{parsed, retention};
use crate::tenant_config;
use crate::{
    answered, print_json, shown, signed_in, HarnessCommand, PersonaCommand, PersonaFields, PersonaFileChange,
    PersonaPatchFields, SecretMemberCommand, SecretSetCommand,
};
use anyhow::{anyhow, bail, Context, Result};
use serde::de::DeserializeOwned;
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use tumpel_api::types::{
    ApiKeyValue, ApiKeyValueKind, ConfigSecretSet, EditPersonaFilePatch, EditPersonaFilePatchKind, EntityId, Harness,
    HarnessPinCreate, HostLabel, HostLabels, HostLabelsPatch, InlinePersonaFile, InlinePersonaFileSource,
    PersonaCreate, PersonaFile, PersonaFilePatch, PersonaFileWrite, PersonaPatch, PersonaUpdate, PutPersonaFilePatch,
    PutPersonaFilePatchKind, RemovePersonaFilePatch, RemovePersonaFilePatchKind, SecretMemberUpdate, SecretSetCreate,
    SecretSetPatch, SecretSetUpdate, SecretText, SecretValue, SecretVariableValue, SecretVariableValueKind,
    StoredPersonaFile, StoredPersonaFileSource, SubscriptionTokenValue, SubscriptionTokenValueKind,
    SupervisorUpdateLatestStep, SupervisorUpdatePhase,
};
use tumpel_api::Client;

pub fn entity_id(raw: &str, what: &str) -> Result<EntityId> {
    raw.to_string()
        .try_into()
        .map_err(|error| anyhow!("{raw:?} is not a {what} id: {error}"))
}

pub(crate) fn read_json<T: DeserializeOwned>(file: &Path, schema: &str) -> Result<T> {
    let text = std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    serde_json::from_str(&text).with_context(|| {
        format!(
            "{} is not a {schema}; fix it to match the {schema} schema in typespec",
            file.display()
        )
    })
}

fn connector_ids(raw: &[String]) -> Result<Vec<EntityId>> {
    raw.iter().map(|id| entity_id(id, "connector")).collect()
}

fn harness(raw: &str) -> Result<Harness> {
    raw.parse()
        .map_err(|error| anyhow!("{raw:?} is not a harness; tumpel harness list names them ({error})"))
}

fn secrets_from_env(secret_set: &ConfigSecretSet) -> Result<Vec<SecretValue>> {
    let values = tenant_config::secret_values([secret_set], |name| std::env::var(name).ok());
    tenant_config::with_values(secret_set, &values)
}

pub fn stdin_value(text: &str, name: &str) -> Result<SecretText> {
    let value = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text);
    if value.is_empty() {
        bail!("stdin held no value for {name}; pipe it in, such as gh auth token | tumpel secret-set set-member ... --stdin");
    }
    value
        .to_string()
        .try_into()
        .map_err(|error| anyhow!("stdin does not hold a usable value for {name} ({error})"))
}

pub fn read_stdin(stdin: bool, name: &str) -> Result<SecretText> {
    if !stdin {
        bail!("pass --stdin and pipe the value of {name} in; a secret value is never an argument");
    }
    let mut input = std::io::stdin();
    if input.is_terminal() {
        bail!("--stdin reads a piped value and stdin is a terminal; pipe the value of {name} in");
    }
    let mut text = String::new();
    input
        .read_to_string(&mut text)
        .with_context(|| format!("reading the value of {name} from stdin"))?;
    stdin_value(&text, name)
}

async fn persona_file_spec(client: &Client, flag: &str, spec: &str, rerun: &str) -> Result<PersonaFileWrite> {
    let (path, local) = spec
        .split_once('=')
        .ok_or_else(|| anyhow!("{flag} {spec:?} is not <path in the harness home>=<local file>"))?;
    crate::bundle::persona_file(client, parsed(flag, path)?, &PathBuf::from(local), rerun).await
}

async fn persona_files(client: &Client, specs: &[String], rerun: &str) -> Result<Vec<PersonaFileWrite>> {
    let mut files = Vec::with_capacity(specs.len());
    for spec in specs {
        files.push(persona_file_spec(client, "--persona-file", spec, rerun).await?);
    }
    Ok(files)
}

async fn persona_create(
    client: &Client,
    file: Option<PathBuf>,
    fields: Option<PersonaFields>,
) -> Result<PersonaCreate> {
    let Some(fields) = fields else {
        return read_json(&file.unwrap_or_default(), "PersonaCreate");
    };
    Ok(PersonaCreate {
        box_retention: retention(fields.box_retention.as_deref())?,
        box_memory_mib: fields.box_memory_mib,
        box_disk_mib: fields.box_disk_mib,
        connector_ids: connector_ids(&fields.connector)?,
        files: persona_files(client, &fields.persona_file, "tumpel persona create").await?,
        harness: harness(&fields.harness)?,
        name: parsed("--name", &fields.name)?,
        secret_set_id: entity_id(&fields.secret_set_id, "secret set")?,
        tumpel_mcp: fields.tumpel_mcp.then_some(true),
    })
}

struct PersonaChange {
    name: Option<String>,
    secret_set_id: Option<String>,
    persona_file: Vec<String>,
    connector: Vec<String>,
    no_connectors: bool,
    box_memory_mib: Option<i32>,
    box_disk_mib: Option<i32>,
    box_retention: Option<String>,
}

async fn persona_update(client: &Client, url: &str, id: &EntityId, change: PersonaChange) -> Result<PersonaUpdate> {
    let PersonaChange {
        name,
        secret_set_id,
        persona_file,
        connector,
        no_connectors,
        box_memory_mib,
        box_disk_mib,
        box_retention,
    } = change;
    let current = answered(client.personas_get(id).await, || url.to_string())?;
    let files = if persona_file.is_empty() {
        current
            .files
            .into_iter()
            .map(|held| match held {
                PersonaFile::InlinePersonaFileRead(held) => InlinePersonaFile {
                    content: held.content,
                    path: held.path,
                    source: InlinePersonaFileSource::Inline,
                }
                .into(),
                PersonaFile::StoredPersonaFileRead(held) => StoredPersonaFile {
                    digest: held.digest,
                    path: held.path,
                    size_bytes: held.size_bytes,
                    source: StoredPersonaFileSource::Stored,
                }
                .into(),
            })
            .collect()
    } else {
        persona_files(client, &persona_file, "tumpel persona update").await?
    };
    let connector_ids = if connector.is_empty() && !no_connectors {
        current.connector_ids
    } else {
        connector_ids(&connector)?
    };
    Ok(PersonaUpdate {
        box_retention: Some(
            box_retention.map_or(Ok(current.box_retention), |value| parsed("--box-retention", &value))?,
        ),
        box_memory_mib: box_memory_mib.or(current.box_memory_mib),
        box_disk_mib: box_disk_mib.or(current.box_disk_mib),
        connector_ids,
        files,
        name: name.map_or(Ok(current.name), |name| parsed("--name", &name))?,
        secret_set_id: secret_set_id.map_or(Ok(current.secret_set_id), |raw| entity_id(&raw, "secret set"))?,
        tumpel_mcp: Some(current.tumpel_mcp),
    })
}

fn persona_patch(fields: PersonaPatchFields) -> Result<PersonaPatch> {
    Ok(PersonaPatch {
        box_disk_mib: fields.box_disk_mib,
        box_memory_mib: fields.box_memory_mib,
        box_retention: retention(fields.box_retention.as_deref())?,
        connector_ids: connector_ids(&fields.connector)?,
        name: fields.name.map(|name| parsed("--name", &name)).transpose()?,
        secret_set_id: fields
            .secret_set_id
            .map(|raw| entity_id(&raw, "secret set"))
            .transpose()?,
        tumpel_mcp: fields.tumpel_mcp,
    })
}

async fn persona_file_patch(client: &Client, change: PersonaFileChange) -> Result<PersonaFilePatch> {
    let PersonaFileChange {
        put,
        remove,
        edit,
        old_text,
        new_text,
    } = change;
    if let Some(put) = put {
        return Ok(PutPersonaFilePatch {
            file: persona_file_spec(client, "--put", &put, "tumpel persona patch-file").await?,
            kind: PutPersonaFilePatchKind::Put,
        }
        .into());
    }
    if let Some(path) = remove {
        return Ok(RemovePersonaFilePatch {
            kind: RemovePersonaFilePatchKind::Remove,
            path: parsed("--remove", &path)?,
        }
        .into());
    }
    Ok(EditPersonaFilePatch {
        kind: EditPersonaFilePatchKind::Edit,
        new_text: parsed("--new-text", &new_text.unwrap_or_default())?,
        old_text: parsed("--old-text", &old_text.unwrap_or_default())?,
        path: parsed("--edit", &edit.unwrap_or_default())?,
    }
    .into())
}

fn kept_members(secret_set: &ConfigSecretSet) -> Result<Vec<SecretMemberUpdate>> {
    tenant_config::with_kept_values(
        secret_set,
        &tenant_config::secret_values([secret_set], |name| std::env::var(name).ok()),
    )
}

fn secret_set_patch(file: Option<PathBuf>, name: Option<String>) -> Result<SecretSetPatch> {
    let Some(file) = file else {
        return Ok(SecretSetPatch {
            name: Some(parsed("--name", &name.unwrap_or_default())?),
            secrets: Vec::new(),
        });
    };
    let secret_set: ConfigSecretSet = read_json(&file, "ConfigSecretSet")?;
    Ok(SecretSetPatch {
        name: Some(secret_set.name.clone()),
        secrets: kept_members(&secret_set)?,
    })
}

pub async fn host_get(host_id: String) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let id = entity_id(&host_id, "host")?;
    let host = answered(client.hosts_get(&id).await, || {
        format!("{}/v1/hosts/{host_id}", credentials.endpoint)
    })?;
    print_json(&host)?;
    if let SupervisorUpdateLatestStep::StepReported(reported) = &host.supervisor_update_step {
        if reported.step.phase == SupervisorUpdatePhase::Staged {
            eprintln!(
                "Update to {} staged, waiting for {} boxes",
                *reported.step.release_version, reported.step.boxes_running
            );
        }
    }
    Ok(())
}

fn host_labels(labels: Vec<String>) -> Result<Vec<HostLabel>> {
    labels
        .into_iter()
        .map(|label| {
            label
                .clone()
                .try_into()
                .map_err(|error| anyhow!("{label:?} is not a host label: {error}"))
        })
        .collect()
}

pub async fn host_set_labels(host_id: String, labels: Vec<String>) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let id = entity_id(&host_id, "host")?;
    let body = HostLabels {
        labels: host_labels(labels)?,
    };
    print_json(&shown(client.hosts_set_labels(&id, &body).await, || {
        format!("{}/v1/hosts/{host_id}/labels", credentials.endpoint)
    })?)
}

pub async fn host_patch_labels(host_id: String, add: Vec<String>, remove: Vec<String>) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let id = entity_id(&host_id, "host")?;
    let body = HostLabelsPatch {
        add: host_labels(add)?,
        remove: host_labels(remove)?,
    };
    print_json(&shown(client.hosts_patch_labels(&id, &body).await, || {
        format!("{}/v1/hosts/{host_id}/labels", credentials.endpoint)
    })?)
}

pub async fn harnesses(command: HarnessCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    match command {
        HarnessCommand::List => print_json(&shown(client.harness_catalog_list().await, || {
            format!("{endpoint}/v1/harnesses")
        })?),
        HarnessCommand::ListPins => print_json(&shown(client.harness_catalog_list_pins().await, || {
            format!("{endpoint}/v1/harnesses/pins")
        })?),
        HarnessCommand::Pin { harness: name, version } => {
            let body = HarnessPinCreate {
                harness: harness(&name)?,
                harness_version: version
                    .clone()
                    .try_into()
                    .map_err(|error| anyhow!("{version:?} is not a harness version: {error}"))?,
            };
            print_json(&shown(client.harness_catalog_pin(&body).await, || {
                format!("{endpoint}/v1/harnesses/pins")
            })?)
        }
        HarnessCommand::Unpin { harness: name } => {
            answered(client.harness_catalog_unpin(harness(&name)?).await, || {
                format!("{endpoint}/v1/harnesses/pins/{name}")
            })?;
            eprintln!("unpinned {name}");
            Ok(())
        }
    }
}

pub async fn personas(command: PersonaCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    match command {
        PersonaCommand::Create { file, fields } => {
            let body = persona_create(&client, file, fields).await?;
            print_json(&shown(client.personas_create(&body).await, || {
                format!("{endpoint}/v1/personas")
            })?)
        }
        PersonaCommand::List => print_json(&shown(client.personas_list().await, || {
            format!("{endpoint}/v1/personas")
        })?),
        PersonaCommand::Get { persona_id } => {
            let id = entity_id(&persona_id, "persona")?;
            print_json(&shown(client.personas_get(&id).await, || {
                format!("{endpoint}/v1/personas/{persona_id}")
            })?)
        }
        PersonaCommand::Update {
            persona_id,
            file,
            name,
            secret_set_id,
            persona_file,
            connector,
            no_connectors,
            box_memory_mib,
            box_disk_mib,
            box_retention,
        } => {
            let id = entity_id(&persona_id, "persona")?;
            let url = format!("{endpoint}/v1/personas/{persona_id}");
            let body = match file {
                Some(file) => read_json(&file, "PersonaUpdate")?,
                None => {
                    persona_update(
                        &client,
                        &url,
                        &id,
                        PersonaChange {
                            name,
                            secret_set_id,
                            persona_file,
                            connector,
                            no_connectors,
                            box_memory_mib,
                            box_disk_mib,
                            box_retention,
                        },
                    )
                    .await?
                }
            };
            print_json(&shown(client.personas_update(&id, &body).await, || url)?)
        }
        PersonaCommand::Patch { persona_id, fields } => {
            let id = entity_id(&persona_id, "persona")?;
            print_json(&shown(
                client.personas_patch(&id, &persona_patch(fields)?).await,
                || format!("{endpoint}/v1/personas/{persona_id}"),
            )?)
        }
        PersonaCommand::PatchFile { persona_id, change } => {
            let id = entity_id(&persona_id, "persona")?;
            let body = persona_file_patch(&client, change).await?;
            print_json(&shown(client.personas_patch_file(&id, &body).await, || {
                format!("{endpoint}/v1/personas/{persona_id}/file-patches")
            })?)
        }
        PersonaCommand::UploadFile { file } => {
            let bytes = std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let (digest, size_bytes) = crate::bundle::upload(&client, &bytes, "tumpel persona upload-file").await?;
            print_json(&serde_json::json!({ "digest": digest, "sizeBytes": size_bytes }))
        }
        PersonaCommand::Delete { persona_id } => {
            let id = entity_id(&persona_id, "persona")?;
            answered(client.personas_delete(&id).await, || {
                format!("{endpoint}/v1/personas/{persona_id}")
            })?;
            eprintln!("deleted persona {persona_id}");
            Ok(())
        }
    }
}

async fn set_member(client: &Client, endpoint: &str, member: SecretMemberCommand) -> Result<()> {
    let (SecretMemberCommand::Variable { target }
    | SecretMemberCommand::ApiKey { target, .. }
    | SecretMemberCommand::SubscriptionToken { target, .. }) = &member;
    let id = entity_id(&target.secret_set_id, "secret set")?;
    let url = format!("{endpoint}/v1/secret-sets/{}/members", target.secret_set_id);
    let name = parsed("--name", &target.name)?;
    let value = read_stdin(target.stdin, &target.name)?;
    let body: SecretValue = match member {
        SecretMemberCommand::Variable { .. } => SecretVariableValue {
            kind: SecretVariableValueKind::Variable,
            name,
            value,
        }
        .into(),
        SecretMemberCommand::ApiKey { provider, .. } => ApiKeyValue {
            kind: ApiKeyValueKind::ApiKey,
            name,
            provider: parsed("--provider", &provider)?,
            value,
        }
        .into(),
        SecretMemberCommand::SubscriptionToken {
            provider, expires_at, ..
        } => SubscriptionTokenValue {
            expires_at: chrono::DateTime::parse_from_rfc3339(&expires_at).map_err(|error| {
                anyhow!("--expires-at {expires_at:?} is not an ISO 8601 date-time with an offset ({error})")
            })?,
            kind: SubscriptionTokenValueKind::SubscriptionToken,
            name,
            provider: parsed("--provider", &provider)?,
            value,
        }
        .into(),
    };
    print_json(&shown(client.secret_sets_set_member(&id, &body).await, || url)?)
}

pub async fn secret_sets(command: SecretSetCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    match command {
        SecretSetCommand::Create { file, name } => {
            let body = match file {
                Some(file) => {
                    let secret_set: ConfigSecretSet = read_json(&file, "ConfigSecretSet")?;
                    SecretSetCreate {
                        name: secret_set.name.clone(),
                        secrets: secrets_from_env(&secret_set)?,
                    }
                }
                None => SecretSetCreate {
                    name: parsed("--name", &name.unwrap_or_default())?,
                    secrets: Vec::new(),
                },
            };
            print_json(&shown(client.secret_sets_create(&body).await, || {
                format!("{endpoint}/v1/secret-sets")
            })?)
        }
        SecretSetCommand::List => print_json(&shown(client.secret_sets_list().await, || {
            format!("{endpoint}/v1/secret-sets")
        })?),
        SecretSetCommand::Get { secret_set_id } => {
            let id = entity_id(&secret_set_id, "secret set")?;
            print_json(&shown(client.secret_sets_get(&id).await, || {
                format!("{endpoint}/v1/secret-sets/{secret_set_id}")
            })?)
        }
        SecretSetCommand::Update { secret_set_id, file } => {
            let id = entity_id(&secret_set_id, "secret set")?;
            let secret_set: ConfigSecretSet = read_json(&file, "ConfigSecretSet")?;
            let body = SecretSetUpdate {
                name: secret_set.name.clone(),
                secrets: kept_members(&secret_set)?,
            };
            print_json(&shown(client.secret_sets_update(&id, &body).await, || {
                format!("{endpoint}/v1/secret-sets/{secret_set_id}")
            })?)
        }
        SecretSetCommand::Patch {
            secret_set_id,
            file,
            name,
        } => {
            let id = entity_id(&secret_set_id, "secret set")?;
            let body = secret_set_patch(file, name)?;
            print_json(&shown(client.secret_sets_patch(&id, &body).await, || {
                format!("{endpoint}/v1/secret-sets/{secret_set_id}")
            })?)
        }
        SecretSetCommand::SetMember { command: member } => set_member(&client, &endpoint, member).await,
        SecretSetCommand::RemoveMember { secret_set_id, name } => {
            let id = entity_id(&secret_set_id, "secret set")?;
            let member = parsed("name", &name)?;
            print_json(&shown(client.secret_sets_remove_member(&id, &member).await, || {
                format!("{endpoint}/v1/secret-sets/{secret_set_id}/members/{name}")
            })?)
        }
        SecretSetCommand::Delete { secret_set_id } => {
            let id = entity_id(&secret_set_id, "secret set")?;
            answered(client.secret_sets_delete(&id).await, || {
                format!("{endpoint}/v1/secret-sets/{secret_set_id}")
            })?;
            eprintln!("deleted secret set {secret_set_id}");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::stdin_value;

    #[test]
    fn stdin_value_drops_one_trailing_newline_and_refuses_an_empty_value() {
        assert_eq!(&*stdin_value("tok\n", "the connector token").unwrap(), "tok");
        assert_eq!(&*stdin_value("tok\r\n", "the connector token").unwrap(), "tok");
        assert!(stdin_value("\n", "the connector token").is_err());
    }
}
