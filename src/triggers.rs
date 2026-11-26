use crate::resources::read_json;
use crate::sessions::{id, parsed};
use crate::{answered, print_json, shown, signed_in, TriggerCommand, TriggerFields};
use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use tumpel_api::types::{
    DraftTriggerTest, DraftTriggerTestKind, GitHubTriggerSource, Prompt, StoredTriggerTest, StoredTriggerTestKind,
    TriggerCondition, TriggerCreate, TriggerPatch, TriggerSource, TriggerTest,
};

fn grouped(wheres: &[String]) -> Result<Vec<(String, Vec<String>)>> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for spec in wheres {
        let (field, value) = spec
            .split_once('=')
            .ok_or_else(|| anyhow!("--where {spec:?} is not <field>=<value>, such as action=closed"))?;
        match groups.iter_mut().find(|(known, _)| known == field) {
            Some((_, values)) => values.push(value.to_string()),
            None => groups.push((field.to_string(), vec![value.to_string()])),
        }
    }
    Ok(groups)
}

fn conditions(wheres: &[String]) -> Result<Vec<TriggerCondition>> {
    grouped(wheres)?
        .into_iter()
        .map(|(field, values)| {
            Ok(TriggerCondition {
                field: parsed("--where", &field)?,
                values: values
                    .iter()
                    .map(|value| parsed("--where", value))
                    .collect::<Result<_>>()?,
            })
        })
        .collect()
}

fn source(kind: &str, event: &str, wheres: &[String]) -> Result<TriggerSource> {
    Ok(GitHubTriggerSource {
        conditions: conditions(wheres)?,
        event: parsed("--event", event)?,
        kind: parsed("--source", kind)?,
    }
    .into())
}

fn prompt(text: Option<String>, file: Option<PathBuf>) -> Result<Option<Prompt>> {
    if let Some(file) = file {
        let text = std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
        return Ok(Some(parsed("--prompt-file", &text)?));
    }
    text.map(|text| parsed("--prompt", &text)).transpose()
}

fn trigger_create(file: Option<PathBuf>, fields: Option<TriggerFields>) -> Result<TriggerCreate> {
    let Some(fields) = fields else {
        let body: TriggerCreate = read_json(&file.unwrap_or_default(), "TriggerCreate")?;
        return Ok(TriggerCreate {
            session_id: None,
            ..body
        });
    };
    Ok(TriggerCreate {
        model: parsed("--model", &fields.model)?,
        name: parsed("--name", &fields.name)?,
        persona_id: id("--persona-id", &fields.persona_id)?,
        prompt: prompt(fields.prompt, fields.prompt_file)?.ok_or_else(|| anyhow!("pass --prompt or --prompt-file"))?,
        session_id: None,
        source: source(&fields.source, &fields.event, &fields.condition)?,
    })
}

pub async fn trigger(command: TriggerCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let url = format!("{}/v1/triggers", credentials.endpoint);
    match command {
        TriggerCommand::List => print_json(&shown(client.triggers_list().await, || url)?),
        TriggerCommand::Get { trigger_id } => {
            let trigger = id("triggerId", &trigger_id)?;
            print_json(&shown(client.triggers_get(&trigger).await, || {
                format!("{url}/{trigger_id}")
            })?)
        }
        TriggerCommand::Create { file, fields } => {
            let body = trigger_create(file, fields)?;
            print_json(&shown(client.triggers_create(&body).await, || url)?)
        }
        TriggerCommand::Update {
            trigger_id,
            file,
            name,
            source: kind,
            event,
            condition,
            persona_id,
            model,
            prompt: text,
            prompt_file,
        } => {
            let trigger = id("triggerId", &trigger_id)?;
            let body = match file {
                Some(file) => read_json(&file, "TriggerPatch")?,
                None => TriggerPatch {
                    model: model.map(|value| parsed("--model", &value)).transpose()?,
                    name: name.map(|value| parsed("--name", &value)).transpose()?,
                    persona_id: persona_id.map(|value| id("--persona-id", &value)).transpose()?,
                    prompt: prompt(text, prompt_file)?,
                    source: kind
                        .zip(event)
                        .map(|(kind, event)| source(&kind, &event, &condition))
                        .transpose()?,
                },
            };
            print_json(&shown(client.triggers_update(&trigger, &body).await, || {
                format!("{url}/{trigger_id}")
            })?)
        }
        TriggerCommand::Delete { trigger_id } => {
            let trigger = id("triggerId", &trigger_id)?;
            answered(client.triggers_delete(&trigger).await, || format!("{url}/{trigger_id}"))?;
            eprintln!("deleted trigger {trigger_id}");
            Ok(())
        }
        TriggerCommand::Test {
            trigger_id,
            file,
            payload_file,
        } => {
            let payload = read_json(&payload_file, "TriggerPayload")?;
            let body: TriggerTest = match trigger_id {
                Some(trigger_id) => StoredTriggerTest {
                    kind: StoredTriggerTestKind::Stored,
                    payload,
                    trigger_id: id("triggerId", &trigger_id)?,
                }
                .into(),
                None => DraftTriggerTest {
                    kind: DraftTriggerTestKind::Draft,
                    payload,
                    trigger: read_json(&file.unwrap_or_default(), "TriggerCreate")?,
                }
                .into(),
            };
            print_json(&shown(client.triggers_test(&body).await, || format!("{url}/test"))?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::grouped;

    #[test]
    fn a_repeated_field_gathers_its_values_and_other_fields_stay_apart_in_order() {
        let wheres = [
            "action=closed".to_string(),
            "sender.login=jdoe".to_string(),
            "action=reopened".to_string(),
            "label.name=agent:work=now".to_string(),
        ];
        assert_eq!(
            grouped(&wheres).expect("every --where holds an ="),
            vec![
                ("action".to_string(), vec!["closed".to_string(), "reopened".to_string()]),
                ("sender.login".to_string(), vec!["jdoe".to_string()]),
                ("label.name".to_string(), vec!["agent:work=now".to_string()]),
            ]
        );
    }

    #[test]
    fn a_where_without_an_equals_sign_is_refused() {
        assert!(grouped(&["action".to_string()]).is_err());
    }
}
