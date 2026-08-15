//! CLI for planning, applying, and verifying the latest-only deployment.

use std::{
    env,
    error::Error,
    fmt::Write as _,
    fs::OpenOptions,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    rc::Rc,
};

use anchor_client::{Client, Cluster, CommitmentConfig, Signer};
use anchor_lang::{prelude::Pubkey, solana_program::program_pack::Pack, AnchorSerialize};
use clap::{Args, Parser, Subcommand};
use serde::Serialize;
use solana_keypair::{read_keypair_file, write_keypair, Keypair};
use solana_loader_v3_interface::state::UpgradeableLoaderState;
use zkp2p_deployer::{parse_witness, DeploymentConfig, Topology};
use zkp2p_solana::{
    DisputeConfig, EscrowConfig, LifecyclePolicy, OrchestratorConfig, ProtocolConfig,
    RateManagerConfig, StakeVaultConfig, VerifierConfig, WhitelistConfig,
};

#[derive(Debug, Parser)]
#[command(about = "Deploy and verify only the latest ZKP2P Solana stack")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate public inputs and print the deterministic PDA topology without RPC writes.
    Plan(PlanArgs),
    /// Initialize if absent, then verify every canonical component and relationship.
    Apply(RpcArgs),
    /// Read-only verification of every canonical component and relationship.
    Verify(RpcArgs),
    /// Validate target cluster, mint, and any existing loader state without writes.
    Preflight(RpcArgs),
    /// Materialize `SOLANA_PRIVATE_KEY` into a mode-0600 temporary CLI keypair file.
    MaterializeKeypair(MaterializeArgs),
}

#[derive(Debug, Args)]
struct PlanArgs {
    /// Deployment authority public key.
    #[arg(long, env = "ZKP2P_AUTHORITY")]
    authority: Pubkey,
    #[command(flatten)]
    protocol: ProtocolArgs,
}

#[derive(Debug, Args)]
struct RpcArgs {
    /// HTTP RPC endpoint.
    #[arg(long, env = "SOLANA_RPC_URL")]
    rpc_url: String,
    /// Human-readable public cluster label recorded in the receipt.
    #[arg(long, env = "ZKP2P_CLUSTER_NAME")]
    cluster_name: String,
    /// Exact genesis hash authenticated before any deployment write.
    #[arg(long, env = "ZKP2P_EXPECTED_GENESIS_HASH")]
    expected_genesis_hash: String,
    /// Optional JSON keypair. If omitted, `SOLANA_PRIVATE_KEY` is read without logging it.
    #[arg(long, env = "SOLANA_KEYPAIR_PATH")]
    keypair: Option<PathBuf>,
    /// Optional path for the public verification receipt.
    #[arg(long)]
    receipt: Option<PathBuf>,
    #[command(flatten)]
    protocol: ProtocolArgs,
}

#[derive(Debug, Args)]
struct ProtocolArgs {
    /// Canonical SPL token mint.
    #[arg(long, env = "ZKP2P_STAKE_MINT")]
    stake_mint: Pubkey,
    /// Protocol fee recipient.
    #[arg(long, env = "ZKP2P_PROTOCOL_FEE_RECIPIENT")]
    protocol_fee_recipient: Pubkey,
    /// Fee at 1e18 precision.
    #[arg(long, env = "ZKP2P_PROTOCOL_FEE", default_value = "10000000000000000")]
    protocol_fee: u128,
    /// Default intent lifetime in seconds.
    #[arg(long, env = "ZKP2P_INTENT_EXPIRATION", default_value_t = 1_800)]
    intent_expiration_period: i64,
    /// Maximum live intents on one deposit.
    #[arg(long, env = "ZKP2P_MAX_INTENTS", default_value_t = 20)]
    max_intents_per_deposit: u16,
    /// StakeVault controller handover delay.
    #[arg(long, env = "ZKP2P_CONTROLLER_CHANGE_DELAY", default_value_t = 86_400)]
    controller_change_delay: i64,
    /// Ethereum witnesses, supplied repeatedly or comma-delimited.
    #[arg(
        long = "witness",
        env = "ZKP2P_INITIAL_WITNESSES",
        value_delimiter = ',',
        num_args = 1..
    )]
    witnesses: Vec<String>,
    /// Required unique witness signatures.
    #[arg(long, env = "ZKP2P_REQUIRED_SIGNATURES", default_value_t = 1)]
    required_signatures: u8,
}

#[derive(Debug, Args)]
struct MaterializeArgs {
    /// New file path; existing files are never overwritten.
    #[arg(long)]
    output: PathBuf,
}

#[derive(Debug, Serialize)]
struct VerificationReceipt {
    program: String,
    cluster: String,
    genesis_hash: String,
    programdata: String,
    programdata_slot: u64,
    upgrade_authority: String,
    authority: String,
    protocol_authority: String,
    domain_chain_id: String,
    domain_seed_slot: u64,
    domain_seed: String,
    stake_mint: String,
    protocol_fee_recipient: String,
    protocol_fee: String,
    required_signatures: u8,
    witness_count: usize,
    configuration_fingerprint: String,
    initialization_signature: Option<String>,
    topology: Topology,
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct PreflightReceipt {
    program: String,
    cluster: String,
    genesis_hash: String,
    program_state: &'static str,
    programdata: Option<String>,
    programdata_slot: Option<u64>,
    upgrade_authority: Option<String>,
    stake_mint: String,
    domain_chain_id: Option<String>,
    domain_seed_slot: Option<u64>,
    domain_seed: Option<String>,
    status: &'static str,
}

fn main() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::Plan(args) => {
            let config = args.protocol.config(args.authority)?;
            config.validate()?;
            let output = serde_json::to_string_pretty(&Topology::derive())?;
            println!("{output}");
        }
        Command::Apply(args) => run_rpc(args, true)?,
        Command::Verify(args) => run_rpc(args, false)?,
        Command::Preflight(args) => run_preflight(args)?,
        Command::MaterializeKeypair(args) => materialize_keypair(&args.output)?,
    }
    Ok(())
}

fn run_preflight(args: RpcArgs) -> Result<(), Box<dyn Error>> {
    let payer = load_keypair(args.keypair.as_deref())?;
    let authority = Pubkey::new_from_array(payer.pubkey().to_bytes());
    let config = args.protocol.config(authority)?;
    config.validate()?;
    let cluster = Cluster::Custom(args.rpc_url.clone(), websocket_url(&args.rpc_url)?);
    let client = Client::new_with_options(cluster, Rc::new(payer), CommitmentConfig::confirmed());
    let program = client.program(zkp2p_solana::ID)?;
    let genesis_hash = program.rpc().get_genesis_hash()?.to_string();
    verify_genesis_hash(&genesis_hash, &args.expected_genesis_hash)?;
    let mint_account = program.rpc().get_account(&config.stake_mint)?;
    validate_legacy_mint(&mint_account.owner, &mint_account.data)?;

    let existing_program = program
        .rpc()
        .get_account_with_commitment(&zkp2p_solana::ID, CommitmentConfig::confirmed())?
        .value;
    let (program_state, programdata, programdata_slot, upgrade_authority, deployment_domain) =
        if let Some(program_account) = existing_program {
            if !program_account.executable {
                return Err("program account exists but is not executable".into());
            }
            let address = programdata_address(&program_account.owner, &program_account.data)?;
            let account = program.rpc().get_account(&address)?;
            let (verified_authority, slot) =
                verify_programdata_control(&account.owner, &account.data, &config.authority)?;
            let initialized = program
                .rpc()
                .get_account_with_commitment(
                    &Topology::derive().protocol,
                    CommitmentConfig::confirmed(),
                )?
                .value
                .is_some();
            let deployment_domain = if initialized {
                let protocol: ProtocolConfig = program.account(Topology::derive().protocol)?;
                validate_deployment_domain(&protocol)?;
                Some((
                    hex32(&protocol.domain_chain_id),
                    protocol.domain_seed_slot,
                    hex32(&protocol.domain_seed),
                ))
            } else {
                None
            };
            (
                if initialized {
                    "initialized"
                } else {
                    "executable-uninitialized"
                },
                Some(address.to_string()),
                Some(slot),
                Some(verified_authority.to_string()),
                deployment_domain,
            )
        } else {
            ("absent", None, None, None, None)
        };
    let (domain_chain_id, domain_seed_slot, domain_seed) = deployment_domain
        .map_or((None, None, None), |(domain, slot, seed)| {
            (Some(domain), Some(slot), Some(seed))
        });
    let receipt = PreflightReceipt {
        program: zkp2p_solana::ID.to_string(),
        cluster: args.cluster_name,
        genesis_hash,
        program_state,
        programdata,
        programdata_slot,
        upgrade_authority,
        stake_mint: config.stake_mint.to_string(),
        domain_chain_id,
        domain_seed_slot,
        domain_seed,
        status: "verified",
    };
    let output = serde_json::to_string_pretty(&receipt)?;
    if let Some(path) = args.receipt {
        std::fs::write(path, output.as_bytes())?;
    }
    println!("{output}");
    Ok(())
}

impl ProtocolArgs {
    fn config(&self, authority: Pubkey) -> Result<DeploymentConfig, Box<dyn Error>> {
        let initial_witnesses = self
            .witnesses
            .iter()
            .map(|witness| parse_witness(witness))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(DeploymentConfig {
            authority,
            stake_mint: self.stake_mint,
            protocol_fee: self.protocol_fee,
            protocol_fee_recipient: self.protocol_fee_recipient,
            intent_expiration_period: self.intent_expiration_period,
            max_intents_per_deposit: self.max_intents_per_deposit,
            controller_change_delay: self.controller_change_delay,
            initial_witnesses,
            required_signatures: self.required_signatures,
        })
    }
}

fn run_rpc(args: RpcArgs, apply: bool) -> Result<(), Box<dyn Error>> {
    let payer = load_keypair(args.keypair.as_deref())?;
    let authority = Pubkey::new_from_array(payer.pubkey().to_bytes());
    let config = args.protocol.config(authority)?;
    config.validate()?;
    let cluster = Cluster::Custom(args.rpc_url.clone(), websocket_url(&args.rpc_url)?);
    let client = Client::new_with_options(cluster, Rc::new(payer), CommitmentConfig::confirmed());
    let program = client.program(zkp2p_solana::ID)?;
    let genesis_hash = program.rpc().get_genesis_hash()?.to_string();
    verify_genesis_hash(&genesis_hash, &args.expected_genesis_hash)?;
    let program_account = program.rpc().get_account(&zkp2p_solana::ID)?;
    if !program_account.executable {
        return Err("program account exists but is not executable".into());
    }
    let programdata_address = programdata_address(&program_account.owner, &program_account.data)?;
    let programdata_account = program.rpc().get_account(&programdata_address)?;
    let (upgrade_authority, programdata_slot) = verify_programdata_control(
        &programdata_account.owner,
        &programdata_account.data,
        &config.authority,
    )?;
    let mint_account = program.rpc().get_account(&config.stake_mint)?;
    validate_legacy_mint(&mint_account.owner, &mint_account.data)?;

    let topology = Topology::derive();
    let initialized = program
        .rpc()
        .get_account_with_commitment(&topology.protocol, CommitmentConfig::confirmed())?
        .value
        .is_some();
    let initialization_signature = if apply && !initialized {
        Some(
            program
                .request()
                .instruction(config.initialize_instruction()?)
                .send()?
                .to_string(),
        )
    } else {
        None
    };
    if !apply && !initialized {
        return Err("protocol is not initialized".into());
    }

    let protocol: ProtocolConfig = program.account(topology.protocol)?;
    let escrow: EscrowConfig = program.account(topology.escrow_config)?;
    let verifier: VerifierConfig = program.account(topology.verifier_config)?;
    let orchestrator: OrchestratorConfig = program.account(topology.orchestrator_config)?;
    let stake_vault: StakeVaultConfig = program.account(topology.stake_vault_config)?;
    let rate_manager: RateManagerConfig = program.account(topology.rate_manager_config)?;
    let whitelist: WhitelistConfig = program.account(topology.whitelist_config)?;
    let dispute: DisputeConfig = program.account(topology.dispute_config)?;

    require_state(
        protocol.authority != Pubkey::default(),
        "protocol authority",
    )?;
    require_state(protocol.version == 1, "latest schema version")?;
    validate_deployment_domain(&protocol)?;
    require_state(escrow.protocol == topology.protocol, "escrow protocol root")?;
    require_state(escrow.token_mint == config.stake_mint, "escrow token mint")?;
    require_state(escrow.dust_recipient != Pubkey::default(), "dust recipient")?;
    require_state(
        escrow.intent_expiration_period > 0
            && escrow.intent_expiration_period
                <= zkp2p_solana::constants::MAX_INTENT_LIFETIME_SECONDS,
        "intent expiration bounds",
    )?;
    require_state(escrow.max_intents_per_deposit > 0, "maximum intents")?;
    require_state(
        verifier.protocol == topology.protocol,
        "verifier protocol root",
    )?;
    require_state(
        verifier.domain_chain_id == protocol.domain_chain_id,
        "verifier deployment domain",
    )?;
    require_state(
        !verifier.witnesses.is_empty()
            && verifier.witnesses.len() <= zkp2p_solana::constants::MAX_WITNESSES
            && verifier.required_signatures > 0
            && usize::from(verifier.required_signatures) <= verifier.witnesses.len(),
        "witness bounds",
    )?;
    require_state(
        orchestrator.protocol == topology.protocol
            && orchestrator.escrow_config == topology.escrow_config
            && orchestrator.verifier_config == topology.verifier_config
            && orchestrator.domain_chain_id == protocol.domain_chain_id,
        "orchestrator wiring",
    )?;
    require_state(
        orchestrator.protocol_fee <= zkp2p_solana::constants::MAX_FEE
            && orchestrator.protocol_fee_recipient != Pubkey::default(),
        "orchestrator fee bounds",
    )?;
    require_state(
        stake_vault.protocol == topology.protocol
            && stake_vault.stake_mint == config.stake_mint
            && stake_vault.controller != Pubkey::default(),
        "StakeVault wiring",
    )?;
    require_state(
        stake_vault.controller_change_delay == config.controller_change_delay,
        "controller delay",
    )?;
    require_state(
        rate_manager.protocol == topology.protocol,
        "RateManager protocol root",
    )?;
    require_state(
        whitelist.protocol == topology.protocol,
        "whitelist protocol root",
    )?;
    require_state(
        dispute.protocol == topology.protocol
            && dispute.stake_vault == topology.stake_vault_config
            && dispute.verifier_config == topology.verifier_config
            && dispute.domain_chain_id == protocol.domain_chain_id,
        "dispute wiring",
    )?;
    if initialization_signature.is_some() {
        require_state(
            protocol.authority == config.authority,
            "initial protocol authority",
        )?;
        require_state(
            protocol.pending_authority.is_none(),
            "initial pending authority",
        )?;
        require_state(
            escrow.dust_recipient == config.protocol_fee_recipient
                && escrow.dust_threshold == 0
                && escrow.intent_expiration_period == config.intent_expiration_period
                && escrow.max_intents_per_deposit == config.max_intents_per_deposit
                && !escrow.paused,
            "initial escrow defaults",
        )?;
        require_state(
            verifier.required_signatures == config.required_signatures
                && verifier.witnesses == config.initial_witnesses
                && verifier.payment_methods.is_empty(),
            "initial verifier defaults",
        )?;
        require_state(
            orchestrator.protocol_fee == config.protocol_fee
                && orchestrator.protocol_fee_recipient == config.protocol_fee_recipient
                && orchestrator.lifecycle_policy == LifecyclePolicy::WhitelistAndDispute
                && !orchestrator.paused,
            "initial orchestrator defaults",
        )?;
        require_state(
            stake_vault.controller_change_delay == config.controller_change_delay
                && stake_vault.controller == topology.dispute_config
                && stake_vault.pending_controller.is_none(),
            "initial StakeVault defaults",
        )?;
        require_state(
            !dispute.admissions_paused,
            "initial dispute admission state",
        )?;
    }

    let mut configuration_bytes = Vec::new();
    macro_rules! fingerprint_field {
        ($value:expr) => {
            AnchorSerialize::serialize(&$value, &mut configuration_bytes)?
        };
    }
    fingerprint_field!(protocol.authority);
    fingerprint_field!(protocol.pending_authority);
    fingerprint_field!(protocol.version);
    fingerprint_field!(protocol.domain_seed_slot);
    fingerprint_field!(protocol.domain_seed);
    fingerprint_field!(protocol.domain_chain_id);
    fingerprint_field!(escrow.dust_recipient);
    fingerprint_field!(escrow.dust_threshold);
    fingerprint_field!(escrow.max_intents_per_deposit);
    fingerprint_field!(escrow.intent_expiration_period);
    fingerprint_field!(escrow.paused);
    fingerprint_field!(verifier.required_signatures);
    fingerprint_field!(verifier.witnesses);
    fingerprint_field!(verifier.payment_methods);
    fingerprint_field!(orchestrator.protocol_fee);
    fingerprint_field!(orchestrator.protocol_fee_recipient);
    fingerprint_field!(orchestrator.lifecycle_policy);
    fingerprint_field!(orchestrator.allow_multiple_intents);
    fingerprint_field!(orchestrator.paused);
    fingerprint_field!(stake_vault.controller);
    fingerprint_field!(stake_vault.pending_controller);
    fingerprint_field!(stake_vault.pending_controller_valid_at);
    fingerprint_field!(stake_vault.controller_change_delay);
    fingerprint_field!(dispute.admissions_paused);
    let configuration_fingerprint = solana_keccak_hasher::hash(&configuration_bytes).to_string();

    let receipt = VerificationReceipt {
        program: zkp2p_solana::ID.to_string(),
        cluster: args.cluster_name,
        genesis_hash,
        programdata: programdata_address.to_string(),
        programdata_slot,
        upgrade_authority: upgrade_authority.to_string(),
        authority: config.authority.to_string(),
        protocol_authority: protocol.authority.to_string(),
        domain_chain_id: hex32(&protocol.domain_chain_id),
        domain_seed_slot: protocol.domain_seed_slot,
        domain_seed: hex32(&protocol.domain_seed),
        stake_mint: config.stake_mint.to_string(),
        protocol_fee_recipient: orchestrator.protocol_fee_recipient.to_string(),
        protocol_fee: orchestrator.protocol_fee.to_string(),
        required_signatures: verifier.required_signatures,
        witness_count: verifier.witnesses.len(),
        configuration_fingerprint,
        initialization_signature,
        topology,
        status: "verified",
    };
    let output = serde_json::to_string_pretty(&receipt)?;
    if let Some(path) = args.receipt {
        std::fs::write(path, output.as_bytes())?;
    }
    println!("{output}");
    Ok(())
}

fn verify_genesis_hash(actual: &str, expected: &str) -> Result<(), Box<dyn Error>> {
    if expected.is_empty() {
        return Err("expected genesis hash must be nonempty".into());
    }
    if actual == expected {
        Ok(())
    } else {
        Err(format!("cluster genesis hash mismatch: expected {expected}, got {actual}").into())
    }
}

fn validate_deployment_domain(protocol: &ProtocolConfig) -> Result<(), Box<dyn Error>> {
    require_state(
        protocol.domain_seed != [0; 32],
        "nonzero deployment-domain seed",
    )?;
    let expected = solana_keccak_hasher::hashv(&[
        zkp2p_solana::constants::DEPLOYMENT_DOMAIN_PREFIX,
        zkp2p_solana::ID.as_ref(),
        &protocol.domain_seed,
    ])
    .to_bytes();
    require_state(expected != [0; 32], "nonzero deployment domain")?;
    require_state(
        protocol.domain_chain_id == expected,
        "runtime-derived deployment domain",
    )
}

fn hex32(value: &[u8; 32]) -> String {
    let mut output = String::with_capacity(64);
    for byte in value {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

fn validate_legacy_mint(owner: &Pubkey, data: &[u8]) -> Result<(), Box<dyn Error>> {
    require_state(
        *owner == anchor_spl::token::ID,
        "legacy SPL stake mint owner",
    )?;
    let mint = anchor_spl::token::spl_token::state::Mint::unpack(data)
        .map_err(|_| "stake mint account is malformed")?;
    require_state(mint.is_initialized, "initialized stake mint")?;
    require_state(mint.decimals == 6, "six-decimal stake mint")
}

fn require_upgradeable_loader(owner: &Pubkey, label: &str) -> Result<(), Box<dyn Error>> {
    let expected = anchor_lang::solana_program::bpf_loader_upgradeable::ID.to_bytes();
    if owner.to_bytes() == expected {
        Ok(())
    } else {
        Err(format!("{label} is not owned by the upgradeable BPF loader").into())
    }
}

fn decode_loader_state(data: &[u8]) -> Result<UpgradeableLoaderState, Box<dyn Error>> {
    bincode::deserialize_from(&mut std::io::Cursor::new(data))
        .map_err(|error| format!("invalid upgradeable-loader state: {error}").into())
}

fn programdata_address(owner: &Pubkey, data: &[u8]) -> Result<Pubkey, Box<dyn Error>> {
    require_upgradeable_loader(owner, "program account")?;
    match decode_loader_state(data)? {
        UpgradeableLoaderState::Program {
            programdata_address,
        } => Ok(Pubkey::new_from_array(programdata_address.to_bytes())),
        _ => Err("executable account is not an upgradeable-loader Program account".into()),
    }
}

fn verify_programdata_control(
    owner: &Pubkey,
    data: &[u8],
    expected_authority: &Pubkey,
) -> Result<(Pubkey, u64), Box<dyn Error>> {
    require_upgradeable_loader(owner, "ProgramData account")?;
    match decode_loader_state(data)? {
        UpgradeableLoaderState::ProgramData {
            slot,
            upgrade_authority_address: Some(authority),
        } if authority.to_bytes() == expected_authority.to_bytes() => {
            Ok((Pubkey::new_from_array(authority.to_bytes()), slot))
        }
        UpgradeableLoaderState::ProgramData {
            upgrade_authority_address: None,
            ..
        } => Err("ProgramData is immutable; expected the configured upgrade authority".into()),
        UpgradeableLoaderState::ProgramData { .. } => {
            Err("ProgramData upgrade authority does not match deployment authority".into())
        }
        _ => Err("derived account is not an upgradeable-loader ProgramData account".into()),
    }
}

fn require_state(condition: bool, label: &str) -> Result<(), Box<dyn Error>> {
    if condition {
        Ok(())
    } else {
        Err(format!("topology verification failed: {label}").into())
    }
}

fn load_keypair(path: Option<&Path>) -> Result<Keypair, Box<dyn Error>> {
    if let Some(path) = path {
        return read_keypair_file(path);
    }
    let encoded = env::var("SOLANA_PRIVATE_KEY")
        .map_err(|_| "SOLANA_PRIVATE_KEY is required when --keypair is omitted")?;
    Keypair::try_from_base58_string(&encoded).map_err(|_| "SOLANA_PRIVATE_KEY is invalid".into())
}

fn materialize_keypair(output: &Path) -> Result<(), Box<dyn Error>> {
    let keypair = load_keypair(None)?;
    write_keypair_exclusive(&keypair, output)?;
    println!(
        "keypair_path={} payer={}",
        output.display(),
        keypair.pubkey()
    );
    Ok(())
}

fn write_keypair_exclusive(keypair: &Keypair, output: &Path) -> Result<(), Box<dyn Error>> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(output)?;
    write_keypair(keypair, &mut file)?;
    Ok(())
}

fn websocket_url(rpc_url: &str) -> Result<String, Box<dyn Error>> {
    if let Some(rest) = rpc_url.strip_prefix("https://") {
        return Ok(format!("wss://{rest}"));
    }
    if let Some(rest) = rpc_url.strip_prefix("http://") {
        return Ok(format!("ws://{rest}"));
    }
    Err("RPC URL must start with http:// or https://".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anchor_lang::solana_program::program_option::COption;
    use solana_loader_v3_interface::state::UpgradeableLoaderState;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn websocket_scheme_tracks_rpc_transport() {
        assert_eq!(
            websocket_url("https://api.devnet.solana.com").expect("websocket URL"),
            "wss://api.devnet.solana.com"
        );
        assert_eq!(
            websocket_url("http://127.0.0.1:8899").expect("websocket URL"),
            "ws://127.0.0.1:8899"
        );
        assert!(websocket_url("file:///tmp/rpc").is_err());
    }

    #[test]
    fn cluster_identity_fails_closed() {
        assert!(verify_genesis_hash("actual", "").is_err());
        assert!(verify_genesis_hash("actual", "wrong").is_err());
        verify_genesis_hash("actual", "actual").expect("matching genesis hash");
    }

    #[test]
    fn deployment_domain_receipt_is_full_width_and_recomputed() {
        let seed = [7_u8; 32];
        let domain = solana_keccak_hasher::hashv(&[
            zkp2p_solana::constants::DEPLOYMENT_DOMAIN_PREFIX,
            zkp2p_solana::ID.as_ref(),
            &seed,
        ])
        .to_bytes();
        let mut protocol = ProtocolConfig {
            authority: Pubkey::new_unique(),
            pending_authority: None,
            version: 1,
            domain_seed_slot: 42,
            domain_seed: seed,
            domain_chain_id: domain,
            bump: 1,
        };
        validate_deployment_domain(&protocol).expect("valid runtime-derived domain");
        assert_eq!(hex32(&domain).len(), 64);

        protocol.domain_chain_id = [8; 32];
        assert!(validate_deployment_domain(&protocol).is_err());
        protocol.domain_chain_id = domain;
        protocol.domain_seed = [0; 32];
        assert!(validate_deployment_domain(&protocol).is_err());
    }

    #[test]
    fn legacy_mint_preflight_requires_initialized_six_decimal_state() {
        let mint = anchor_spl::token::spl_token::state::Mint {
            mint_authority: COption::None,
            supply: 0,
            decimals: 6,
            is_initialized: true,
            freeze_authority: COption::None,
        };
        let mut data = vec![0_u8; anchor_spl::token::spl_token::state::Mint::LEN];
        anchor_spl::token::spl_token::state::Mint::pack(mint, &mut data).expect("pack valid mint");
        validate_legacy_mint(&anchor_spl::token::ID, &data).expect("valid legacy mint");
        assert!(validate_legacy_mint(&Pubkey::new_unique(), &data).is_err());
        assert!(validate_legacy_mint(&anchor_spl::token::ID, &[0; 4]).is_err());

        let mut wrong_decimals = data.clone();
        *wrong_decimals.get_mut(44).expect("mint decimals") = 9;
        assert!(validate_legacy_mint(&anchor_spl::token::ID, &wrong_decimals).is_err());
        let mut uninitialized = data;
        *uninitialized.get_mut(45).expect("mint state") = 0;
        assert!(validate_legacy_mint(&anchor_spl::token::ID, &uninitialized).is_err());
    }

    #[test]
    fn materialized_keypair_is_private_and_never_overwritten() {
        let keypair = Keypair::new();
        let path = env::temp_dir().join(format!(
            "zkp2p-deployer-{}-{}.json",
            std::process::id(),
            keypair.pubkey()
        ));
        write_keypair_exclusive(&keypair, &path).expect("create private keypair file");

        let metadata = std::fs::metadata(&path).expect("keypair metadata");
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        assert!(write_keypair_exclusive(&Keypair::new(), &path).is_err());
        let loaded = read_keypair_file(&path).expect("read materialized keypair");
        assert_eq!(loaded.pubkey(), keypair.pubkey());

        std::fs::remove_file(path).expect("remove test keypair");
    }

    #[test]
    fn program_control_verification_fails_closed() {
        let loader = anchor_lang::solana_program::bpf_loader_upgradeable::ID;
        let expected_authority = Pubkey::new_unique();
        let programdata = Pubkey::new_unique();
        let interface_programdata = programdata
            .to_string()
            .parse()
            .expect("interface programdata address");
        let interface_authority = expected_authority
            .to_string()
            .parse()
            .expect("interface upgrade authority");
        let program_state = bincode::serialize(&UpgradeableLoaderState::Program {
            programdata_address: interface_programdata,
        })
        .expect("serialize Program state");
        assert_eq!(
            programdata_address(&loader, &program_state).expect("programdata link"),
            programdata
        );

        let controlled_state = bincode::serialize(&UpgradeableLoaderState::ProgramData {
            slot: 1,
            upgrade_authority_address: Some(interface_authority),
        })
        .expect("serialize ProgramData state");
        let (verified_authority, slot) =
            verify_programdata_control(&loader, &controlled_state, &expected_authority)
                .expect("matching authority");
        assert_eq!(verified_authority, expected_authority);
        assert_eq!(slot, 1);

        let wrong_authority = Pubkey::new_unique();
        assert!(verify_programdata_control(&loader, &controlled_state, &wrong_authority).is_err());
        let immutable_state = bincode::serialize(&UpgradeableLoaderState::ProgramData {
            slot: 1,
            upgrade_authority_address: None,
        })
        .expect("serialize immutable ProgramData state");
        assert!(
            verify_programdata_control(&loader, &immutable_state, &expected_authority).is_err()
        );
        assert!(programdata_address(&Pubkey::new_unique(), &program_state).is_err());
    }
}
