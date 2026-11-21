use crate::resources::read_json;
use crate::{print_json, shown, signed_in, LimitsCommand};
use anyhow::Result;
use tumpel_api::types::{TenantLimitsProfile, TenantLimitsReset, TenantLimitsSet};

pub async fn limits(command: LimitsCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let url = format!("{}/v1/limits", credentials.endpoint);
    let body = match command {
        LimitsCommand::Get => return print_json(&shown(client.limits_get().await, || url)?),
        LimitsCommand::Set { file } => {
            TenantLimitsSet::Profile(read_json::<TenantLimitsProfile>(&file, "TenantLimitsProfile")?)
        }
        LimitsCommand::Reset => TenantLimitsSet::Reset(TenantLimitsReset { reset: true }),
    };
    print_json(&shown(client.limits_set(&body).await, || url)?)
}
