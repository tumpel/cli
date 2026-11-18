mod bundle;
mod circleci_authorize;
mod claude_setups;
mod config;
mod connections;
mod connectors;
mod members;
mod metrics;
mod openrouter;
mod prebuilds;
mod resources;
mod sessions;
mod tenant_config;

use anyhow::{bail, Context, Result};
use clap::{ArgGroup, Args, Parser, Subcommand};
use serde::Serialize;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};
use tumpel_api::types::{
    CliCredentials, CliProofCredentials, DeviceApproval, DeviceCodeRequest, DeviceTokenRequest,
    DeviceTokenRequestGrantType, EnrolmentTokenMint, OAuthErrorCode, Problem,
};
use tumpel_api::{Client, Error, ResponseValue};

#[derive(Parser)]
#[command(name = "tumpel", version = env!("TUMPEL_RELEASE_VERSION"), about = "Harnesses on any hardware, driven over MCP")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Sign this terminal in through a browser on any device.
    Login {
        /// The control plane to sign in to; defaults to `TUMPEL_ENDPOINT` from the environment or the env file.
        #[arg(long)]
        endpoint: Option<String>,
        /// The name this device is listed under; defaults to the host name.
        #[arg(long)]
        name: Option<String>,
    },
    /// Show the tenant and principal this terminal is signed in as.
    Whoami,
    /// List every tenant you belong to and your role there; log in again to act in another.
    Tenants,
    /// List the people of the tenant, or remove one (Owner only).
    Member {
        #[command(subcommand)]
        command: MemberCommand,
    },
    /// Invite an email to the tenant, list open invitations or revoke one (Owner only).
    Invitation {
        #[command(subcommand)]
        command: InvitationCommand,
    },
    /// List or revoke the devices signed in as you.
    Devices {
        #[command(subcommand)]
        command: DevicesCommand,
    },
    /// Add, list or revoke the machines that run your boxes.
    Host {
        #[command(subcommand)]
        command: HostCommand,
    },
    /// List the harness catalog and pin a harness to one of its versions.
    Harness {
        #[command(subcommand)]
        command: HarnessCommand,
    },
    /// Read the current release: the supervisor and guest images hosts run.
    Release {
        #[command(subcommand)]
        command: ReleaseCommand,
    },
    /// Create, list, read, update or delete the tenant's personas.
    Persona {
        #[command(subcommand)]
        command: PersonaCommand,
    },
    /// Create, list, read, update, delete, connect or disconnect the remote MCP servers personas reach (Owner only to change).
    Connector {
        #[command(subcommand)]
        command: ConnectorCommand,
    },
    /// Create, list, read, update or delete secret sets; values come from environment variables, never arguments.
    SecretSet {
        #[command(subcommand)]
        command: SecretSetCommand,
    },
    /// Connect GitHub accounts or organizations through the stage's GitHub App, or list and remove them.
    Github {
        #[command(subcommand)]
        command: GithubCommand,
    },
    /// Keep the tenant's personas, secret sets, harness pins and host labels in one file.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Start, get, list, stop or archive sessions, and print what their turns wrote.
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
    /// Send, get or list a session's turns, and read why one failed; tumpel transcript read prints what a turn wrote.
    Turn {
        #[command(subcommand)]
        command: TurnCommand,
    },
    /// Read what the harness wrote in a turn.
    Transcript {
        #[command(subcommand)]
        command: TranscriptCommand,
    },
    /// List or get a session's snapshots.
    Snapshot {
        #[command(subcommand)]
        command: SnapshotCommand,
    },
    /// List the tenant's metrics, chart one over time, or list the turns behind it.
    Metrics {
        #[command(subcommand)]
        command: MetricsCommand,
    },
    /// List the OAuth clients, such as an MCP connector or the dashboard, signed in as you.
    OauthGrant {
        #[command(subcommand)]
        command: OauthGrantCommand,
    },
    /// List every connection with its state and the account it bills.
    Connection {
        #[command(subcommand)]
        command: ConnectionCommand,
    },
    /// Connect your GitHub Actions account as a build provider, or read or remove it.
    GithubActions {
        #[command(subcommand)]
        command: GithubActionsCommand,
    },
    /// Connect your CircleCI organization as a build provider, bind repositories to its projects, or read or remove them.
    Circleci {
        #[command(subcommand)]
        command: CircleciCommand,
    },
    /// Connect your Namespace workspace as a build provider, or read or remove it.
    Namespace {
        #[command(subcommand)]
        command: NamespaceCommand,
    },
    /// Start or read the prebuilds that install a repository's dependencies ahead of its sessions.
    Prebuild {
        #[command(subcommand)]
        command: PrebuildCommand,
    },
    /// List and read the devcontainer.json templates, and open a pull request that adds one to a repository.
    PrebuildTemplate {
        #[command(subcommand)]
        command: PrebuildTemplateCommand,
    },
    /// Mint a Claude subscription token into a secret set from a short-lived box, without installing Claude Code.
    ClaudeSetup {
        #[command(subcommand)]
        command: ClaudeSetupCommand,
    },
    /// Mint an OpenRouter API key into a secret set through OpenRouter's approval page, without copying a key.
    Openrouter {
        #[command(subcommand)]
        command: OpenrouterCommand,
    },
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Check a file against the tenant config schema and print every error with its JSON pointer.
    Validate {
        /// The tenant config file, such as tumpel.json.
        file: PathBuf,
    },
    /// Print what apply would create, update and delete; the tenant stays as it is.
    Plan {
        /// The tenant config file, such as tumpel.json.
        file: PathBuf,
    },
    /// Replace the tenant's config with the file, all of it or nothing: whatever the file leaves out is deleted. Export every variable the secret sets name to rewrite them all, or none to keep their stored values.
    Apply {
        /// The tenant config file, such as tumpel.json.
        file: PathBuf,
    },
    /// Write a new file holding the tenant's current configuration.
    Init {
        /// Where to write it; the file must not exist yet.
        file: PathBuf,
    },
}

#[derive(Subcommand)]
enum MemberCommand {
    /// List the people of the tenant with their roles and join times.
    List,
    /// Remove a person from the tenant and sign out their devices there; the last Owner stays.
    Remove {
        /// The `principalId` that tumpel member list prints.
        principal_id: String,
    },
}

#[derive(Subcommand)]
enum InvitationCommand {
    /// Invite an email for seven days; prints the accept link once, since no email is sent.
    Create {
        /// The email the invitee signs in to Cloudflare Access with.
        email: String,
        /// The role they join as: Owner or Member.
        #[arg(long, default_value = "Member")]
        role: String,
    },
    /// List the tenant's open invitations.
    List,
    /// Revoke an open invitation; its link stops working.
    Revoke {
        /// The `invitationId` that tumpel invitation list prints.
        invitation_id: String,
    },
}

#[derive(Subcommand)]
enum HostCommand {
    /// Mint a one-time enrolment token; it is printed once, to stdout only, and never again.
    MintToken {
        /// How long the token enrols a host, in seconds (60 to 86400).
        #[arg(long, default_value_t = 3600)]
        expires_in: i32,
    },
    /// List the unused, unexpired enrolment tokens, without their token values.
    ListTokens,
    /// Revoke an unused enrolment token; no host can enrol with it.
    RevokeToken {
        /// The `enrolmentTokenId` that tumpel host list-tokens prints.
        enrolment_token_id: String,
    },
    /// List your hosts and whether each is online.
    List,
    /// List the box slots, running and warm boxes, load and available memory of each unrevoked host that has connected.
    ListUsage,
    /// Show one host with its presence and labels.
    Get {
        /// The `hostId` that tumpel host list prints.
        host_id: String,
    },
    /// Replace every label of a host; pass none to clear them.
    SetLabels {
        /// The `hostId` that tumpel host list prints.
        host_id: String,
        /// The labels, such as gpu or region-eu.
        labels: Vec<String>,
    },
    /// Revoke a host: its channel closes and it cannot connect again.
    Revoke {
        /// The `hostId` that tumpel host list prints.
        host_id: String,
    },
}

#[derive(Subcommand)]
enum ReleaseCommand {
    /// Show the current release: its version, commit, channel protocol version, supervisors, CLIs and guest images.
    Current,
}

#[derive(Subcommand)]
enum HarnessCommand {
    /// List every harness and version the catalog offers.
    List,
    /// List the tenant's harness pins.
    ListPins,
    /// Pin a harness to a catalog version, replacing its pin.
    Pin {
        /// The harness, such as `ClaudeCode`.
        harness: String,
        /// A version tumpel harness list prints for it.
        version: String,
    },
    /// Remove the pin of a harness.
    Unpin {
        /// The harness, such as `ClaudeCode`.
        harness: String,
    },
}

#[derive(Subcommand)]
enum PersonaCommand {
    /// Create a persona from a `PersonaCreate` JSON file, or from --name, --harness, --secret-set-id and --persona-file.
    #[command(group(ArgGroup::new("source").required(true).args(["file", "name"])))]
    Create {
        /// The `PersonaCreate` JSON file.
        #[arg(long, conflicts_with = "PersonaFields")]
        file: Option<PathBuf>,
        #[command(flatten)]
        fields: Option<PersonaFields>,
    },
    /// List the tenant's personas.
    List,
    /// Show one persona.
    Get {
        /// The `personaId` that tumpel persona list prints.
        persona_id: String,
    },
    /// Replace a persona with a `PersonaUpdate` JSON file, or change the parts its flags name; --persona-file replaces the whole file set.
    #[command(group(ArgGroup::new("change").required(true).multiple(true).args(["file", "name", "secret_set_id", "persona_file", "connector", "no_connectors", "box_memory_mib", "box_disk_mib"])))]
    Update {
        /// The `personaId` that tumpel persona list prints.
        persona_id: String,
        /// The `PersonaUpdate` JSON file.
        #[arg(long, conflicts_with_all = ["name", "secret_set_id", "persona_file", "connector", "no_connectors", "box_memory_mib", "box_disk_mib"])]
        file: Option<PathBuf>,
        /// The new persona name; left out, the name stays.
        #[arg(long)]
        name: Option<String>,
        /// The `secretSetId` of the secret set it runs on; left out, the set stays.
        #[arg(long)]
        secret_set_id: Option<String>,
        /// A persona file as `<path in the harness home>=<local file>`; repeat it for each file. Left out, the files stay.
        #[arg(long, value_name = "PATH=LOCAL")]
        persona_file: Vec<String>,
        /// The `connectorId` of a connector every session of the persona reaches; repeat it for each connector. It replaces the whole list; left out, the list stays.
        #[arg(long = "connector", value_name = "CONNECTOR_ID")]
        connector: Vec<String>,
        /// Give the persona no connectors.
        #[arg(long, conflicts_with = "connector")]
        no_connectors: bool,
        /// The memory in MiB of each box the persona's sessions boot, from 512 up to 65536; left out, the memory stays.
        #[arg(long)]
        box_memory_mib: Option<i32>,
        /// The disk in MiB of each box the persona's sessions boot, from 32768 up to 262144; left out, the disk stays.
        #[arg(long)]
        box_disk_mib: Option<i32>,
    },
    /// Delete a persona.
    Delete {
        /// The `personaId` that tumpel persona list prints.
        persona_id: String,
    },
    /// Upload a persona file larger than the inline cap and print the digest and size to name it by in a persona write.
    UploadFile {
        /// The local file to upload.
        file: PathBuf,
    },
}

#[derive(Subcommand)]
enum ConnectorCommand {
    /// List the tenant's connectors with their status.
    List,
    /// Show one connector.
    Get {
        /// The `connectorId` that tumpel connector list prints.
        connector_id: String,
    },
    /// Create a connector that sends one bearer token to a remote MCP server; set the token with set-token.
    CreateStatic {
        /// The connector name, unique in the tenant.
        #[arg(long)]
        name: String,
        /// The HTTPS address of the MCP server; it never changes once the connector exists.
        #[arg(long)]
        url: String,
    },
    /// Create a connector that signs in to a remote MCP server with OAuth; connect it with authorize.
    CreateOauth {
        /// The connector name, unique in the tenant.
        #[arg(long)]
        name: String,
        /// The HTTPS address of the MCP server; it never changes once the connector exists.
        #[arg(long)]
        url: String,
    },
    /// Rename a connector; its address never changes.
    Update {
        /// The `connectorId` that tumpel connector list prints.
        connector_id: String,
        /// The new connector name.
        #[arg(long)]
        name: String,
    },
    /// Delete a connector and its credential; it is refused while a persona lists it or a running turn uses it.
    Delete {
        /// The `connectorId` that tumpel connector list prints.
        connector_id: String,
    },
    /// Start the sign-in of an OAuth connector and print the link an Owner opens, the attempt id and when the attempt expires.
    Authorize {
        /// The `connectorId` of an OAuth connector.
        connector_id: String,
    },
    /// Set the bearer token of a static connector, read from stdin.
    SetToken {
        /// The `connectorId` of a static connector.
        connector_id: String,
        /// Read the token from stdin; it is never taken as an argument.
        #[arg(long)]
        stdin: bool,
    },
    /// Disconnect a connector: new turns stop using it, and turns already running keep it until they end.
    ClearCredential {
        /// The `connectorId` that tumpel connector list prints.
        connector_id: String,
    },
}

#[derive(Subcommand)]
enum SecretSetCommand {
    /// Create a secret set from a `ConfigSecretSet` JSON file, each value read from the variable its `valueFrom.env` names, or an empty one with --name.
    Create {
        /// The `ConfigSecretSet` JSON file.
        #[arg(long, required_unless_present = "name", conflicts_with = "name")]
        file: Option<PathBuf>,
        /// The name of a new, empty secret set; add its members with set-member.
        #[arg(long)]
        name: Option<String>,
    },
    /// List the secret sets with their member names and kinds; values are never shown.
    List,
    /// Show one secret set with its member names and kinds.
    Get {
        /// The `secretSetId` that tumpel secret-set list prints.
        secret_set_id: String,
    },
    /// Set a secret set's name and members from a `ConfigSecretSet` JSON file; a member the file leaves out is deleted, and a member whose variable is unset keeps its stored value.
    Update {
        /// The `secretSetId` that tumpel secret-set list prints.
        secret_set_id: String,
        /// The `ConfigSecretSet` JSON file.
        #[arg(long)]
        file: PathBuf,
    },
    /// Delete a secret set and its stored values; refused while a persona names it.
    Delete {
        /// The `secretSetId` that tumpel secret-set list prints.
        secret_set_id: String,
    },
    /// Write one member with the value piped on stdin, replacing a member of the same name; every other member keeps its value.
    SetMember {
        #[command(subcommand)]
        command: SecretMemberCommand,
    },
    /// Delete one member and its stored value; every other member keeps its value.
    RemoveMember {
        /// The `secretSetId` that tumpel secret-set list prints.
        secret_set_id: String,
        /// The member name tumpel secret-set get prints.
        name: String,
    },
}

#[derive(Args)]
struct PersonaFields {
    /// The persona name.
    #[arg(long)]
    name: String,
    /// The harness, such as `ClaudeCode`; fixed once the persona exists.
    #[arg(long)]
    harness: String,
    /// The `secretSetId` that tumpel secret-set list prints.
    #[arg(long)]
    secret_set_id: String,
    /// A persona file as `<path in the harness home>=<local file>`, such as `.claude/settings.json=./settings.json`; repeat it for each file.
    #[arg(long, value_name = "PATH=LOCAL")]
    persona_file: Vec<String>,
    /// The `connectorId` of a connector every session of the persona reaches; repeat it for each connector.
    #[arg(
        long = "connector",
        value_name = "CONNECTOR_ID",
        required_unless_present = "no_connectors"
    )]
    connector: Vec<String>,
    /// Give the persona no connectors.
    #[arg(long, conflicts_with = "connector")]
    no_connectors: bool,
    /// The memory in MiB of each box the persona's sessions boot, from 512 up to 65536; left out, boxes get 2048.
    #[arg(long)]
    box_memory_mib: Option<i32>,
    /// The disk in MiB of each box the persona's sessions boot, from 32768 up to 262144; left out, boxes get 32768.
    #[arg(long)]
    box_disk_mib: Option<i32>,
}

#[derive(Args)]
struct MemberTarget {
    /// The `secretSetId` that tumpel secret-set list prints.
    secret_set_id: String,
    /// The member name, such as `GITHUB_TOKEN` or `CLAUDE_CODE_OAUTH_TOKEN`.
    #[arg(long)]
    name: String,
    /// Read the value from stdin; it is never taken as an argument.
    #[arg(long)]
    stdin: bool,
}

#[derive(Subcommand)]
enum SecretMemberCommand {
    /// Write a plain environment variable.
    Variable {
        #[command(flatten)]
        target: MemberTarget,
    },
    /// Write a provider API key.
    ApiKey {
        #[command(flatten)]
        target: MemberTarget,
        /// The provider, such as `Anthropic`.
        #[arg(long)]
        provider: String,
    },
    /// Write a provider subscription token.
    SubscriptionToken {
        #[command(flatten)]
        target: MemberTarget,
        /// The provider, such as `Anthropic`.
        #[arg(long)]
        provider: String,
        /// When the token stops working, as an ISO 8601 date-time with an offset.
        #[arg(long)]
        expires_at: String,
    },
}

#[derive(Subcommand)]
enum SessionCommand {
    /// Start a session with its first prompt on an online host of the tenant.
    Start {
        /// The GitHub repository, as `https://github.com/owner/name`.
        #[arg(long)]
        repository: String,
        /// The model the harness uses.
        #[arg(long)]
        model: String,
        /// The personaId tumpel persona list prints; the session runs that persona's harness, files and secret set.
        #[arg(long)]
        persona_id: String,
        /// The first prompt.
        #[arg(long)]
        prompt: String,
    },
    /// Start a session from an open GitHub issue; the control plane writes the first prompt from it.
    StartFromIssue {
        /// The owner of the issue's repository, such as `acme`.
        #[arg(long)]
        owner: String,
        /// The name of the issue's repository, such as `app`.
        #[arg(long)]
        name: String,
        /// The number of an open issue of that repository.
        #[arg(long)]
        issue_number: String,
        /// The model the harness uses.
        #[arg(long)]
        model: String,
        /// The personaId tumpel persona list prints; the session runs that persona's harness, files and secret set.
        #[arg(long)]
        persona_id: String,
    },
    /// Show one session with the stops of it that failed, the prebuild layer its files start from or why they start without one, and the image its latest turn boots.
    Get {
        /// The sessionId tumpel session list prints.
        session_id: String,
    },
    /// List the stops of a session that failed, oldest first.
    StopFailures {
        /// The sessionId tumpel session list prints.
        session_id: String,
    },
    /// Stop a session: its host snapshots it and frees its box. Refused while a turn runs, unless --cancel-turn is given.
    Stop {
        /// The sessionId tumpel session list prints.
        session_id: String,
        /// End the running turn at once: it fails Cancelled, and the host snapshots and frees the box.
        #[arg(long)]
        cancel_turn: bool,
    },
    /// Archive idle, stopped or failed sessions: discard an idle session's box, delete its snapshots, keep its transcript and refuse later sends.
    Archive {
        /// The sessionIds tumpel session list prints; give several to archive each of them.
        #[arg(required_unless_present_any = ["state", "last_turn_before"], conflicts_with_all = ["state", "last_turn_before"])]
        session_ids: Vec<String>,
        /// Archive every session in this state: Idle, Stopped or Failed.
        #[arg(long)]
        state: Option<String>,
        /// Archive every session whose latest turn last changed state at or before this time, such as 2026-11-17T12:00:00Z.
        #[arg(long)]
        last_turn_before: Option<String>,
    },
    /// List the credential handovers of a session, oldest first: variable names, never values.
    Handovers {
        /// The sessionId tumpel session list prints.
        session_id: String,
    },
    /// Print what the harness wrote in every turn of a session, oldest first, one JSONL line per line.
    Transcript {
        /// The sessionId tumpel session list prints.
        session_id: String,
        #[command(flatten)]
        read: TranscriptArgs,
    },
    /// List your sessions.
    List {
        /// Only sessions in this state: Starting, Idle, Running, Stopping, Stopped or Archived.
        #[arg(long)]
        state: Option<String>,
        /// Only sessions on this host.
        #[arg(long)]
        host_id: Option<String>,
        /// Only sessions whose latest turn last changed state at or before this time, such as 2026-11-17T12:00:00Z.
        #[arg(long)]
        last_turn_before: Option<String>,
    },
}

#[derive(Subcommand)]
enum TurnCommand {
    /// Send a session its next prompt; refused while another turn of it runs.
    Send {
        /// The sessionId tumpel session list prints.
        session_id: String,
        /// The prompt.
        #[arg(long)]
        prompt: String,
        /// When the work is done: the harness keeps working until this holds. See tumpel harness list for which harnesses take a goal.
        #[arg(long)]
        goal: Option<String>,
    },
    /// Show one turn with the image its box boots.
    Get {
        /// The turnId tumpel turn list prints.
        turn_id: String,
    },
    /// List a session's turns, oldest first.
    List {
        /// The sessionId tumpel session list prints.
        session_id: String,
    },
    /// Show why a Failed turn failed.
    Failure {
        /// The turnId tumpel turn list prints.
        turn_id: String,
    },
    /// Wait until the first of up to 50 turns of any of your sessions is Completed or Failed, then show every turn; a Failed turn shows why it failed.
    Wait {
        /// The turnIds tumpel turn list prints.
        #[arg(required = true)]
        turn_ids: Vec<String>,
        /// The most seconds to wait, from 0 to 50; every turn shows once they pass, even when none ended.
        #[arg(long)]
        timeout_seconds: i32,
    },
}

#[derive(Subcommand)]
enum TranscriptCommand {
    /// Print what the harness wrote in a turn, one JSONL line per line.
    Read {
        /// The turnId tumpel turn list prints.
        turn_id: String,
        #[command(flatten)]
        read: TranscriptArgs,
    },
}

#[derive(Args)]
struct TranscriptArgs {
    /// Keep reading until the transcript reads finished, polling with backoff.
    #[arg(long)]
    follow: bool,
    /// Start after this lineSeq, the cursor of an earlier read.
    #[arg(long, default_value_t = 0)]
    after: i64,
    /// The most lines to print in total; reads page by page until it has them.
    #[arg(long)]
    limit: Option<NonZeroU32>,
    /// The most bytes one read returns, cut on a line boundary; 32768 when left out.
    #[arg(long)]
    max_bytes: Option<NonZeroU32>,
    /// Print the assistant's text and one line per tool call, and token, cost or error reports, without tool output; only a harness that tumpel harness list shows with compactTranscript has a compact form.
    #[arg(long)]
    compact: bool,
}

#[derive(Subcommand)]
enum SnapshotCommand {
    /// List a session's snapshots in order, each stored or failed with its reason.
    List {
        /// The sessionId tumpel session list prints.
        session_id: String,
    },
    /// Show one snapshot: its stored size, or why it was not stored.
    Get {
        /// The snapshotId tumpel snapshot list prints.
        snapshot_id: String,
    },
}

#[derive(Args)]
struct MetricWindowArgs {
    /// The start of the window, ISO 8601 with an offset, such as 2026-11-16T00:00:00+02:00; Day buckets start at midnight in its offset.
    #[arg(long)]
    from: String,
    /// The end of the window, ISO 8601 with an offset, after --from.
    #[arg(long)]
    to: String,
    /// Keep series whose dimension has this value, as dimension:value; repeat one dimension for OR, different dimensions for AND.
    #[arg(long)]
    filter: Vec<String>,
}

#[derive(Subcommand)]
enum MetricsCommand {
    /// List every metric with series seen in the window, with its dimensions and their values.
    Catalog {
        #[command(flatten)]
        window: MetricWindowArgs,
    },
    /// Show a metric's buckets in the window: count, sum, min, max, p50, p90 and p99.
    Query {
        /// The metric name tumpel metrics catalog prints, such as turn.cost.
        metric: String,
        #[command(flatten)]
        window: MetricWindowArgs,
        /// The bucket size: Minute, Hour or Day.
        #[arg(long)]
        bucket: String,
        /// A dimension to group by; repeat it for a second.
        #[arg(long)]
        group_by: Vec<String>,
    },
    /// List the turns behind a Turn metric in the window, oldest first, a page at a time.
    Turns {
        /// The metric name tumpel metrics catalog prints, such as turn.cost.
        metric: String,
        #[command(flatten)]
        window: MetricWindowArgs,
        /// Keep the points of this sessionId alone.
        #[arg(long)]
        session_id: Option<String>,
        /// The cursor the previous page printed.
        #[arg(long)]
        cursor: Option<String>,
    },
}

#[derive(Subcommand)]
enum OauthGrantCommand {
    /// List the OAuth clients that hold a live token for you in this tenant.
    List,
    /// Revoke one of your OAuth grants; every token of the grant stops working at once.
    Revoke {
        /// The `grantId` that tumpel oauth-grant list prints.
        grant_id: String,
    },
}

#[derive(Subcommand)]
enum ConnectionCommand {
    /// List one row per connection kind with its state and, once connected, its account.
    List,
}

#[derive(Subcommand)]
enum GithubActionsCommand {
    /// Show the GitHub Actions connection.
    Get,
    /// Connect your GitHub Actions account, replacing an earlier connection.
    Start {
        /// The account GitHub Actions bills, as you name it.
        #[arg(long)]
        account_label: String,
        /// The caller workflow each repository holds, such as `.github/workflows/tumpel-prebuild.yml`.
        #[arg(long)]
        caller_workflow: String,
    },
    /// Disconnect the GitHub Actions build provider.
    Delete,
}

#[derive(Subcommand)]
enum OpenrouterCommand {
    /// Start an authorization for a secret set: prints it with the `authorizeUrl` to open; OpenRouter sends the browser back to the control plane, which stores the key as `OPENROUTER_API_KEY`.
    Authorize {
        /// The `secretSetId` that tumpel secret-set list prints.
        #[arg(long)]
        secret_set_id: String,
    },
    /// Show one OpenRouter authorization you started: `AwaitingCallback`, `Exchanging`, `Completed` or `Failed` with its reason; the key is never shown.
    GetAuthorization {
        /// The `openRouterAuthorizationId` that tumpel openrouter authorize printed.
        open_router_authorization_id: String,
    },
}

#[derive(Subcommand)]
enum CircleciCommand {
    /// Show the CircleCI connection, the orb and the config each repository commits; the token is never shown.
    Get,
    /// Connect your CircleCI organization through CircleCI's consent page: prints the address to open, listens on 127.0.0.1 for the redirect, and connects; replaces an earlier connection.
    Authorize {
        /// The organization to connect when the token reads several; asked on the terminal when left out.
        #[arg(long)]
        organization_id: Option<String>,
    },
    /// Finish an authorization with the address the browser landed on, piped on stdin, for a machine where tumpel circleci authorize cannot listen.
    Complete {
        /// The `circleCIAuthorizationId` that `tumpel_circleci_authorize` answered.
        #[arg(long = "circleci-authorization-id")]
        circle_ci_authorization_id: String,
        /// Read the redirect address from stdin; it carries a one-time code and is never an argument.
        #[arg(long)]
        stdin: bool,
    },
    /// Connect one of the organizations a `ChoosingOrganization` authorization lists.
    Choose {
        /// The `circleCIAuthorizationId` of the authorization.
        #[arg(long = "circleci-authorization-id")]
        circle_ci_authorization_id: String,
        /// One organizationId the authorization lists.
        #[arg(long)]
        organization_id: String,
    },
    /// Connect your CircleCI organization with a personal API token piped on stdin, the fallback to authorize; replaces an earlier connection.
    Start {
        /// The CircleCI organization that bills the builds, as you name it.
        #[arg(long)]
        account_label: String,
        /// The organization id from CircleCI's Organization Settings > Overview.
        #[arg(long)]
        organization_id: String,
        /// Read the personal API token from stdin; it is never taken as an argument.
        #[arg(long)]
        token_stdin: bool,
    },
    /// Disconnect the CircleCI build provider and delete its project bindings.
    Delete,
    /// Bind repositories to their CircleCI project, or list or remove the bindings.
    Project {
        #[command(subcommand)]
        command: CircleciProjectCommand,
    },
}

#[derive(Subcommand)]
enum CircleciProjectCommand {
    /// List the repositories bound to a CircleCI project.
    List,
    /// Bind a repository to its CircleCI project and pipeline definition, once.
    Add {
        /// The GitHub repository, as `https://github.com/owner/name`.
        #[arg(long)]
        repository: String,
        /// `gh/<owner>/<name>` in a GitHub OAuth organization, `circleci/<org id>/<project id>` in a GitHub App organization.
        #[arg(long)]
        project_slug: String,
        /// The pipeline definition that checks out the repository, from Project Settings > Project Setup.
        #[arg(long)]
        pipeline_definition_id: String,
    },
    /// Delete a repository's CircleCI project binding.
    Delete {
        /// The circleCIProjectId tumpel circleci project list prints.
        circle_ci_project_id: String,
    },
}

#[derive(Subcommand)]
enum NamespaceCommand {
    /// Show the issuer, subject pattern and audience Namespace trusts.
    Get,
    /// Connect your Namespace workspace, replacing an earlier connection.
    Start {
        /// The Namespace workspace that bills the builds, as you name it.
        #[arg(long)]
        account_label: String,
        /// The workspace id, such as `tenant_123456789ab`, that the builds run in.
        #[arg(long)]
        workspace_id: String,
    },
    /// Disconnect the Namespace build provider.
    Delete,
}

#[derive(Subcommand)]
enum ClaudeSetupCommand {
    /// Start a box that runs claude setup-token; read it with get until it shows the sign-in URL.
    Start {
        /// The `secretSetId` tumpel secret-set list prints; the token lands there as `CLAUDE_CODE_OAUTH_TOKEN`.
        secret_set_id: String,
    },
    /// Show the running Claude setup of one secret set, if any, to resume it.
    List {
        /// The `secretSetId` tumpel secret-set list prints.
        secret_set_id: String,
    },
    /// Show one Claude setup: its sign-in URL while it waits for the code, then the member it wrote or why it failed.
    Get {
        /// The `claudeSetupId` tumpel claude-setup start printed.
        claude_setup_id: String,
    },
    /// Send the code the sign-in page showed, read from stdin, to the setup's box once.
    SubmitCode {
        /// The `claudeSetupId` tumpel claude-setup start printed.
        claude_setup_id: String,
        /// Read the code from stdin; the code is a secret and is never taken as an argument.
        #[arg(long)]
        stdin: bool,
    },
}

#[derive(Subcommand)]
enum PrebuildCommand {
    /// Start a prebuild of a repository's default branch; a stored or running one with the same key comes back.
    Start {
        /// The GitHub repository, as `https://github.com/owner/name`.
        #[arg(long)]
        repository: String,
        /// The `personaId` whose secret set's `GITHUB_TOKEN` reads the repository when the GitHub App does not cover it.
        #[arg(long)]
        persona_id: String,
        /// The connected build provider that runs the job: `GitHubActions`, `CircleCI` or `Namespace`.
        #[arg(long)]
        provider: String,
        /// Start a job even when a prebuild with the same key is stored or running.
        #[arg(long)]
        rebuild: bool,
    },
    /// List the prebuilds of one repository, or of every repository when left out, newest first.
    List {
        /// The GitHub repository, as `https://github.com/owner/name`.
        #[arg(long)]
        repository: Option<String>,
    },
    /// List the newest prebuild of each repository you have prebuilt, ordered by repository.
    ListLatest,
    /// Show one prebuild.
    Get {
        /// The prebuildId tumpel prebuild list prints.
        prebuild_id: String,
    },
    /// Show why a Failed prebuild failed: the phase and its fix.
    Failure {
        /// The prebuildId tumpel prebuild list prints.
        prebuild_id: String,
    },
    /// Show what a prebuild would run: the commands in order, containerEnv, the input digest and what a build would refuse.
    Preview {
        /// The GitHub repository, as `https://github.com/owner/name`.
        #[arg(long)]
        repository: String,
        /// The `personaId` whose secret set's `GITHUB_TOKEN` reads the repository when the GitHub App does not cover it.
        #[arg(long)]
        persona_id: String,
        /// A devcontainer.json to preview in place of the repository's own.
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum PrebuildTemplateCommand {
    /// List every template with its versions, newest first, and the toolchains each version pins.
    List,
    /// Show one template version with its devcontainer.json.
    Get {
        /// The template, such as `npm`, `rust` or `python`.
        #[arg(long)]
        template: String,
        /// The version tumpel prebuild-template list prints.
        #[arg(long)]
        version: String,
    },
    /// Open a pull request that adds a template version's devcontainer.json to a repository.
    OpenPullRequest {
        /// The GitHub repository, as `https://github.com/owner/name`.
        #[arg(long)]
        repository: String,
        /// The template, such as `npm`, `rust` or `python`.
        #[arg(long)]
        template: String,
        /// The version tumpel prebuild-template list prints.
        #[arg(long)]
        version: String,
        /// Replace the repository's devcontainer.json; without it a repository that holds one is refused.
        #[arg(long)]
        replace: bool,
    },
    /// Show the template pull request of a repository: Open, Merged or Closed.
    GetPullRequest {
        /// The GitHub repository, as `https://github.com/owner/name`.
        #[arg(long)]
        repository: String,
    },
}

#[derive(Subcommand)]
enum GithubCommand {
    /// Print the URL where a person installs the GitHub App on one more account or organization for your tenant.
    Start,
    /// Show whether the stage has a GitHub App.
    Get,
    /// List every GitHub account or organization your tenant connected, with the repositories each covers.
    List,
    /// Remove one installation from your tenant; uninstall the App on GitHub as well.
    Delete {
        /// The installationId tumpel github list prints.
        installation_id: String,
    },
    /// List the repositories every installation of your tenant covers.
    ListRepositories,
    /// List the open issues of a repository an installation covers, most recently updated first.
    ListIssues {
        /// The owner tumpel github list-repositories prints, such as `acme`.
        owner: String,
        /// The repository name tumpel github list-repositories prints, such as `app`.
        name: String,
    },
}

#[derive(Subcommand)]
enum DevicesCommand {
    /// List every device signed in as you.
    List,
    /// Sign a device out; its token stops working at once.
    Revoke {
        /// The deviceId that tumpel devices list prints.
        device_id: String,
    },
}

fn host_name() -> String {
    hostname::get()
        .ok()
        .and_then(|name| name.into_string().ok())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "tumpel cli".to_string())
}

fn problem_text(problem: &Problem) -> String {
    format!("{}: {}. {}", problem.title, problem.detail, problem.fix)
}

fn sensitive_header(value: &str, name: &str) -> Result<reqwest::header::HeaderValue> {
    let mut header = reqwest::header::HeaderValue::from_str(value)
        .map_err(|_| anyhow::anyhow!("{name} holds characters a header cannot carry; set it from the tofu output"))?;
    header.set_sensitive(true);
    Ok(header)
}

fn proof_tenant_id(client_id: &str) -> Result<tumpel_api::types::EntityId> {
    let hex = bundle::sha256(format!("tenant:{client_id}").as_bytes());
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
    .parse()
    .map_err(|error| anyhow::anyhow!("the proof tenant id is no entity id: {error}"))
}

async fn approve_with_service_token(endpoint: &str, proof: &CliProofCredentials, user_code: &str) -> Result<()> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "cf-access-client-id",
        sensitive_header(&proof.client_id, "TUMPEL_PROOF_CLIENT_ID")?,
    );
    headers.insert(
        "cf-access-client-secret",
        sensitive_header(&proof.client_secret, "TUMPEL_PROOF_CLIENT_SECRET")?,
    );
    let http = reqwest::Client::builder()
        .default_headers(headers)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("building the HTTP client")?;
    let decision = DeviceApproval {
        user_code: user_code.to_string(),
        tenant_id: proof_tenant_id(&proof.client_id)?,
    };
    answered(
        Client::new_with_client(endpoint, http)
            .device_flow_approve(&decision)
            .await,
        || format!("{endpoint}/v1/device/approve with the proof's service token"),
    )?;
    Ok(())
}

async fn login(endpoint: Option<String>, name: Option<String>) -> Result<()> {
    let proof = config::proof_credentials()?;
    let Some(endpoint) = endpoint
        .map(|value| value.trim_end_matches('/').to_string())
        .or(config::endpoint()?)
    else {
        bail!("pass --endpoint https://<your tumpel host>");
    };
    let client = Client::new(&endpoint).context("building the HTTP client")?;
    let device_name = name.unwrap_or_else(host_name);
    let request = DeviceCodeRequest {
        device_name: device_name
            .clone()
            .try_into()
            .map_err(|error| anyhow::anyhow!("--name {device_name:?} is not a device name: {error}"))?,
    };
    let started = match client.device_flow_start(&request).await {
        Ok(started) => started.into_inner(),
        Err(Error::ErrorResponse(refusal)) => {
            bail!("{endpoint} refused the sign-in: {}", refusal.error_description)
        }
        Err(error) => return Err(error).with_context(|| format!("starting the device login at {endpoint}")),
    };
    match &proof {
        Some(proof) => {
            approve_with_service_token(&endpoint, proof, &started.user_code).await?;
            eprintln!("approved the code {} with the proof's service token", started.user_code);
        }
        None => eprintln!(
            "Open {} and enter the code {}\n(or open {})",
            started.verification_uri, started.user_code, started.verification_uri_complete
        ),
    }
    let deadline = Instant::now() + Duration::from_secs(started.expires_in.get().into());
    let mut interval = Duration::from_secs(started.interval.get().into());
    let poll = DeviceTokenRequest {
        device_code: started.device_code.clone(),
        grant_type: DeviceTokenRequestGrantType::UrnIetfParamsOauthGrantTypeDeviceCode,
    };
    loop {
        tokio::time::sleep(interval).await;
        if Instant::now() >= deadline {
            bail!("the code expired before it was approved; run tumpel login again");
        }
        match client.device_flow_token(&poll).await {
            Ok(granted) => {
                let token = granted
                    .into_inner()
                    .access_token
                    .try_into()
                    .map_err(|error| anyhow::anyhow!("the control plane issued an empty token: {error}"))?;
                let path = config::save_login(&CliCredentials {
                    endpoint: endpoint.clone(),
                    token,
                })?;
                eprintln!("signed in; wrote {}", path.display());
                return Ok(());
            }
            Err(Error::ErrorResponse(refusal)) => match refusal.error {
                OAuthErrorCode::AuthorizationPending => {}
                OAuthErrorCode::SlowDown => interval += Duration::from_secs(5),
                OAuthErrorCode::AccessDenied => bail!("the sign-in was denied in the browser"),
                OAuthErrorCode::ExpiredToken => {
                    bail!("the code expired before it was approved; run tumpel login again")
                }
                _ => bail!("the device login failed: {}", refusal.error_description),
            },
            Err(error) => return Err(error).context("polling for the device token"),
        }
    }
}

fn signed_in() -> Result<(CliCredentials, Client)> {
    let credentials = config::load_credentials()?;
    let mut headers = reqwest::header::HeaderMap::new();
    let mut authorization = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", *credentials.token))
        .context("the stored token is not a valid header value; run tumpel login")?;
    authorization.set_sensitive(true);
    headers.insert(reqwest::header::AUTHORIZATION, authorization);
    let http = reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .context("building the HTTP client")?;
    let client = Client::new_with_client(&credentials.endpoint, http);
    Ok((credentials, client))
}

#[derive(Debug)]
struct UnexpectedAnswer(reqwest::Response);

impl std::fmt::Display for UnexpectedAnswer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "The Tumpel service at {} answered status {}",
            self.0.url().host_str().unwrap_or("your endpoint"),
            self.0.status().as_u16()
        )
    }
}

impl std::error::Error for UnexpectedAnswer {}

async fn explained(error: anyhow::Error) -> anyhow::Error {
    let UnexpectedAnswer(response) = match error.downcast::<UnexpectedAnswer>() {
        Ok(answer) => answer,
        Err(error) => return error,
    };
    let host = response.url().host_str().unwrap_or("your endpoint").to_string();
    let status = response.status().as_u16();
    if !response.status().is_client_error() {
        return anyhow::anyhow!(
            "The Tumpel service at {host} failed with status {status}; try again in a few minutes."
        );
    }
    let body = response.text().await.unwrap_or_default();
    match serde_json::from_str::<Problem>(&body) {
        Ok(problem) => anyhow::anyhow!("{}. {}", problem.title, problem.fix),
        Err(_) => anyhow::anyhow!(
            "The Tumpel service at {host} refused the request with status {status} and no problem document. Check the command's arguments; if they are right, report it at https://github.com/tumpel/cli/issues/new."
        ),
    }
}

fn answered<T>(outcome: Result<ResponseValue<T>, Error<Problem>>, call: impl FnOnce() -> String) -> Result<T> {
    match outcome {
        Ok(answer) => Ok(answer.into_inner()),
        Err(Error::ErrorResponse(problem)) => bail!("{}", problem_text(&problem)),
        Err(Error::UnexpectedResponse(response)) => Err(UnexpectedAnswer(response).into()),
        Err(Error::InvalidResponsePayload(_, error)) => bail!(
            "calling {} returned an answer this tumpel cannot read ({error}). The control plane runs an older API than this CLI; install the CLI it serves with: curl -fsSL <endpoint>/v1/releases/install.sh | sh",
            call()
        ),
        Err(error) => Err(error).with_context(|| format!("calling {}", call())),
    }
}

fn shown<T: Serialize>(
    outcome: Result<ResponseValue<T>, Error<Problem>>,
    call: impl FnOnce() -> String,
) -> Result<serde_json::Value> {
    match outcome {
        Err(Error::InvalidResponsePayload(body, _)) => {
            let value: serde_json::Value = serde_json::from_slice(&body)
                .with_context(|| format!("calling {} returned a body that is not JSON", call()))?;
            if let Some(problem) = problem_fields(&value) {
                bail!("{problem}");
            }
            Ok(value)
        }
        outcome => Ok(serde_json::to_value(answered(outcome, call)?)?),
    }
}

fn problem_fields(value: &serde_json::Value) -> Option<String> {
    let field = |name: &str| value.get(name).and_then(serde_json::Value::as_str);
    Some(format!("{}: {}. {}", field("title")?, field("detail")?, field("fix")?))
}

fn print_json(value: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

async fn whoami() -> Result<()> {
    let (credentials, client) = signed_in()?;
    let whoami = shown(client.identity_whoami().await, || {
        format!("{}/v1/whoami", credentials.endpoint)
    })?;
    print_json(&whoami)
}

async fn devices(command: DevicesCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    match command {
        DevicesCommand::List => print_json(&shown(client.devices_list().await, || {
            format!("{endpoint}/v1/devices")
        })?),
        DevicesCommand::Revoke { device_id } => {
            let id = device_id
                .clone()
                .try_into()
                .map_err(|error| anyhow::anyhow!("{device_id:?} is not a device id: {error}"))?;
            answered(client.devices_revoke(&id).await, || {
                format!("{endpoint}/v1/devices/{device_id}")
            })?;
            eprintln!("revoked {device_id}");
            Ok(())
        }
    }
}

async fn host(command: HostCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    match command {
        HostCommand::MintToken { expires_in } => {
            let body = EnrolmentTokenMint {
                expires_in_seconds: expires_in,
            };
            let minted = answered(client.enrolment_tokens_mint(&body).await, || {
                format!("{endpoint}/v1/enrolment-tokens")
            })?;
            println!("{}", minted.token);
            eprintln!(
                "The token on stdout enrols one host until {}, and is shown only now.\nOn the new x86_64 host, run this with the token on stdin; it downloads the supervisor, installs it and enrols the host:\n  {}\nFor example: tumpel host mint-token | ssh <host> '{}'",
                minted.enrolment_token.expires_at.to_rfc3339(),
                minted.install_command,
                minted.install_command
            );
            Ok(())
        }
        HostCommand::ListTokens => print_json(&shown(client.enrolment_tokens_list().await, || {
            format!("{endpoint}/v1/enrolment-tokens")
        })?),
        HostCommand::RevokeToken { enrolment_token_id } => {
            let id = sessions::id("enrolment_token_id", &enrolment_token_id)?;
            answered(client.enrolment_tokens_revoke(&id).await, || {
                format!("{endpoint}/v1/enrolment-tokens/{enrolment_token_id}")
            })?;
            eprintln!("revoked {enrolment_token_id}");
            Ok(())
        }
        HostCommand::List => print_json(&shown(client.hosts_list().await, || format!("{endpoint}/v1/hosts"))?),
        HostCommand::ListUsage => print_json(&shown(client.hosts_list_usage().await, || {
            format!("{endpoint}/v1/hosts/usage")
        })?),
        HostCommand::Get { host_id } => resources::host_get(host_id).await,
        HostCommand::SetLabels { host_id, labels } => resources::host_set_labels(host_id, labels).await,
        HostCommand::Revoke { host_id } => {
            let id = host_id
                .clone()
                .try_into()
                .map_err(|error| anyhow::anyhow!("{host_id:?} is not a host id: {error}"))?;
            print_json(&shown(client.hosts_revoke(&id).await, || {
                format!("{endpoint}/v1/hosts/{host_id}/revoke")
            })?)
        }
    }
}

async fn release(command: ReleaseCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    match command {
        ReleaseCommand::Current => print_json(&shown(client.releases_current().await, || {
            format!("{endpoint}/v1/releases/current")
        })?),
    }
}

async fn github(command: GithubCommand) -> Result<()> {
    let (credentials, client) = signed_in()?;
    let endpoint = credentials.endpoint;
    match command {
        GithubCommand::Start => {
            let started = answered(client.git_hub_connections_start(None, None).await, || {
                format!("{endpoint}/v1/github-connection/start")
            })?;
            println!("{}", started.install_url);
            eprintln!(
                "Open that URL in a browser signed in to Cloudflare Access as you, and install the App before {}.",
                started.expires_at.to_rfc3339()
            );
            Ok(())
        }
        GithubCommand::Get => print_json(&shown(client.git_hub_connections_get().await, || {
            format!("{endpoint}/v1/github-connection")
        })?),
        GithubCommand::List => print_json(&shown(client.git_hub_connections_list().await, || {
            format!("{endpoint}/v1/github-connection/installations")
        })?),
        GithubCommand::Delete { installation_id } => {
            let installation = sessions::parsed("installation_id", &installation_id)?;
            answered(client.git_hub_connections_delete(&installation).await, || {
                format!("{endpoint}/v1/github-connection/installations/{installation_id}")
            })?;
            eprintln!(
                "removed GitHub App installation {installation_id}; uninstall the App on GitHub to remove it there too"
            );
            Ok(())
        }
        GithubCommand::ListRepositories => {
            print_json(&shown(client.git_hub_connections_list_repositories().await, || {
                format!("{endpoint}/v1/github-connection/repositories")
            })?)
        }
        GithubCommand::ListIssues { owner, name } => {
            let login = sessions::parsed("owner", &owner)?;
            let repository = sessions::parsed("name", &name)?;
            print_json(&shown(
                client.git_hub_connections_list_issues(&login, &repository).await,
                || format!("{endpoint}/v1/github-connection/repositories/{owner}/{name}/issues"),
            )?)
        }
    }
}

async fn tenant_config(command: ConfigCommand) -> Result<()> {
    match command {
        ConfigCommand::Validate { file } => {
            let config = tenant_config::parse(&file)?;
            bundle::read(&file, &config)?;
            eprintln!(
                "{} matches the TenantConfig schema and every persona file it names is readable",
                file.display()
            );
            Ok(())
        }
        ConfigCommand::Plan { file } => {
            let (_, client) = signed_in()?;
            tenant_config::print_plan(&tenant_config::plan(&client, &file, |name| std::env::var(name).ok()).await?);
            Ok(())
        }
        ConfigCommand::Apply { file } => {
            let (_, client) = signed_in()?;
            let applied = tenant_config::apply(&client, &file, |name| std::env::var(name).ok()).await?;
            tenant_config::print_plan(&applied);
            eprintln!("applied {} changes", applied.changes.len());
            Ok(())
        }
        ConfigCommand::Init { file } => {
            let (_, client) = signed_in()?;
            let live = tenant_config::live(&client).await?;
            tenant_config::write_new(&file, &tenant_config::starter(&live)?)?;
            eprintln!(
                "wrote {}; export the environment variables its secret sets name, and put each persona file's bytes at the from path it names, before tumpel config apply",
                file.display()
            );
            Ok(())
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("tumpel: starting the async runtime failed: {error}");
            return ExitCode::FAILURE;
        }
    };
    let outcome = runtime.block_on(async {
        let outcome = match cli.command {
            Command::Login { endpoint, name } => login(endpoint, name).await,
            Command::Whoami => whoami().await,
            Command::Tenants => members::tenants().await,
            Command::Member { command } => members::member(command).await,
            Command::Invitation { command } => members::invitation(command).await,
            Command::Devices { command } => devices(command).await,
            Command::Host { command } => host(command).await,
            Command::Harness { command } => resources::harnesses(command).await,
            Command::Release { command } => release(command).await,
            Command::Persona { command } => resources::personas(command).await,
            Command::Connector { command } => connectors::connector(command).await,
            Command::SecretSet { command } => resources::secret_sets(command).await,
            Command::Github { command } => github(command).await,
            Command::Config { command } => tenant_config(command).await,
            Command::Session { command } => sessions::session(command).await,
            Command::Turn { command } => sessions::turn(command).await,
            Command::Transcript { command } => sessions::transcript(command).await,
            Command::Snapshot { command } => sessions::snapshot(command).await,
            Command::Metrics { command } => metrics::metrics(command).await,
            Command::Prebuild { command } => prebuilds::prebuild(command).await,
            Command::PrebuildTemplate { command } => prebuilds::prebuild_template(command).await,
            Command::ClaudeSetup { command } => claude_setups::claude_setup(command).await,
            Command::Openrouter { command } => openrouter::openrouter(command).await,
            Command::OauthGrant { command } => connections::oauth_grant(command).await,
            Command::Connection { command } => connections::connection(command).await,
            Command::GithubActions { command } => connections::github_actions(command).await,
            Command::Circleci { command } => connections::circleci(command).await,
            Command::Namespace { command } => connections::namespace(command).await,
        };
        match outcome {
            Err(error) => Err(explained(error).await),
            done => done,
        }
    });
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("tumpel: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tumpel_api::types::Whoami;

    #[test]
    fn prints_an_answer_that_lacks_a_field_this_cli_expects() {
        let body = r#"{"tenant":{"tenantId":"7d0f0b8e-0000-4000-8000-000000000001"}}"#;
        let error = serde_json::from_str::<Whoami>(body).unwrap_err();
        let outcome: Result<ResponseValue<Whoami>, Error<Problem>> =
            Err(Error::InvalidResponsePayload(body.as_bytes().to_vec().into(), error));
        assert_eq!(
            shown(outcome, String::new).unwrap(),
            serde_json::json!({"tenant": {"tenantId": "7d0f0b8e-0000-4000-8000-000000000001"}})
        );
    }

    #[test]
    fn fails_on_a_problem_this_cli_cannot_decode() {
        let body =
            r#"{"code":"HostRetired","title":"Host retired","detail":"The host left the pool","fix":"Enrol it again"}"#;
        let error = serde_json::from_str::<Whoami>(body).unwrap_err();
        let outcome: Result<ResponseValue<Whoami>, Error<Problem>> =
            Err(Error::InvalidResponsePayload(body.as_bytes().to_vec().into(), error));
        assert_eq!(
            shown(outcome, String::new).unwrap_err().to_string(),
            "Host retired: The host left the pool. Enrol it again"
        );
    }

    fn parses(args: &[&str]) -> bool {
        Cli::try_parse_from(["tumpel"].into_iter().chain(args.iter().copied())).is_ok()
    }

    const PERSONA: [&str; 9] = [
        "persona",
        "create",
        "--name",
        "dev",
        "--harness",
        "ClaudeCode",
        "--secret-set-id",
        "5a8c0b0e-0000-4000-8000-000000000001",
        "--persona-file=a=b",
    ];

    #[test]
    fn persona_create_needs_connectors_or_an_explicit_none() {
        assert!(!parses(&PERSONA));
        assert!(parses(&[&PERSONA[..], &["--no-connectors"]].concat()));
        assert!(parses(
            &[&PERSONA[..], &["--connector", "a", "--connector", "b"]].concat()
        ));
        assert!(!parses(
            &[&PERSONA[..], &["--connector", "a", "--no-connectors"]].concat()
        ));
    }

    #[test]
    fn persona_update_takes_connectors_alone_or_an_explicit_none() {
        assert!(parses(&["persona", "update", "p1", "--connector", "a"]));
        assert!(parses(&["persona", "update", "p1", "--no-connectors"]));
        assert!(!parses(&[
            "persona",
            "update",
            "p1",
            "--connector",
            "a",
            "--no-connectors"
        ]));
        assert!(!parses(&[
            "persona",
            "update",
            "p1",
            "--file",
            "f.json",
            "--no-connectors"
        ]));
    }

    #[test]
    fn a_connector_secret_is_never_an_argument() {
        assert!(parses(&["connector", "set-token", "c1", "--stdin"]));
        assert!(!parses(&["connector", "set-token", "c1", "--token", "secret"]));
        assert!(!parses(&["connector", "set-token", "c1", "secret"]));
    }
}
