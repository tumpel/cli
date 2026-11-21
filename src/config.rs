use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use tumpel_api::types::{CliCredentials, CliProofCredentials};

const ENDPOINT_KEY: &str = "TUMPEL_ENDPOINT";
const TOKEN_KEY: &str = "TUMPEL_TOKEN";

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_default()
}

pub fn env_file() -> PathBuf {
    std::env::var_os("TUMPEL_ENV_FILE").map_or_else(|| config_home().join("tumpel").join("env"), PathBuf::from)
}

fn env_key(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (key, _) = line.strip_prefix("export ").unwrap_or(line).split_once('=')?;
    Some(key.trim())
}

fn parse_env(text: &str) -> HashMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.strip_prefix("export ").unwrap_or(line).split_once('='))
        .map(|(key, value)| {
            let value = value.trim();
            let unquoted = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(value);
            (key.trim().to_string(), unquoted.to_string())
        })
        .collect()
}

fn read_file(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

fn lookup(file: &HashMap<String, String>, key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| file.get(key).filter(|value| !value.is_empty()).cloned())
}

pub fn endpoint() -> Result<Option<String>> {
    let file = parse_env(&read_file(&env_file())?);
    Ok(lookup(&file, ENDPOINT_KEY).map(|value| value.trim_end_matches('/').to_string()))
}

pub fn load_credentials() -> Result<CliCredentials> {
    let path = env_file();
    let file = parse_env(&read_file(&path)?);
    let Some(endpoint) = lookup(&file, ENDPOINT_KEY) else {
        bail!(
            "{ENDPOINT_KEY} is not set in {} or the environment; run tumpel login --endpoint https://<your tumpel host>",
            path.display()
        );
    };
    let Some(token) = lookup(&file, TOKEN_KEY) else {
        bail!("this terminal is not signed in to tumpel; run tumpel login");
    };
    let token = token.try_into().map_err(|error| {
        anyhow::anyhow!(
            "{TOKEN_KEY} in {} or the environment is malformed ({error}); run tumpel login",
            path.display()
        )
    })?;
    Ok(CliCredentials {
        endpoint: endpoint
            .trim_end_matches('/')
            .parse()
            .map_err(|error| anyhow::anyhow!("{ENDPOINT_KEY} {endpoint:?} is not a valid URL: {error}"))?,
        token,
    })
}

fn keep_lines(text: &str, drop: &[&str]) -> String {
    text.lines()
        .filter(|line| !env_key(line).is_some_and(|key| drop.contains(&key)))
        .fold(String::new(), |mut kept, line| {
            kept.push_str(line);
            kept.push('\n');
            kept
        })
}

fn write_private(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let staged = path.with_extension("tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&staged)
        .with_context(|| format!("writing {}", staged.display()))?;
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600))?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&staged, path).with_context(|| format!("writing {}", path.display()))
}

pub fn save_login(credentials: &CliCredentials) -> Result<PathBuf> {
    let path = env_file();
    let kept = keep_lines(&read_file(&path)?, &[ENDPOINT_KEY, TOKEN_KEY]);
    write_private(
        &path,
        &format!(
            "{kept}{ENDPOINT_KEY}={}\n{TOKEN_KEY}={}\n",
            credentials.endpoint, *credentials.token
        ),
    )?;
    Ok(path)
}

const PROOF_CLIENT_ID_KEY: &str = "TUMPEL_PROOF_CLIENT_ID";
const PROOF_CLIENT_SECRET_KEY: &str = "TUMPEL_PROOF_CLIENT_SECRET";

pub fn proof_credentials() -> Result<Option<CliProofCredentials>> {
    let read = |key: &str| std::env::var(key).ok().filter(|value| !value.is_empty());
    match (read(PROOF_CLIENT_ID_KEY), read(PROOF_CLIENT_SECRET_KEY)) {
        (None, None) => Ok(None),
        (Some(client_id), Some(client_secret)) => Ok(Some(CliProofCredentials {
            client_id: client_id
                .try_into()
                .map_err(|_| anyhow::anyhow!("{PROOF_CLIENT_ID_KEY} is malformed; set it from the tofu output"))?,
            client_secret: client_secret
                .try_into()
                .map_err(|_| anyhow::anyhow!("{PROOF_CLIENT_SECRET_KEY} is malformed; set it from the tofu output"))?,
        })),
        _ => bail!("set both {PROOF_CLIENT_ID_KEY} and {PROOF_CLIENT_SECRET_KEY}, or neither"),
    }
}
