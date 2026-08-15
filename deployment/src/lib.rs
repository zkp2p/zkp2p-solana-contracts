//! Latest-only deployment planning and topology validation.

use anchor_lang::{prelude::Pubkey, InstructionData, ToAccountMetas};
use serde::Serialize;
use zkp2p_solana::{
    constants::{
        DISPUTE_CONFIG_SEED, ESCROW_CONFIG_SEED, ORCHESTRATOR_CONFIG_SEED, PROTOCOL_SEED,
        RATE_MANAGER_CONFIG_SEED, STAKE_VAULT_CONFIG_SEED, VERIFIER_CONFIG_SEED,
        WHITELIST_CONFIG_SEED,
    },
    InitializeProtocolArgs,
};

/// All public inputs that become canonical protocol configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeploymentConfig {
    /// Deployment and initial governance authority.
    pub authority: Pubkey,
    /// Canonical SPL token mint used by escrow and StakeVault.
    pub stake_mint: Pubkey,
    /// Initial protocol fee, at 1e18 precision.
    pub protocol_fee: u128,
    /// Initial protocol fee recipient.
    pub protocol_fee_recipient: Pubkey,
    /// Default escrow intent lifetime in seconds.
    pub intent_expiration_period: i64,
    /// Maximum concurrent intents for one deposit.
    pub max_intents_per_deposit: u16,
    /// Delay for StakeVault controller changes.
    pub controller_change_delay: i64,
    /// Initial Ethereum witness addresses.
    pub initial_witnesses: Vec<[u8; 20]>,
    /// Initial unique-signature threshold.
    pub required_signatures: u8,
}

impl DeploymentConfig {
    /// Performs off-chain validation before any transaction can be built.
    pub fn validate(&self) -> Result<(), String> {
        if self.authority == Pubkey::default()
            || self.stake_mint == Pubkey::default()
            || self.protocol_fee_recipient == Pubkey::default()
        {
            return Err("authority, stake mint, and fee recipient must be nonzero".to_owned());
        }
        if self.protocol_fee > zkp2p_solana::constants::MAX_FEE {
            return Err("protocol fee exceeds the immutable 5% ceiling".to_owned());
        }
        if self.intent_expiration_period <= 0
            || self.intent_expiration_period > zkp2p_solana::constants::MAX_INTENT_LIFETIME_SECONDS
        {
            return Err("intent expiration must be in 1..=5 days".to_owned());
        }
        if self.max_intents_per_deposit == 0 {
            return Err("maximum intents per deposit must be nonzero".to_owned());
        }
        if self.controller_change_delay
            < zkp2p_solana::constants::MIN_CONTROLLER_CHANGE_DELAY_SECONDS
        {
            return Err("controller change delay must be at least one day".to_owned());
        }
        if self.initial_witnesses.is_empty()
            || self.initial_witnesses.len() > zkp2p_solana::constants::MAX_WITNESSES
        {
            return Err("initial witness count must be in 1..=2".to_owned());
        }
        if self.required_signatures == 0
            || usize::from(self.required_signatures) > self.initial_witnesses.len()
        {
            return Err("signature threshold must be in 1..=witness count".to_owned());
        }
        for (index, witness) in self.initial_witnesses.iter().enumerate() {
            if *witness == [0; 20] {
                return Err("witnesses must be nonzero".to_owned());
            }
            if self
                .initial_witnesses
                .get(..index)
                .is_some_and(|earlier| earlier.contains(witness))
            {
                return Err("witnesses must be unique".to_owned());
            }
        }
        Ok(())
    }

    /// Builds the one latest-stack initialization instruction.
    pub fn initialize_instruction(&self) -> Result<anchor_client::Instruction, String> {
        self.validate()?;
        let topology = Topology::derive();
        Ok(anchor_client::Instruction {
            program_id: zkp2p_solana::ID,
            accounts: zkp2p_solana::accounts::InitializeProtocol {
                authority: self.authority,
                program: zkp2p_solana::ID,
                program_data: program_data_address(),
                slot_hashes: solana_program::sysvar::slot_hashes::ID,
                protocol: topology.protocol,
                stake_mint: self.stake_mint,
                escrow_config: topology.escrow_config,
                verifier_config: topology.verifier_config,
                orchestrator_config: topology.orchestrator_config,
                stake_vault_config: topology.stake_vault_config,
                rate_manager_config: topology.rate_manager_config,
                whitelist_config: topology.whitelist_config,
                dispute_config: topology.dispute_config,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: zkp2p_solana::instruction::InitializeProtocol {
                args: InitializeProtocolArgs {
                    protocol_fee: self.protocol_fee,
                    protocol_fee_recipient: self.protocol_fee_recipient,
                    intent_expiration_period: self.intent_expiration_period,
                    max_intents_per_deposit: self.max_intents_per_deposit,
                    controller_change_delay: self.controller_change_delay,
                    initial_witnesses: self.initial_witnesses.clone(),
                    required_signatures: self.required_signatures,
                },
            }
            .data(),
        })
    }
}

/// Canonical latest-stack PDA topology.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Topology {
    /// Program ID.
    pub program: String,
    /// Protocol root PDA.
    #[serde(skip)]
    pub protocol: Pubkey,
    /// Escrow configuration PDA.
    #[serde(skip)]
    pub escrow_config: Pubkey,
    /// Unified verifier configuration PDA.
    #[serde(skip)]
    pub verifier_config: Pubkey,
    /// Orchestrator configuration PDA.
    #[serde(skip)]
    pub orchestrator_config: Pubkey,
    /// StakeVault configuration PDA.
    #[serde(skip)]
    pub stake_vault_config: Pubkey,
    /// RateManager configuration PDA.
    #[serde(skip)]
    pub rate_manager_config: Pubkey,
    /// Whitelist configuration PDA.
    #[serde(skip)]
    pub whitelist_config: Pubkey,
    /// Dispute configuration PDA.
    #[serde(skip)]
    pub dispute_config: Pubkey,
    /// Serializable protocol PDA.
    pub protocol_address: String,
    /// Serializable escrow PDA.
    pub escrow_config_address: String,
    /// Serializable verifier PDA.
    pub verifier_config_address: String,
    /// Serializable orchestrator PDA.
    pub orchestrator_config_address: String,
    /// Serializable StakeVault PDA.
    pub stake_vault_config_address: String,
    /// Serializable RateManager PDA.
    pub rate_manager_config_address: String,
    /// Serializable whitelist PDA.
    pub whitelist_config_address: String,
    /// Serializable dispute PDA.
    pub dispute_config_address: String,
}

impl Topology {
    /// Derives every canonical component without reading cluster state.
    pub fn derive() -> Self {
        let protocol = pda(PROTOCOL_SEED);
        let escrow_config = pda(ESCROW_CONFIG_SEED);
        let verifier_config = pda(VERIFIER_CONFIG_SEED);
        let orchestrator_config = pda(ORCHESTRATOR_CONFIG_SEED);
        let stake_vault_config = pda(STAKE_VAULT_CONFIG_SEED);
        let rate_manager_config = pda(RATE_MANAGER_CONFIG_SEED);
        let whitelist_config = pda(WHITELIST_CONFIG_SEED);
        let dispute_config = pda(DISPUTE_CONFIG_SEED);
        Self {
            program: zkp2p_solana::ID.to_string(),
            protocol_address: protocol.to_string(),
            escrow_config_address: escrow_config.to_string(),
            verifier_config_address: verifier_config.to_string(),
            orchestrator_config_address: orchestrator_config.to_string(),
            stake_vault_config_address: stake_vault_config.to_string(),
            rate_manager_config_address: rate_manager_config.to_string(),
            whitelist_config_address: whitelist_config.to_string(),
            dispute_config_address: dispute_config.to_string(),
            protocol,
            escrow_config,
            verifier_config,
            orchestrator_config,
            stake_vault_config,
            rate_manager_config,
            whitelist_config,
            dispute_config,
        }
    }
}

fn pda(seed: &[u8]) -> Pubkey {
    Pubkey::find_program_address(&[seed], &zkp2p_solana::ID).0
}

fn program_data_address() -> Pubkey {
    let loader = anchor_lang::solana_program::bpf_loader_upgradeable::ID;
    Pubkey::find_program_address(&[zkp2p_solana::ID.as_ref()], &loader).0
}

/// Parses one 20-byte Ethereum address without accepting truncation or padding.
pub fn parse_witness(value: &str) -> Result<[u8; 20], String> {
    let raw = value.strip_prefix("0x").unwrap_or(value);
    if raw.len() != 40 {
        return Err("witness must contain exactly 20 bytes".to_owned());
    }
    let mut witness = [0_u8; 20];
    for (index, destination) in witness.iter_mut().enumerate() {
        let start = index.checked_mul(2).ok_or("witness index overflow")?;
        let end = start.checked_add(2).ok_or("witness index overflow")?;
        let pair = raw
            .get(start..end)
            .ok_or("witness must be valid UTF-8 hex")?;
        *destination =
            u8::from_str_radix(pair, 16).map_err(|_| "witness must be hexadecimal".to_owned())?;
    }
    Ok(witness)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> DeploymentConfig {
        DeploymentConfig {
            authority: Pubkey::new_unique(),
            stake_mint: Pubkey::new_unique(),
            protocol_fee: 10_000_000_000_000_000,
            protocol_fee_recipient: Pubkey::new_unique(),
            intent_expiration_period: 1_800,
            max_intents_per_deposit: 20,
            controller_change_delay: 86_400,
            initial_witnesses: vec![[7; 20]],
            required_signatures: 1,
        }
    }

    #[test]
    fn plan_contains_only_latest_canonical_components() {
        let config = valid_config();
        let instruction = config.initialize_instruction().expect("valid plan");
        let topology = Topology::derive();
        assert_eq!(instruction.program_id, zkp2p_solana::ID);
        assert_eq!(instruction.accounts.len(), 14);
        assert!(instruction
            .accounts
            .iter()
            .any(|meta| meta.pubkey == solana_program::sysvar::slot_hashes::ID));
        for component in [
            topology.protocol,
            topology.escrow_config,
            topology.verifier_config,
            topology.orchestrator_config,
            topology.stake_vault_config,
            topology.rate_manager_config,
            topology.whitelist_config,
            topology.dispute_config,
        ] {
            assert!(instruction
                .accounts
                .iter()
                .any(|meta| meta.pubkey == component));
        }
    }

    #[test]
    fn plan_rejects_unsafe_configuration_before_rpc() {
        let mut config = valid_config();
        config.required_signatures = 2;
        assert!(config.initialize_instruction().is_err());
        config.required_signatures = 1;
        config.initial_witnesses.push([7; 20]);
        assert!(config.initialize_instruction().is_err());
        config.initial_witnesses = vec![[7; 20]];
        config.controller_change_delay = 1;
        assert!(config.initialize_instruction().is_err());
    }

    #[test]
    fn witness_parser_is_exact_width_and_case_insensitive() {
        assert_eq!(
            parse_witness("0x00112233445566778899aAbBcCdDeEfF00112233"),
            Ok([
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff, 0x00, 0x11, 0x22, 0x33,
            ])
        );
        assert!(parse_witness("0x01").is_err());
        assert!(parse_witness("zz112233445566778899aabbccddeeff00112233").is_err());
    }
}
