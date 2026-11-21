use crate::resources::read_json;
use crate::sessions::parsed;
use crate::{print_json, shown, signed_in, LimitsCommand};
use anyhow::Result;
use tumpel_api::types::{TenantLimitsPreset, TenantLimitsProfile, TenantLimitsSet};

pub async fn limits(command: LimitsCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let url = format!("{}/v1/limits", credentials.endpoint);
    match command {
        LimitsCommand::Get => print_json(&shown(client.limits_get().await, || url)?),
        LimitsCommand::Set { file, tier } => {
            let body = match (file, tier) {
                (Some(file), _) => {
                    TenantLimitsSet::Profile(read_json::<TenantLimitsProfile>(&file, "TenantLimitsProfile")?)
                }
                (None, tier) => TenantLimitsSet::Preset(TenantLimitsPreset {
                    tier: parsed("--tier", &tier.unwrap_or_default())?,
                }),
            };
            print_json(&shown(client.limits_set(&body).await, || url)?)
        }
    }
}
