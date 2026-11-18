use crate::bundle;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::Path;
use tumpel_api::types::{
    ApiKeyUpdate, ApiKeyUpdateKind, ApiKeyValue, ApiKeyValueKind, ConfigApiKey, ConfigApiKeyKind, ConfigExpiry,
    ConfigExpiryValue, ConfigHost, ConfigPersona, ConfigPersonaFile, ConfigPlan, ConfigSecret, ConfigSecretSet,
    ConfigSecretValue, ConfigSecretVariable, ConfigSecretVariableKind, ConfigSubscriptionToken,
    ConfigSubscriptionTokenKind, Connector, DisplayName, EffectiveHarnessVersion, EntityId, EnvironmentVariableName,
    HarnessPinCreate, Host, HostState, Persona, PersonaFile, PersonaFilePath, Problem, SecretKey, SecretMemberUpdate,
    SecretName, SecretSet, SecretValue, SecretValueSource, SecretVariableUpdate, SecretVariableUpdateKind,
    SecretVariableValue, SecretVariableValueKind, Sha256Digest, SubscriptionTokenUpdate, SubscriptionTokenUpdateKind,
    SubscriptionTokenValue, SubscriptionTokenValueKind, TenantConfig, TenantConfigApply, TenantConfigPlanRequest,
};
use tumpel_api::{Client, Error, TENANT_CONFIG_SCHEMA};

fn held_digest(file: &PersonaFile) -> &Sha256Digest {
    match file {
        PersonaFile::InlinePersonaFileRead(file) => &file.digest,
        PersonaFile::StoredPersonaFileRead(file) => &file.digest,
    }
}

fn connector_identity(connector: &Connector) -> (&EntityId, &DisplayName) {
    match connector {
        Connector::StaticConnector(connector) => (&connector.connector_id, &connector.name),
        Connector::OAuthConnector(connector) => (&connector.connector_id, &connector.name),
    }
}

fn held_path(file: &PersonaFile) -> &PersonaFilePath {
    match file {
        PersonaFile::InlinePersonaFileRead(file) => &file.path,
        PersonaFile::StoredPersonaFileRead(file) => &file.path,
    }
}

fn schema() -> Result<Value> {
    serde_json::from_str(TENANT_CONFIG_SCHEMA).context("the built-in TenantConfig schema is not JSON; rebuild tumpel")
}

pub fn schema_errors(document: &Value) -> Result<Vec<String>> {
    let validator = jsonschema::validator_for(&schema()?)
        .map_err(|error| anyhow!("the built-in TenantConfig schema does not compile: {error}"))?;
    Ok(validator
        .iter_errors(document)
        .map(|error| {
            let pointer = error.instance_path().to_string();
            format!(
                "{}: {}",
                if pointer.is_empty() { "/" } else { &pointer },
                error.masked()
            )
        })
        .collect())
}

pub fn parse(file: &Path) -> Result<TenantConfig> {
    let text = std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let document: Value = serde_json::from_str(&text).with_context(|| {
        format!(
            "{} is not JSON; fix the syntax at the line and column above",
            file.display()
        )
    })?;
    let errors = schema_errors(&document)?;
    if !errors.is_empty() {
        bail!(
            "{} does not match the TenantConfig schema:\n  {}\nFix each key named above; tumpel config init writes a valid starter file.",
            file.display(),
            errors.join("\n  ")
        );
    }
    serde_json::from_value(document).with_context(|| {
        format!(
            "{} matches the schema but does not read as a TenantConfig",
            file.display()
        )
    })
}

fn refused(problem: &Problem) -> anyhow::Error {
    anyhow!("{}: {}. {}", problem.title, problem.detail, problem.fix)
}

async fn answer<T>(
    call: impl std::future::Future<Output = Result<tumpel_api::ResponseValue<T>, Error<Problem>>>,
    what: &str,
) -> Result<T> {
    match call.await {
        Ok(value) => Ok(value.into_inner()),
        Err(Error::ErrorResponse(problem)) => Err(refused(&problem)).with_context(|| what.to_string()),
        Err(error) => Err(error).with_context(|| what.to_string()),
    }
}

pub struct Live {
    pub personas: Vec<Persona>,
    pub secret_sets: Vec<SecretSet>,
    pub pins: Vec<EffectiveHarnessVersion>,
    pub hosts: Vec<Host>,
    pub connectors: Vec<Connector>,
}

pub async fn live(client: &Client) -> Result<Live> {
    Ok(Live {
        personas: answer(client.personas_list(), "listing personas").await?,
        secret_sets: answer(client.secret_sets_list(), "listing secret sets").await?,
        pins: answer(client.harness_catalog_list_pins(), "listing harness pins").await?,
        hosts: answer(client.hosts_list(), "listing hosts").await?,
        connectors: answer(client.connectors_list(), "listing connectors").await?,
    })
}

pub fn expiry_values(config: &TenantConfig, lookup: impl Fn(&str) -> Option<String>) -> Result<Vec<ConfigExpiryValue>> {
    let mut values = BTreeMap::new();
    for secret_set in &config.secret_sets {
        for secret in &secret_set.secrets {
            let ConfigSecret::SubscriptionToken(token) = secret else {
                continue;
            };
            let ConfigExpiry::SecretValueSource(from) = &token.expires_at else {
                continue;
            };
            let name = from.env.as_str();
            let text = lookup(name).filter(|value| !value.is_empty()).ok_or_else(|| {
                anyhow!(
                    "${name} is unset or empty; member {} of secret set {} reads its expiresAt from it, so export the token's expiry as an ISO 8601 date-time with an offset",
                    *token.name,
                    *secret_set.name
                )
            })?;
            let expires_at = chrono::DateTime::parse_from_rfc3339(&text).map_err(|error| {
                anyhow!(
                    "${name} is not an ISO 8601 date-time with an offset ({error}); member {} of secret set {} reads its expiresAt from it",
                    *token.name,
                    *secret_set.name
                )
            })?;
            values.insert(
                name.to_string(),
                ConfigExpiryValue {
                    env: from.env.clone(),
                    expires_at,
                },
            );
        }
    }
    Ok(values.into_values().collect())
}

pub fn source(secret: &ConfigSecret) -> (&SecretName, &SecretValueSource) {
    match secret {
        ConfigSecret::SecretVariable(variable) => (&variable.name, &variable.value_from),
        ConfigSecret::ApiKey(key) => (&key.name, &key.value_from),
        ConfigSecret::SubscriptionToken(token) => (&token.name, &token.value_from),
    }
}

pub fn secret_values<'a>(
    secret_sets: impl IntoIterator<Item = &'a ConfigSecretSet>,
    lookup: impl Fn(&str) -> Option<String>,
) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for secret_set in secret_sets {
        for secret in &secret_set.secrets {
            let name = source(secret).1.env.as_str();
            if let Some(value) = lookup(name).filter(|value| !value.is_empty()) {
                values.insert(name.to_string(), value);
            }
        }
    }
    values
}

fn env_names<'a>(names: impl IntoIterator<Item = &'a String>) -> Result<Vec<EnvironmentVariableName>> {
    names
        .into_iter()
        .map(|env| {
            env.clone()
                .try_into()
                .map_err(|error| anyhow!("{env} is not an environment variable name: {error}"))
        })
        .collect()
}

pub fn with_values(secret_set: &ConfigSecretSet, values: &BTreeMap<String, String>) -> Result<Vec<SecretValue>> {
    secret_set
        .secrets
        .iter()
        .map(|secret| {
            let (member, from) = source(secret);
            let value = values
                .get(from.env.as_str())
                .cloned()
                .ok_or_else(|| {
                    anyhow!(
                        "${} is unset or empty; export it with the value of member {}, then run this again",
                        *from.env,
                        **member
                    )
                })?
                .try_into()
                .map_err(|error| {
                    anyhow!(
                        "${} does not hold a usable value for member {} ({error})",
                        *from.env,
                        **member
                    )
                })?;
            Ok(match secret {
                ConfigSecret::SecretVariable(variable) => SecretVariableValue {
                    kind: SecretVariableValueKind::Variable,
                    name: variable.name.clone(),
                    value,
                }
                .into(),
                ConfigSecret::ApiKey(key) => ApiKeyValue {
                    kind: ApiKeyValueKind::ApiKey,
                    name: key.name.clone(),
                    provider: key.provider,
                    value,
                }
                .into(),
                ConfigSecret::SubscriptionToken(token) => SubscriptionTokenValue {
                    expires_at: match &token.expires_at {
                        ConfigExpiry::DateTime(expires_at) => *expires_at,
                        ConfigExpiry::SecretValueSource(from) => bail!(
                            "the expiresAt of member {} names ${}; give it as an ISO 8601 date-time with an offset",
                            *token.name,
                            *from.env
                        ),
                    },
                    kind: SubscriptionTokenValueKind::SubscriptionToken,
                    name: token.name.clone(),
                    provider: token.provider,
                    value,
                }
                .into(),
            })
        })
        .collect()
}

pub fn with_kept_values(
    secret_set: &ConfigSecretSet,
    values: &BTreeMap<String, String>,
) -> Result<Vec<SecretMemberUpdate>> {
    secret_set
        .secrets
        .iter()
        .map(|secret| {
            let (member, from) = source(secret);
            let value = values
                .get(from.env.as_str())
                .map(|value| {
                    value.clone().try_into().map_err(|error| {
                        anyhow!(
                            "${} does not hold a usable value for member {} ({error})",
                            *from.env,
                            **member
                        )
                    })
                })
                .transpose()?;
            Ok(match secret {
                ConfigSecret::SecretVariable(variable) => SecretVariableUpdate {
                    kind: SecretVariableUpdateKind::Variable,
                    name: variable.name.clone(),
                    previous_name: None,
                    value,
                }
                .into(),
                ConfigSecret::ApiKey(key) => ApiKeyUpdate {
                    kind: ApiKeyUpdateKind::ApiKey,
                    name: key.name.clone(),
                    previous_name: None,
                    provider: key.provider,
                    value,
                }
                .into(),
                ConfigSecret::SubscriptionToken(token) => SubscriptionTokenUpdate {
                    expires_at: match &token.expires_at {
                        ConfigExpiry::DateTime(expires_at) => *expires_at,
                        ConfigExpiry::SecretValueSource(from) => bail!(
                            "the expiresAt of member {} names ${}; give it as an ISO 8601 date-time with an offset",
                            *token.name,
                            *from.env
                        ),
                    },
                    kind: SubscriptionTokenUpdateKind::SubscriptionToken,
                    name: token.name.clone(),
                    previous_name: None,
                    provider: token.provider,
                    value,
                }
                .into(),
            })
        })
        .collect()
}

pub fn print_plan(plan: &ConfigPlan) {
    if plan.changes.is_empty() {
        println!("no changes; the tenant matches the file");
        return;
    }
    for change in &plan.changes {
        println!("{}", change.description);
    }
}

pub async fn plan(client: &Client, file: &Path, lookup: impl Fn(&str) -> Option<String>) -> Result<ConfigPlan> {
    let config = parse(file)?;
    let files = bundle::read(file, &config)?;
    let sent = secret_values(&config.secret_sets, &lookup);
    let body = TenantConfigPlanRequest {
        expiry_values: expiry_values(&config, &lookup)?,
        files: files.iter().map(bundle::digest).collect::<Result<_>>()?,
        secret_values: env_names(sent.keys())?
            .into_iter()
            .map(|env| SecretValueSource { env })
            .collect(),
        config,
    };
    answer(client.tenant_configs_plan(&body), "planning the tenant config").await
}

pub async fn apply(client: &Client, file: &Path, lookup: impl Fn(&str) -> Option<String>) -> Result<ConfigPlan> {
    let config = parse(file)?;
    let files = bundle::read(file, &config)?;
    let values = secret_values(&config.secret_sets, &lookup);
    let personas = answer(client.personas_list(), "listing personas").await?;
    let held: HashSet<&str> = personas
        .iter()
        .flat_map(|persona| persona.files.iter())
        .map(|file| held_digest(file).as_str())
        .collect();
    let mut writes = Vec::with_capacity(files.len());
    for local in &files {
        writes.push(bundle::write(client, local, held.contains(local.digest.as_str())).await?);
    }
    let body = TenantConfigApply {
        expiry_values: expiry_values(&config, &lookup)?,
        files: writes,
        secret_values: values
            .into_iter()
            .map(|(env, value)| {
                Ok(ConfigSecretValue {
                    env: env
                        .clone()
                        .try_into()
                        .map_err(|error| anyhow!("{env} is not an environment variable name: {error}"))?,
                    value: value
                        .try_into()
                        .map_err(|error| anyhow!("${env} does not hold a usable secret value ({error})"))?,
                })
            })
            .collect::<Result<_>>()?,
        config,
    };
    answer(client.tenant_configs_apply(&body), "applying the tenant config").await
}

fn env_name(secret_set: &SecretSet, key: &str) -> Result<EnvironmentVariableName> {
    let raw: String = format!("{}_{key}", *secret_set.name)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    let name = if raw.starts_with(|c: char| c.is_ascii_digit()) {
        format!("_{raw}")
    } else {
        raw
    };
    name.clone()
        .try_into()
        .map_err(|error| anyhow!("{name:?} is not a usable environment variable name: {error}"))
}

fn starter_member(secret_set: &SecretSet, key: &SecretKey) -> Result<ConfigSecret> {
    Ok(match key {
        SecretKey::SecretVariableKey(variable) => ConfigSecretVariable {
            kind: ConfigSecretVariableKind::Variable,
            name: variable.name.clone(),
            value_from: SecretValueSource {
                env: env_name(secret_set, &variable.name)?,
            },
        }
        .into(),
        SecretKey::ApiKeyKey(api_key) => ConfigApiKey {
            kind: ConfigApiKeyKind::ApiKey,
            name: api_key.name.clone(),
            provider: api_key.provider,
            value_from: SecretValueSource {
                env: env_name(secret_set, &api_key.name)?,
            },
        }
        .into(),
        SecretKey::SubscriptionTokenKey(token) => ConfigSubscriptionToken {
            expires_at: token.expires_at.into(),
            kind: ConfigSubscriptionTokenKind::SubscriptionToken,
            name: token.name.clone(),
            provider: token.provider,
            value_from: SecretValueSource {
                env: env_name(secret_set, &token.name)?,
            },
        }
        .into(),
        SecretKey::CredentialFileKey(file) => bail!(
            "secret set {} holds the login file {}, which a tenant config cannot carry; remove it from the set, apply the config, then sign in again from the dashboard",
            *secret_set.name,
            *file.name
        ),
    })
}

pub fn starter_path(persona: &str, path: &str) -> String {
    let folder: String = persona
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("persona-files/{folder}/{path}")
}

pub fn starter(live: &Live) -> Result<TenantConfig> {
    let secret_sets = live
        .secret_sets
        .iter()
        .map(|secret_set| {
            Ok(ConfigSecretSet {
                name: secret_set.name.clone(),
                secrets: secret_set
                    .keys
                    .iter()
                    .map(|key| starter_member(secret_set, key))
                    .collect::<Result<_>>()?,
            })
        })
        .collect::<Result<_>>()?;
    let connector_names: HashMap<&str, &DisplayName> = live
        .connectors
        .iter()
        .map(connector_identity)
        .map(|(id, name)| (id.as_str(), name))
        .collect();
    let set_names: HashMap<&str, &SecretSet> = live
        .secret_sets
        .iter()
        .map(|set| (set.secret_set_id.as_str(), set))
        .collect();
    Ok(TenantConfig {
        schema: None,
        personas: live
            .personas
            .iter()
            .map(|persona| {
                let secret_set = set_names.get(persona.secret_set_id.as_str()).ok_or_else(|| {
                    anyhow!(
                        "persona {} names secret set {}, which tumpel secret-set list does not show",
                        *persona.name,
                        *persona.secret_set_id
                    )
                })?;
                let connector_names = persona
                    .connector_ids
                    .iter()
                    .map(|id| {
                        connector_names
                            .get(id.as_str())
                            .map(|name| (*name).clone())
                            .ok_or_else(|| {
                                anyhow!(
                                    "persona {} lists connector {}, which tumpel connector list does not show",
                                    *persona.name,
                                    **id
                                )
                            })
                    })
                    .collect::<Result<_>>()?;
                Ok(ConfigPersona {
                    box_retention: Some(persona.box_retention),
                    box_memory_mib: persona.box_memory_mib,
                    box_disk_mib: persona.box_disk_mib,
                    connector_names,
                    files: persona
                        .files
                        .iter()
                        .map(|file| {
                            let from = starter_path(&persona.name, held_path(file));
                            Ok(ConfigPersonaFile {
                                from: from
                                    .clone()
                                    .try_into()
                                    .map_err(|error| anyhow!("{from} is not a usable local path: {error}"))?,
                                path: held_path(file).clone(),
                            })
                        })
                        .collect::<Result<_>>()?,
                    harness: persona.harness,
                    name: persona.name.clone(),
                    secret_set_name: secret_set.name.clone(),
                })
            })
            .collect::<Result<_>>()?,
        secret_sets,
        harness_pins: live
            .pins
            .iter()
            .filter(|pin| pin.pinned)
            .map(|pin| HarnessPinCreate {
                harness: pin.harness,
                harness_version: pin.harness_version.clone(),
            })
            .collect(),
        hosts: live
            .hosts
            .iter()
            .filter(|host| host.state != HostState::Revoked)
            .map(|host| ConfigHost {
                host_id: host.host_id.clone(),
                labels: host.labels.clone(),
            })
            .collect(),
    })
}

pub fn write_new(file: &Path, config: &TenantConfig) -> Result<()> {
    let text = format!("{}\n", serde_json::to_string_pretty(&serde_json::to_value(config)?)?);
    let mut handle = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(file)
        .with_context(|| format!("creating {}; pick a path where no file exists yet", file.display()))?;
    handle
        .write_all(text.as_bytes())
        .with_context(|| format!("writing {}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::{expiry_values, parse, schema_errors};
    use serde_json::json;
    use std::path::PathBuf;
    use tumpel_api::types::{ConfigExpiry, ConfigSecret};

    const EXPIRY_VARIABLE: &str = "TUMPEL_TEST_TOKEN_EXPIRES_AT";

    fn config_with_expiry_from_env(name: &str) -> PathBuf {
        let file = std::env::temp_dir().join(format!("tumpel-{}-{name}.json", std::process::id()));
        let document = json!({
            "$schema": "https://example.test/TenantConfig.json",
            "personas": [],
            "secretSets": [{
                "name": "prod",
                "secrets": [{
                    "kind": "SubscriptionToken", "name": "ANTHROPIC_API_KEY", "provider": "Anthropic",
                    "expiresAt": { "env": EXPIRY_VARIABLE }, "valueFrom": { "env": "ANTHROPIC_API_KEY" },
                }],
            }],
            "harnessPins": [],
            "hosts": [],
        });
        std::fs::write(&file, document.to_string()).expect("the test config is written");
        file
    }

    fn expiry(config: &tumpel_api::types::TenantConfig) -> &ConfigExpiry {
        let ConfigSecret::SubscriptionToken(token) = &config.secret_sets[0].secrets[0] else {
            panic!("the first secret is not a subscription token");
        };
        &token.expires_at
    }

    #[test]
    fn validate_reads_an_expiry_variable_name_without_its_value() {
        let file = config_with_expiry_from_env("validate");
        let config = parse(&file).expect("the file matches the schema");
        assert!(
            matches!(expiry(&config), ConfigExpiry::SecretValueSource(from) if from.env.as_str() == EXPIRY_VARIABLE)
        );
    }

    #[test]
    fn plan_fails_naming_an_unset_expiry_variable() {
        let file = config_with_expiry_from_env("plan");
        let config = parse(&file).expect("the file matches the schema");
        let error = expiry_values(&config, |_| None).expect_err("the expiry variable is unset");
        assert!(error.to_string().contains(EXPIRY_VARIABLE), "{error}");
    }

    #[test]
    fn apply_resolves_a_set_expiry_variable() {
        let file = config_with_expiry_from_env("apply");
        let config = parse(&file).expect("the file matches the schema");
        let values = expiry_values(&config, |name| {
            (name == EXPIRY_VARIABLE).then(|| "2026-12-01T00:00:00+01:00".to_string())
        })
        .expect("the expiry variable is set");
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].env.as_str(), EXPIRY_VARIABLE);
        assert_eq!(values[0].expires_at.to_rfc3339(), "2026-12-01T00:00:00+01:00");
    }

    #[test]
    fn a_secret_value_where_its_variable_name_belongs_fails_with_every_pointer_and_without_the_value() {
        let document = json!({
            "$schema": "https://example.test/TenantConfig.json",
            "personas": [{
                "name": "", "harness": "ClaudeCode", "secretSetName": "prod", "connectorNames": [], "files": [],
            }],
            "secretSets": [{
                "name": "prod",
                "secrets": [{ "kind": "ApiKey", "name": "ANTHROPIC_API_KEY", "provider": "Anthropic", "value": "sk-ant-secret", "valueFrom": { "env": "sk-ant-secret" } }],
            }],
            "harnessPins": [],
        });
        let errors = schema_errors(&document).expect("the schema compiles");
        assert_eq!(errors.len(), 3, "{errors:#?}");
        for pointer in ["/: ", "/personas/0/name: ", "/secretSets/0/secrets/0: "] {
            assert!(
                errors.iter().any(|error| error.starts_with(pointer)),
                "no error at {pointer} in {errors:#?}"
            );
        }
        assert!(errors.iter().any(|error| error.contains("hosts")), "{errors:#?}");
        assert!(
            errors.iter().all(|error| !error.contains("sk-ant-secret")),
            "{errors:#?}"
        );
    }
}
