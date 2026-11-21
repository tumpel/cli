use crate::resources::read_stdin;
use crate::sessions::parsed;
use crate::tenant_config::{self, Live};
use crate::{answered, print_json, shown, signed_in};
use anyhow::{anyhow, bail, Result};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::path::PathBuf;
use tumpel_api::types::{
    ApiKeyUpdate, ApiKeyUpdateKind, CredentialFileUpdate, CredentialFileUpdateKind, DisplayName, EntityId, Harness,
    HarnessPinCreate, HostLabel, HostLabelsPatch, HostState, Persona, PersonaFilePath, PersonaPatch,
    PutPersonaFilePatch, PutPersonaFilePatchKind, RemovePersonaFilePatch, RemovePersonaFilePatchKind, SecretKey,
    SecretMemberUpdate, SecretName, SecretSet, SecretSetPatch, SecretText, SecretVariableUpdate,
    SecretVariableUpdateKind, SubscriptionTokenUpdate, SubscriptionTokenUpdateKind, Trigger, TriggerPatch,
};
use tumpel_api::Client;

#[derive(Debug, PartialEq)]
enum PersonaField {
    Whole,
    Name,
    Harness,
    SecretSetName,
    ConnectorNames,
    ConnectorName(String),
    BoxMemoryMib,
    BoxDiskMib,
    BoxRetention,
    TumpelMcp,
    Files,
    File(String),
}

#[derive(Debug, PartialEq)]
enum MemberField {
    Name,
    Provider,
    ExpiresAt,
    Kind,
    ValueFrom,
}

#[derive(Debug, PartialEq)]
enum SecretSetField {
    Whole,
    Name,
    Secrets,
    Member(String),
    MemberField(String, MemberField),
}

#[derive(Debug, PartialEq)]
enum PinField {
    Whole,
    Harness,
    HarnessVersion,
}

#[derive(Debug, PartialEq)]
enum HostField {
    Whole,
    HostId,
    Labels,
    Label(String),
}

#[derive(Debug, PartialEq)]
enum TriggerField {
    Whole,
    Name,
    PersonaName,
    Model,
    Prompt,
    Source,
}

#[derive(Debug, PartialEq)]
enum Target {
    Persona(String, PersonaField),
    SecretSet(String, SecretSetField),
    HarnessPin(String, PinField),
    Host(String, HostField),
    Trigger(String, TriggerField),
}

enum Change {
    Set(String),
    Secret(SecretText),
    Delete,
}

enum Edit {
    Set(String),
    Delete,
}

enum Request {
    Edit(Target, Edit),
    Secret(String, String, SecretText),
}

#[derive(Debug, PartialEq)]
enum Write {
    Persona(EntityId, PersonaPatch),
    PutFile(EntityId, PersonaFilePath, PathBuf),
    RemoveFile(EntityId, PersonaFilePath),
    DeletePersona(EntityId),
    SecretSet(EntityId, SecretSetPatch),
    RemoveMember(EntityId, SecretName),
    DeleteSecretSet(EntityId),
    Pin(HarnessPinCreate),
    Unpin(Harness),
    Host(EntityId, HostLabelsPatch),
    Trigger(EntityId, TriggerPatch),
    DeleteTrigger(EntityId),
}

fn segments(path: &str) -> Result<Vec<String>> {
    let Some(rest) = path.strip_prefix('/') else {
        bail!("{path:?} is not a config path; start it with /, such as /personas/reviewer/boxMemoryMib");
    };
    if rest.is_empty() {
        return Ok(Vec::new());
    }
    rest.split('/')
        .map(|segment| {
            if segment.is_empty() {
                bail!("{path:?} holds an empty segment; drop the doubled or trailing /");
            }
            Ok(segment.replace("~1", "/").replace("~0", "~"))
        })
        .collect()
}

fn persona_field(rest: &[&str]) -> Option<PersonaField> {
    Some(match rest {
        [] => PersonaField::Whole,
        ["name"] => PersonaField::Name,
        ["harness"] => PersonaField::Harness,
        ["secretSetName"] => PersonaField::SecretSetName,
        ["connectorNames"] => PersonaField::ConnectorNames,
        ["connectorNames", connector] => PersonaField::ConnectorName((*connector).to_string()),
        ["boxMemoryMib"] => PersonaField::BoxMemoryMib,
        ["boxDiskMib"] => PersonaField::BoxDiskMib,
        ["boxRetention"] => PersonaField::BoxRetention,
        ["tumpelMcp"] => PersonaField::TumpelMcp,
        ["files"] => PersonaField::Files,
        ["files", file @ ..] => PersonaField::File(file.join("/")),
        _ => return None,
    })
}

fn member_field(field: &str) -> Option<MemberField> {
    Some(match field {
        "name" => MemberField::Name,
        "provider" => MemberField::Provider,
        "expiresAt" => MemberField::ExpiresAt,
        "kind" => MemberField::Kind,
        "valueFrom" => MemberField::ValueFrom,
        _ => return None,
    })
}

fn secret_set_field(rest: &[&str]) -> Option<SecretSetField> {
    Some(match rest {
        [] => SecretSetField::Whole,
        ["name"] => SecretSetField::Name,
        ["secrets"] => SecretSetField::Secrets,
        ["secrets", member] => SecretSetField::Member((*member).to_string()),
        ["secrets", member, field] => SecretSetField::MemberField((*member).to_string(), member_field(field)?),
        _ => return None,
    })
}

fn pin_field(rest: &[&str]) -> Option<PinField> {
    Some(match rest {
        [] => PinField::Whole,
        ["harness"] => PinField::Harness,
        ["harnessVersion"] => PinField::HarnessVersion,
        _ => return None,
    })
}

fn host_field(rest: &[&str]) -> Option<HostField> {
    Some(match rest {
        [] => HostField::Whole,
        ["hostId"] => HostField::HostId,
        ["labels"] => HostField::Labels,
        ["labels", label] => HostField::Label((*label).to_string()),
        _ => return None,
    })
}

fn trigger_field(rest: &[&str]) -> Option<TriggerField> {
    Some(match rest {
        [] => TriggerField::Whole,
        ["name"] => TriggerField::Name,
        ["personaName"] => TriggerField::PersonaName,
        ["model"] => TriggerField::Model,
        ["prompt"] => TriggerField::Prompt,
        ["source"] => TriggerField::Source,
        _ => return None,
    })
}

fn target(path: &str) -> Result<Target> {
    let segments = segments(path)?;
    let parts: Vec<&str> = segments.iter().map(String::as_str).collect();
    let target = match parts.as_slice() {
        ["personas", persona, rest @ ..] => {
            persona_field(rest).map(|field| Target::Persona((*persona).to_string(), field))
        }
        ["secretSets", set, rest @ ..] => {
            secret_set_field(rest).map(|field| Target::SecretSet((*set).to_string(), field))
        }
        ["harnessPins", harness, rest @ ..] => {
            pin_field(rest).map(|field| Target::HarnessPin((*harness).to_string(), field))
        }
        ["hosts", host, rest @ ..] => host_field(rest).map(|field| Target::Host((*host).to_string(), field)),
        ["triggers", trigger, rest @ ..] => {
            trigger_field(rest).map(|field| Target::Trigger((*trigger).to_string(), field))
        }
        _ => None,
    };
    target.ok_or_else(|| {
        anyhow!("{path} is not a field of a persona, secret set, harness pin, host or trigger; tumpel config get / prints every field")
    })
}

fn request(path: &str, change: Change) -> Result<Request> {
    let target = target(path)?;
    Ok(match (change, target) {
        (Change::Secret(value), Target::SecretSet(set, SecretSetField::Member(member))) => {
            Request::Secret(set, member, value)
        }
        (Change::Secret(_), _) => bail!(
            "--stdin carries the value of a secret set member only, such as /secretSets/prod/secrets/GITHUB_TOKEN; give {path} its value as an argument"
        ),
        (Change::Set(_), Target::SecretSet(_, SecretSetField::Member(_))) => {
            bail!("a secret value is never an argument; pipe it in with tumpel config set {path} --stdin")
        }
        (Change::Set(raw), target) => Request::Edit(target, Edit::Set(raw)),
        (Change::Delete, target) => Request::Edit(target, Edit::Delete),
    })
}

fn value<T: DeserializeOwned>(path: &str, raw: &str) -> Result<T> {
    serde_json::from_str(raw).map_err(|error| {
        anyhow!("{raw} is not a value {path} takes ({error}); give it as JSON and quote text, such as '\"reviewer\"'")
    })
}

fn container(path: &str) -> anyhow::Error {
    anyhow!("{path} holds several fields; address one of the fields tumpel config get {path} prints")
}

fn required(path: &str) -> anyhow::Error {
    anyhow!("{path} cannot be deleted, because every entry holds it; set it to a new value instead")
}

fn list_entry(path: &str) -> anyhow::Error {
    let list = path.rsplit_once('/').map_or(path, |(list, _)| list);
    anyhow!("{path} is one entry of a list; set the whole list at {list}")
}

fn persona<'a>(live: &'a Live, name: &str) -> Result<&'a Persona> {
    live.personas
        .iter()
        .find(|persona| persona.name.as_str() == name)
        .ok_or_else(|| anyhow!("the tenant has no persona named {name}; tumpel config get /personas prints them"))
}

fn secret_set<'a>(live: &'a Live, name: &str) -> Result<&'a SecretSet> {
    live.secret_sets
        .iter()
        .find(|set| set.name.as_str() == name)
        .ok_or_else(|| anyhow!("the tenant has no secret set named {name}; tumpel config get /secretSets prints them"))
}

fn trigger<'a>(live: &'a Live, name: &str) -> Result<&'a Trigger> {
    live.triggers
        .iter()
        .find(|trigger| trigger.name.as_str() == name)
        .ok_or_else(|| anyhow!("the tenant has no trigger named {name}; tumpel config get /triggers prints them"))
}

fn connector_id(live: &Live, name: &str) -> Result<EntityId> {
    live.connectors
        .iter()
        .map(tenant_config::connector_identity)
        .find(|(_, connector)| connector.as_str() == name)
        .map(|(id, _)| parsed("connectorId", id))
        .ok_or_else(|| anyhow!("the tenant has no connector named {name}; tumpel connector list prints them"))?
}

fn no_connectors(persona: &Persona) -> anyhow::Error {
    anyhow!(
        "a patch cannot leave persona {} without connectors; run tumpel persona update {} --no-connectors",
        *persona.name,
        *persona.persona_id
    )
}

fn persona_set(live: &Live, path: &str, persona: &Persona, field: PersonaField, raw: &str) -> Result<Write> {
    let id = persona.persona_id.clone();
    let body = match field {
        PersonaField::Name => PersonaPatch {
            name: Some(value(path, raw)?),
            ..PersonaPatch::default()
        },
        PersonaField::SecretSetName => PersonaPatch {
            secret_set_id: Some(
                secret_set(live, &value::<DisplayName>(path, raw)?)?
                    .secret_set_id
                    .clone(),
            ),
            ..PersonaPatch::default()
        },
        PersonaField::ConnectorNames => {
            let names: Vec<DisplayName> = value(path, raw)?;
            if names.is_empty() {
                return Err(no_connectors(persona));
            }
            PersonaPatch {
                connector_ids: names
                    .iter()
                    .map(|name| connector_id(live, name))
                    .collect::<Result<_>>()?,
                ..PersonaPatch::default()
            }
        }
        PersonaField::BoxMemoryMib => PersonaPatch {
            box_memory_mib: Some(value(path, raw)?),
            ..PersonaPatch::default()
        },
        PersonaField::BoxDiskMib => PersonaPatch {
            box_disk_mib: Some(value(path, raw)?),
            ..PersonaPatch::default()
        },
        PersonaField::BoxRetention => PersonaPatch {
            box_retention: Some(value(path, raw)?),
            ..PersonaPatch::default()
        },
        PersonaField::TumpelMcp => PersonaPatch {
            tumpel_mcp: Some(value(path, raw)?),
            ..PersonaPatch::default()
        },
        PersonaField::File(file) => return Ok(Write::PutFile(id, parsed(path, &file)?, value(path, raw)?)),
        PersonaField::Harness => bail!(
            "the harness of persona {} is fixed once it exists; create a persona for another harness",
            *persona.name
        ),
        PersonaField::ConnectorName(_) => return Err(list_entry(path)),
        PersonaField::Whole | PersonaField::Files => return Err(container(path)),
    };
    Ok(Write::Persona(id, body))
}

fn persona_delete(live: &Live, path: &str, persona: &Persona, field: PersonaField) -> Result<Write> {
    let id = persona.persona_id.clone();
    Ok(match field {
        PersonaField::Whole => Write::DeletePersona(id),
        PersonaField::ConnectorName(connector) => {
            let removed = connector_id(live, &connector)?;
            if !persona.connector_ids.contains(&removed) {
                bail!("persona {} has no connector named {connector}", *persona.name);
            }
            let kept: Vec<EntityId> = persona
                .connector_ids
                .iter()
                .filter(|held| **held != removed)
                .cloned()
                .collect();
            if kept.is_empty() {
                return Err(no_connectors(persona));
            }
            Write::Persona(
                id,
                PersonaPatch {
                    connector_ids: kept,
                    ..PersonaPatch::default()
                },
            )
        }
        PersonaField::File(file) => Write::RemoveFile(id, parsed(path, &file)?),
        PersonaField::BoxMemoryMib
        | PersonaField::BoxDiskMib
        | PersonaField::BoxRetention
        | PersonaField::TumpelMcp => {
            bail!("a patch cannot unset {path}, so it keeps its value; set it to the value you want instead")
        }
        PersonaField::ConnectorNames | PersonaField::Files => return Err(container(path)),
        PersonaField::Name | PersonaField::Harness | PersonaField::SecretSetName => return Err(required(path)),
    })
}

fn key_name(key: &SecretKey) -> &SecretName {
    match key {
        SecretKey::SecretVariableKey(key) => &key.name,
        SecretKey::ApiKeyKey(key) => &key.name,
        SecretKey::SubscriptionTokenKey(key) => &key.name,
        SecretKey::CredentialFileKey(key) => &key.name,
    }
}

fn member_key<'a>(set: &'a SecretSet, member: &str) -> Result<&'a SecretKey> {
    set.keys
        .iter()
        .find(|key| key_name(key).as_str() == member)
        .ok_or_else(|| {
            anyhow!(
                "secret set {} has no member {member}; add it with tumpel secret-set set-member, which names its kind",
                *set.name
            )
        })
}

fn member_update(set: &SecretSet, member: &str, value: Option<SecretText>) -> Result<SecretMemberUpdate> {
    Ok(match member_key(set, member)? {
        SecretKey::SecretVariableKey(key) => SecretVariableUpdate {
            kind: SecretVariableUpdateKind::Variable,
            name: key.name.clone(),
            previous_name: None,
            value,
        }
        .into(),
        SecretKey::ApiKeyKey(key) => ApiKeyUpdate {
            kind: ApiKeyUpdateKind::ApiKey,
            name: key.name.clone(),
            previous_name: None,
            provider: key.provider,
            value,
        }
        .into(),
        SecretKey::SubscriptionTokenKey(key) => SubscriptionTokenUpdate {
            expires_at: key.expires_at,
            kind: SubscriptionTokenUpdateKind::SubscriptionToken,
            name: key.name.clone(),
            previous_name: None,
            provider: key.provider,
            value,
        }
        .into(),
        SecretKey::CredentialFileKey(key) => CredentialFileUpdate {
            expires_at: key.expires_at,
            kind: CredentialFileUpdateKind::CredentialFile,
            name: key.name.clone(),
            previous_name: None,
            provider: key.provider,
            value,
        }
        .into(),
    })
}

fn renamed(update: SecretMemberUpdate, name: SecretName) -> SecretMemberUpdate {
    match update {
        SecretMemberUpdate::SecretVariableUpdate(held) => SecretVariableUpdate {
            previous_name: Some(held.name),
            name,
            ..held
        }
        .into(),
        SecretMemberUpdate::ApiKeyUpdate(held) => ApiKeyUpdate {
            previous_name: Some(held.name),
            name,
            ..held
        }
        .into(),
        SecretMemberUpdate::SubscriptionTokenUpdate(held) => SubscriptionTokenUpdate {
            previous_name: Some(held.name),
            name,
            ..held
        }
        .into(),
        SecretMemberUpdate::CredentialFileUpdate(held) => CredentialFileUpdate {
            previous_name: Some(held.name),
            name,
            ..held
        }
        .into(),
    }
}

fn member_change(
    set: &SecretSet,
    member: &str,
    field: MemberField,
    path: &str,
    raw: &str,
) -> Result<SecretMemberUpdate> {
    let update = member_update(set, member, None)?;
    Ok(match (field, update) {
        (MemberField::Name, update) => renamed(update, value(path, raw)?),
        (MemberField::Provider, SecretMemberUpdate::ApiKeyUpdate(held)) => ApiKeyUpdate {
            provider: value(path, raw)?,
            ..held
        }
        .into(),
        (MemberField::Provider, SecretMemberUpdate::SubscriptionTokenUpdate(held)) => SubscriptionTokenUpdate {
            provider: value(path, raw)?,
            ..held
        }
        .into(),
        (MemberField::ExpiresAt, SecretMemberUpdate::SubscriptionTokenUpdate(held)) => SubscriptionTokenUpdate {
            expires_at: value(path, raw)?,
            ..held
        }
        .into(),
        (MemberField::Provider | MemberField::ExpiresAt, _) => bail!(
            "member {member} of secret set {} has no field {path}; tumpel config get /secretSets/{}/secrets/{member} prints the fields it has",
            *set.name,
            *set.name
        ),
        (MemberField::Kind, _) => bail!(
            "the kind of member {member} is fixed; delete the member with tumpel config delete and add it again with tumpel secret-set set-member"
        ),
        (MemberField::ValueFrom, _) => bail!(
            "valueFrom names the variable tumpel config apply reads, and only the file holds it; to change the stored value, pipe it in with tumpel config set /secretSets/{}/secrets/{member} --stdin",
            *set.name
        ),
    })
}

fn secret_set_write(live: &Live, path: &str, name: &str, field: SecretSetField, edit: Edit) -> Result<Write> {
    let set = secret_set(live, name)?;
    let id = set.secret_set_id.clone();
    Ok(match (field, edit) {
        (SecretSetField::Whole, Edit::Delete) => Write::DeleteSecretSet(id),
        (SecretSetField::Member(member), Edit::Delete) => {
            Write::RemoveMember(id, key_name(member_key(set, &member)?).clone())
        }
        (SecretSetField::Name, Edit::Set(raw)) => Write::SecretSet(
            id,
            SecretSetPatch {
                name: Some(value(path, &raw)?),
                secrets: Vec::new(),
            },
        ),
        (SecretSetField::MemberField(member, field), Edit::Set(raw)) => Write::SecretSet(
            id,
            SecretSetPatch {
                name: None,
                secrets: vec![member_change(set, &member, field, path, &raw)?],
            },
        ),
        (SecretSetField::Name | SecretSetField::MemberField(..), Edit::Delete) => return Err(required(path)),
        (SecretSetField::Whole | SecretSetField::Secrets | SecretSetField::Member(_), _) => {
            return Err(container(path))
        }
    })
}

fn pin_write(path: &str, harness: &str, field: PinField, edit: Edit) -> Result<Write> {
    let harness: Harness = harness
        .parse()
        .map_err(|error| anyhow!("{harness:?} is not a harness; tumpel harness list names them ({error})"))?;
    Ok(match (field, edit) {
        (PinField::HarnessVersion, Edit::Set(raw)) => Write::Pin(HarnessPinCreate {
            harness,
            harness_version: value(path, &raw)?,
        }),
        (PinField::Whole, Edit::Delete) => Write::Unpin(harness),
        (PinField::Harness, Edit::Set(_)) => {
            bail!("{path} names the pin itself; pin another harness at /harnessPins/<harness>/harnessVersion")
        }
        (PinField::Harness | PinField::HarnessVersion, Edit::Delete) => return Err(required(path)),
        (PinField::Whole, Edit::Set(_)) => return Err(container(path)),
    })
}

fn host_write(live: &Live, path: &str, host_id: &str, field: HostField, edit: Edit) -> Result<Write> {
    let host = live
        .hosts
        .iter()
        .find(|host| host.host_id.as_str() == host_id && host.state != HostState::Revoked)
        .ok_or_else(|| anyhow!("the tenant has no enrolled host {host_id}; tumpel config get /hosts prints them"))?;
    let id = host.host_id.clone();
    Ok(match (field, edit) {
        (HostField::Labels, Edit::Set(raw)) => {
            let labels: Vec<HostLabel> = value(path, &raw)?;
            Write::Host(
                id,
                HostLabelsPatch {
                    add: labels
                        .iter()
                        .filter(|label| !host.labels.contains(label))
                        .cloned()
                        .collect(),
                    remove: host
                        .labels
                        .iter()
                        .filter(|label| !labels.contains(label))
                        .cloned()
                        .collect(),
                },
            )
        }
        (HostField::Label(label), Edit::Delete) => {
            let held = host
                .labels
                .iter()
                .find(|held| held.as_str() == label)
                .ok_or_else(|| anyhow!("host {host_id} has no label {label}"))?;
            Write::Host(
                id,
                HostLabelsPatch {
                    add: Vec::new(),
                    remove: vec![held.clone()],
                },
            )
        }
        (HostField::Label(_), Edit::Set(_)) => return Err(list_entry(path)),
        (HostField::HostId, Edit::Set(_)) => bail!("{path} names the host itself and cannot change"),
        (HostField::HostId, Edit::Delete) => return Err(required(path)),
        (HostField::Labels, Edit::Delete) | (HostField::Whole, Edit::Set(_)) => return Err(container(path)),
        (HostField::Whole, Edit::Delete) => {
            bail!("a host leaves the tenant through tumpel host revoke {host_id}, which config delete never runs")
        }
    })
}

fn trigger_write(live: &Live, path: &str, name: &str, field: TriggerField, edit: Edit) -> Result<Write> {
    let id = trigger(live, name)?.trigger_id.clone();
    let (field, raw) = match (field, edit) {
        (TriggerField::Whole, Edit::Delete) => return Ok(Write::DeleteTrigger(id)),
        (TriggerField::Whole, Edit::Set(_)) => return Err(container(path)),
        (_, Edit::Delete) => return Err(required(path)),
        (field, Edit::Set(raw)) => (field, raw),
    };
    let body = match field {
        TriggerField::Name => TriggerPatch {
            name: Some(value(path, &raw)?),
            ..TriggerPatch::default()
        },
        TriggerField::PersonaName => TriggerPatch {
            persona_id: Some(persona(live, &value::<DisplayName>(path, &raw)?)?.persona_id.clone()),
            ..TriggerPatch::default()
        },
        TriggerField::Model => TriggerPatch {
            model: Some(value(path, &raw)?),
            ..TriggerPatch::default()
        },
        TriggerField::Prompt => TriggerPatch {
            prompt: Some(value(path, &raw)?),
            ..TriggerPatch::default()
        },
        TriggerField::Source => TriggerPatch {
            source: Some(value(path, &raw)?),
            ..TriggerPatch::default()
        },
        TriggerField::Whole => return Err(container(path)),
    };
    Ok(Write::Trigger(id, body))
}

fn plan(live: &Live, path: &str, request: Request) -> Result<Write> {
    let (target, edit) = match request {
        Request::Secret(set, member, value) => {
            let set = secret_set(live, &set)?;
            return Ok(Write::SecretSet(
                set.secret_set_id.clone(),
                SecretSetPatch {
                    name: None,
                    secrets: vec![member_update(set, &member, Some(value))?],
                },
            ));
        }
        Request::Edit(target, edit) => (target, edit),
    };
    match (target, edit) {
        (Target::Persona(name, field), Edit::Set(raw)) => persona_set(live, path, persona(live, &name)?, field, &raw),
        (Target::Persona(name, field), Edit::Delete) => persona_delete(live, path, persona(live, &name)?, field),
        (Target::SecretSet(name, field), edit) => secret_set_write(live, path, &name, field, edit),
        (Target::HarnessPin(harness, field), edit) => pin_write(path, &harness, field, edit),
        (Target::Host(host_id, field), edit) => host_write(live, path, &host_id, field, edit),
        (Target::Trigger(name, field), edit) => trigger_write(live, path, &name, field, edit),
    }
}

async fn written(client: &Client, endpoint: &str, write: Write) -> Result<()> {
    match write {
        Write::Persona(id, body) => print_json(&shown(client.personas_patch(&id, &body).await, || {
            format!("{endpoint}/v1/personas/{}", id.as_str())
        })?),
        Write::PutFile(id, path, local) => {
            let file = crate::bundle::persona_file(client, path, &local, "tumpel config set").await?;
            let body = PutPersonaFilePatch {
                file,
                kind: PutPersonaFilePatchKind::Put,
            }
            .into();
            print_json(&shown(client.personas_patch_file(&id, &body).await, || {
                format!("{endpoint}/v1/personas/{}/file-patches", id.as_str())
            })?)
        }
        Write::RemoveFile(id, path) => {
            let body = RemovePersonaFilePatch {
                kind: RemovePersonaFilePatchKind::Remove,
                path,
            }
            .into();
            print_json(&shown(client.personas_patch_file(&id, &body).await, || {
                format!("{endpoint}/v1/personas/{}/file-patches", id.as_str())
            })?)
        }
        Write::DeletePersona(id) => {
            answered(client.personas_delete(&id).await, || {
                format!("{endpoint}/v1/personas/{}", id.as_str())
            })?;
            eprintln!("deleted persona {}", id.as_str());
            Ok(())
        }
        Write::SecretSet(id, body) => print_json(&shown(client.secret_sets_patch(&id, &body).await, || {
            format!("{endpoint}/v1/secret-sets/{}", id.as_str())
        })?),
        Write::RemoveMember(id, name) => {
            print_json(&shown(client.secret_sets_remove_member(&id, &name).await, || {
                format!("{endpoint}/v1/secret-sets/{}/members/{}", id.as_str(), name.as_str())
            })?)
        }
        Write::DeleteSecretSet(id) => {
            answered(client.secret_sets_delete(&id).await, || {
                format!("{endpoint}/v1/secret-sets/{}", id.as_str())
            })?;
            eprintln!("deleted secret set {}", id.as_str());
            Ok(())
        }
        Write::Pin(body) => print_json(&shown(client.harness_catalog_pin(&body).await, || {
            format!("{endpoint}/v1/harnesses/pins")
        })?),
        Write::Unpin(harness) => {
            answered(client.harness_catalog_unpin(harness).await, || {
                format!("{endpoint}/v1/harnesses/pins/{harness}")
            })?;
            eprintln!("unpinned {harness}");
            Ok(())
        }
        Write::Host(id, body) => print_json(&shown(client.hosts_patch_labels(&id, &body).await, || {
            format!("{endpoint}/v1/hosts/{}/labels", id.as_str())
        })?),
        Write::Trigger(id, body) => print_json(&shown(client.triggers_update(&id, &body).await, || {
            format!("{endpoint}/v1/triggers/{}", id.as_str())
        })?),
        Write::DeleteTrigger(id) => {
            answered(client.triggers_delete(&id).await, || {
                format!("{endpoint}/v1/triggers/{}", id.as_str())
            })?;
            eprintln!("deleted trigger {}", id.as_str());
            Ok(())
        }
    }
}

fn entry_key(entry: &Value) -> Option<&str> {
    match entry {
        Value::String(text) => Some(text),
        Value::Object(fields) => ["name", "harness", "hostId", "path"]
            .iter()
            .find_map(|key| fields.get(*key)?.as_str()),
        _ => None,
    }
}

fn step<'a>(here: &'a Value, rest: &[String]) -> Option<(&'a Value, usize)> {
    let first = rest.first()?;
    match here {
        Value::Object(fields) => fields.get(first.as_str()).map(|field| (field, 1)),
        Value::Array(entries) => (1..=rest.len())
            .rev()
            .find_map(|used| {
                let key = rest[..used].join("/");
                entries
                    .iter()
                    .find(|entry| entry_key(entry) == Some(key.as_str()))
                    .map(|entry| (entry, used))
            })
            .or_else(|| {
                first
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| entries.get(index))
                    .map(|entry| (entry, 1))
            }),
        _ => None,
    }
}

fn located<'a>(document: &'a Value, path: &str) -> Result<&'a Value> {
    let segments = segments(path)?;
    let mut here = document;
    let mut rest = segments.as_slice();
    while !rest.is_empty() {
        let (next, used) = step(here, rest).ok_or_else(|| {
            anyhow!("the tenant config has no field at {path}; tumpel config get / prints every field")
        })?;
        here = next;
        rest = &rest[used..];
    }
    Ok(here)
}

pub async fn get(path: &str) -> Result<()> {
    let (_, client) = signed_in()?;
    let live = tenant_config::live(&client).await?;
    let document = serde_json::to_value(tenant_config::starter(&live)?)?;
    print_json(located(&document, path)?)
}

async fn changed(path: &str, change: Change) -> Result<()> {
    let request = request(path, change)?;
    let (credentials, client) = signed_in()?;
    let live = tenant_config::live(&client).await?;
    written(&client, &credentials.endpoint, plan(&live, path, request)?).await
}

pub async fn set(path: &str, value: Option<String>, stdin: bool) -> Result<()> {
    let change = match value {
        Some(raw) => Change::Set(raw),
        None => Change::Secret(read_stdin(stdin, path)?),
    };
    changed(path, change).await
}

pub async fn delete(path: &str) -> Result<()> {
    changed(path, Change::Delete).await
}

#[cfg(test)]
mod tests {
    use super::{
        located, plan, request, target, Change, MemberField, PersonaField, Request, SecretSetField, Target, Write,
    };
    use crate::tenant_config::Live;
    use serde_json::json;
    use tumpel_api::types::{PersonaPatch, SecretSetPatch, SecretVariableUpdate, SecretVariableUpdateKind};

    #[test]
    fn a_path_addresses_an_entry_by_its_name_and_a_persona_file_by_its_whole_path() {
        assert_eq!(
            target("/personas/reviewer/boxMemoryMib").unwrap(),
            Target::Persona("reviewer".into(), PersonaField::BoxMemoryMib)
        );
        assert_eq!(
            target("/personas/reviewer/files/.claude/settings.json").unwrap(),
            Target::Persona("reviewer".into(), PersonaField::File(".claude/settings.json".into()))
        );
        assert_eq!(
            target("/personas/a~1b~0c/name").unwrap(),
            Target::Persona("a/b~c".into(), PersonaField::Name)
        );
        assert_eq!(
            target("/secretSets/prod/secrets/ANTHROPIC_API_KEY/expiresAt").unwrap(),
            Target::SecretSet(
                "prod".into(),
                SecretSetField::MemberField("ANTHROPIC_API_KEY".into(), MemberField::ExpiresAt)
            )
        );
        for refused in [
            "personas/reviewer/name",
            "/personas//name",
            "/personas/reviewer/colour",
            "/$schema",
            "/triggers",
        ] {
            assert!(target(refused).is_err(), "{refused} parsed");
        }
    }

    #[test]
    fn get_walks_lists_by_entry_key_and_a_file_path_with_slashes() {
        let document = json!({
            "personas": [{
                "name": "reviewer",
                "files": [{ "path": ".claude/settings.json", "from": "persona-files/reviewer/.claude/settings.json" }],
                "connectorNames": ["github"],
            }],
            "harnessPins": [{ "harness": "ClaudeCode", "harnessVersion": "2.1.0" }],
            "hosts": [{ "hostId": "h1", "labels": ["gpu"] }],
        });
        assert_eq!(
            located(&document, "/personas/reviewer/files/.claude/settings.json/from").unwrap(),
            &json!("persona-files/reviewer/.claude/settings.json")
        );
        assert_eq!(
            located(&document, "/harnessPins/ClaudeCode/harnessVersion").unwrap(),
            &json!("2.1.0")
        );
        assert_eq!(located(&document, "/hosts/h1/labels/gpu").unwrap(), &json!("gpu"));
        assert_eq!(located(&document, "/").unwrap(), &document);
        assert!(located(&document, "/personas/critic").is_err());
    }

    #[test]
    fn a_secret_value_comes_over_stdin_only_and_is_never_echoed() {
        let error = request(
            "/secretSets/prod/secrets/GITHUB_TOKEN",
            Change::Set("ghp-secret".into()),
        )
        .err()
        .expect("a secret value on the command line is refused");
        assert!(error.to_string().contains("--stdin"), "{error}");
        assert!(!error.to_string().contains("ghp-secret"), "{error}");
        assert!(request("/personas/reviewer/name", Change::Secret("x".parse().unwrap())).is_err());
        assert!(matches!(
            request(
                "/secretSets/prod/secrets/GITHUB_TOKEN",
                Change::Secret("x".parse().unwrap())
            )
            .ok(),
            Some(Request::Secret(..))
        ));
    }

    #[test]
    fn each_path_maps_to_the_patch_that_changes_that_field_alone() {
        let live = Live {
            personas: vec![serde_json::from_value(json!({
                "personaId": "7d0f0b8e-0000-4000-8000-000000000001",
                "tenantId": "7d0f0b8e-0000-4000-8000-000000000009",
                "name": "reviewer",
                "harness": "ClaudeCode",
                "secretSetId": "7d0f0b8e-0000-4000-8000-000000000002",
                "boxRetention": "Auto",
                "connectorIds": [],
                "files": [],
                "tumpelMcp": false,
                "createdAt": 1,
                "updatedAt": 1
            }))
            .unwrap()],
            secret_sets: vec![serde_json::from_value(json!({
                "secretSetId": "7d0f0b8e-0000-4000-8000-000000000002",
                "tenantId": "7d0f0b8e-0000-4000-8000-000000000009",
                "name": "prod",
                "keys": [{ "kind": "Variable", "name": "GITHUB_TOKEN", "updatedAt": 1 }],
                "createdAt": 1,
                "updatedAt": 1
            }))
            .unwrap()],
            pins: Vec::new(),
            hosts: Vec::new(),
            connectors: Vec::new(),
            triggers: Vec::new(),
        };
        let persona = "7d0f0b8e-0000-4000-8000-000000000001".parse().unwrap();
        let set = "7d0f0b8e-0000-4000-8000-000000000002".parse().unwrap();
        let planned = |path: &str, change: Change| plan(&live, path, request(path, change)?);

        assert_eq!(
            planned("/personas/reviewer/boxMemoryMib", Change::Set("4096".into())).unwrap(),
            Write::Persona(
                persona,
                PersonaPatch {
                    box_memory_mib: Some(4096),
                    ..PersonaPatch::default()
                }
            )
        );
        assert!(planned("/personas/reviewer/boxMemoryMib", Change::Delete).is_err());
        assert_eq!(
            planned(
                "/secretSets/prod/secrets/GITHUB_TOKEN",
                Change::Secret("ghp-new".parse().unwrap())
            )
            .unwrap(),
            Write::SecretSet(
                set,
                SecretSetPatch {
                    name: None,
                    secrets: vec![SecretVariableUpdate {
                        kind: SecretVariableUpdateKind::Variable,
                        name: "GITHUB_TOKEN".parse().unwrap(),
                        previous_name: None,
                        value: Some("ghp-new".parse().unwrap()),
                    }
                    .into()],
                }
            )
        );
        assert!(matches!(
            planned("/secretSets/prod/secrets/GITHUB_TOKEN/name", Change::Set("\"GH_TOKEN\"".into())),
            Ok(Write::SecretSet(_, SecretSetPatch { secrets, .. }))
                if secrets == vec![SecretVariableUpdate {
                    kind: SecretVariableUpdateKind::Variable,
                    name: "GH_TOKEN".parse().unwrap(),
                    previous_name: Some("GITHUB_TOKEN".parse().unwrap()),
                    value: None,
                }
                .into()]
        ));
        assert!(matches!(
            planned("/secretSets/prod/secrets/GITHUB_TOKEN", Change::Delete),
            Ok(Write::RemoveMember(..))
        ));
        assert!(planned("/secretSets/prod/secrets/MISSING", Change::Delete).is_err());
    }
}
