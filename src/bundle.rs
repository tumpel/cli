use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ring::digest::{digest as hash, SHA256};
use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use tumpel_api::types::{
    ConfigFileDigest, ConfigFileWrite, InlineConfigFile, InlineConfigFileSource, InlinePersonaFile,
    InlinePersonaFileSource, LocalFilePath, PersonaFileContent, PersonaFileLimit, PersonaFilePath, PersonaFileSize,
    PersonaFileUploadRequest, PersonaFileWrite, Sha256Digest, StoredConfigFile, StoredConfigFileSource,
    StoredPersonaFile, StoredPersonaFileSource, TenantConfig,
};
use tumpel_api::Client;

pub struct LocalFile {
    pub from: LocalFilePath,
    pub digest: String,
    pub bytes: Vec<u8>,
}

impl LocalFile {
    fn size(&self) -> i64 {
        i64::try_from(self.bytes.len()).unwrap_or(i64::MAX)
    }
}

pub fn sha256(bytes: &[u8]) -> String {
    hash(&SHA256, bytes)
        .as_ref()
        .iter()
        .fold(String::with_capacity(64), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}

fn read_file(base: &Path, from: &LocalFilePath) -> Result<LocalFile> {
    let local: PathBuf = base.join(from.as_str());
    let size = std::fs::metadata(&local)
        .with_context(|| format!("reading {}, a persona file the config names", local.display()))?
        .len();
    if i64::try_from(size).unwrap_or(i64::MAX) > PersonaFileLimit::PERSONA_BYTES {
        bail!(
            "{} is {size} bytes, over the {}-byte cap on a persona's files; trim it",
            local.display(),
            PersonaFileLimit::PERSONA_BYTES
        );
    }
    let bytes = std::fs::read(&local).with_context(|| format!("reading {}", local.display()))?;
    Ok(LocalFile {
        from: from.clone(),
        digest: sha256(&bytes),
        bytes,
    })
}

pub fn read(config_file: &Path, config: &TenantConfig) -> Result<Vec<LocalFile>> {
    let base = config_file.parent().unwrap_or_else(|| Path::new("."));
    let froms: BTreeMap<&str, &LocalFilePath> = config
        .personas
        .iter()
        .flat_map(|persona| persona.files.iter())
        .map(|file| (file.from.as_str(), &file.from))
        .collect();
    froms.values().map(|from| read_file(base, from)).collect()
}

pub fn digest(file: &LocalFile) -> Result<ConfigFileDigest> {
    Ok(ConfigFileDigest {
        digest: file
            .digest
            .clone()
            .try_into()
            .map_err(|error| anyhow!("{} has no usable digest: {error}", *file.from))?,
        from: file.from.clone(),
        size_bytes: PersonaFileSize(file.size()),
    })
}

pub async fn upload(client: &Client, bytes: &[u8], rerun: &str) -> Result<(Sha256Digest, PersonaFileSize)> {
    let digest = sha256(bytes);
    let request = PersonaFileUploadRequest {
        digest: digest
            .clone()
            .try_into()
            .map_err(|error| anyhow!("the digest {digest} is not usable: {error}"))?,
        size_bytes: PersonaFileSize(i64::try_from(bytes.len()).unwrap_or(i64::MAX)),
    };
    let presigned = match client.personas_upload_file(&request).await {
        Ok(answer) => answer.into_inner(),
        Err(tumpel_api::Error::ErrorResponse(problem)) => {
            bail!("{}: {}. {}", problem.title, problem.detail, problem.fix)
        }
        Err(error) => return Err(error).context("asking for a presigned persona file upload"),
    };
    let response = reqwest::Client::new()
        .put(&presigned.upload_url)
        .body(bytes.to_vec())
        .send()
        .await
        .context("uploading a persona file to its presigned URL")?;
    let status = response.status();
    if !status.is_success() {
        bail!("the store refused the persona file upload with {status}; run {rerun} again for a fresh URL");
    }
    Ok((request.digest, request.size_bytes))
}

fn stored(file: &LocalFile) -> Result<ConfigFileWrite> {
    let stored = digest(file)?;
    Ok(StoredConfigFile {
        digest: stored.digest,
        from: stored.from,
        size_bytes: stored.size_bytes,
        source: StoredConfigFileSource::Stored,
    }
    .into())
}

enum Placement {
    Inline(PersonaFileContent),
    Stored(Sha256Digest, PersonaFileSize),
}

async fn place(client: &Client, bytes: &[u8], what: &str, rerun: &str) -> Result<Placement> {
    if i64::try_from(bytes.len()).unwrap_or(i64::MAX) <= PersonaFileLimit::INLINE_FILE_BYTES {
        return Ok(Placement::Inline(
            STANDARD
                .encode(bytes)
                .try_into()
                .map_err(|error| anyhow!("{what} does not fit inline: {error}"))?,
        ));
    }
    let (digest, size_bytes) = upload(client, bytes, rerun).await?;
    Ok(Placement::Stored(digest, size_bytes))
}

pub async fn write(client: &Client, file: &LocalFile, held: bool) -> Result<ConfigFileWrite> {
    if held {
        return stored(file);
    }
    Ok(
        match place(client, &file.bytes, &file.from, "tumpel config apply").await? {
            Placement::Inline(content) => InlineConfigFile {
                content,
                from: file.from.clone(),
                source: InlineConfigFileSource::Inline,
            }
            .into(),
            Placement::Stored(digest, size_bytes) => StoredConfigFile {
                digest,
                from: file.from.clone(),
                size_bytes,
                source: StoredConfigFileSource::Stored,
            }
            .into(),
        },
    )
}

pub async fn persona_file(
    client: &Client,
    path: PersonaFilePath,
    local: &Path,
    rerun: &str,
) -> Result<PersonaFileWrite> {
    let bytes =
        std::fs::read(local).with_context(|| format!("reading {}, the persona file for {}", local.display(), *path))?;
    Ok(
        match place(client, &bytes, &local.display().to_string(), rerun).await? {
            Placement::Inline(content) => InlinePersonaFile {
                content,
                path,
                source: InlinePersonaFileSource::Inline,
            }
            .into(),
            Placement::Stored(digest, size_bytes) => StoredPersonaFile {
                digest,
                path,
                size_bytes,
                source: StoredPersonaFileSource::Stored,
            }
            .into(),
        },
    )
}
