use crate::sessions::{id, parsed};
use crate::{answered, print_json, shown, signed_in, InvitationCommand, MemberCommand};
use anyhow::Result;
use tumpel_api::types::InvitationCreate;

pub async fn tenants() -> Result<()> {
    let (credentials, client) = signed_in()?;
    print_json(&shown(client.identity_list_tenants().await, || {
        format!("{}/v1/whoami/tenants", credentials.endpoint)
    })?)
}

pub async fn member(command: MemberCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let url = format!("{}/v1/members", credentials.endpoint);
    match command {
        MemberCommand::List => print_json(&shown(client.members_list().await, || url)?),
        MemberCommand::Remove { principal_id } => {
            let principal = id("principal_id", &principal_id)?;
            answered(client.members_remove(&principal).await, || {
                format!("{url}/{principal_id}")
            })?;
            eprintln!("removed {principal_id} and signed out its devices");
            Ok(())
        }
    }
}

pub async fn invitation(command: InvitationCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let url = format!("{}/v1/invitations", credentials.endpoint);
    match command {
        InvitationCommand::Create { email, role } => {
            let body = InvitationCreate {
                email: parsed("email", &email)?,
                role: parsed("--role", &role)?,
            };
            let created = answered(client.invitations_create(&body).await, || url)?;
            println!("{}", created.accept_url);
            eprintln!(
                "No email is sent. Give that link to {email}; it lets them join as {} once, until {}, signed in to Cloudflare Access as that email. It is shown only now.",
                created.invitation.role,
                created.invitation.expires_at.to_rfc3339()
            );
            Ok(())
        }
        InvitationCommand::List => print_json(&shown(client.invitations_list().await, || url)?),
        InvitationCommand::Revoke { invitation_id } => {
            let invitation = id("invitation_id", &invitation_id)?;
            answered(client.invitations_revoke(&invitation).await, || {
                format!("{url}/{invitation_id}")
            })?;
            eprintln!("revoked {invitation_id}");
            Ok(())
        }
    }
}
