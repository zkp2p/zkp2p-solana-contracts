//! Shared LiteSVM fixture for real-SBF parity tests.

#![allow(dead_code)]

use std::path::PathBuf;

use anchor_lang::{InstructionData, ToAccountMetas};
use litesvm::LiteSVM;
use solana_account::Account;
use solana_address::Address;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_loader_v3_interface::state::UpgradeableLoaderState;
use solana_message::{v0, AddressLookupTableAccount, Message, VersionedMessage};
use solana_signer::Signer;
use solana_transaction::{versioned::VersionedTransaction, Transaction};
use zkp2p_solana::{
    constants::{
        DISPUTE_CONFIG_SEED, ESCROW_CONFIG_SEED, ORCHESTRATOR_CONFIG_SEED, PROTOCOL_SEED,
        RATE_MANAGER_CONFIG_SEED, STAKE_VAULT_CONFIG_SEED, VERIFIER_CONFIG_SEED,
        WHITELIST_CONFIG_SEED,
    },
    InitializeProtocolArgs,
};

pub fn address(bytes: [u8; 32]) -> Address {
    Address::new_from_array(bytes)
}

pub fn anchor_pubkey(value: Address) -> anchor_lang::prelude::Pubkey {
    anchor_lang::prelude::Pubkey::new_from_array(value.to_bytes())
}

pub fn program_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/deploy/zkp2p_solana.so")
}

pub fn pda(seeds: &[&[u8]]) -> anchor_lang::prelude::Pubkey {
    anchor_lang::prelude::Pubkey::find_program_address(seeds, &zkp2p_solana::ID).0
}

pub fn authorize_program_upgrade(
    svm: &mut LiteSVM,
    authority: Address,
) -> anchor_lang::prelude::Pubkey {
    let loader = anchor_lang::solana_program::bpf_loader_upgradeable::ID;
    let program_data =
        anchor_lang::prelude::Pubkey::find_program_address(&[zkp2p_solana::ID.as_ref()], &loader).0;
    let mut account = svm
        .get_account(&address(program_data.to_bytes()))
        .expect("programdata account");
    let state = UpgradeableLoaderState::ProgramData {
        slot: 0,
        upgrade_authority_address: Some(
            authority.to_string().parse().expect("interface authority"),
        ),
    };
    bincode::serialize_into(
        account
            .data
            .get_mut(..UpgradeableLoaderState::size_of_programdata_metadata())
            .expect("programdata metadata"),
        &state,
    )
    .expect("set test upgrade authority");
    svm.set_account(address(program_data.to_bytes()), account)
        .expect("update programdata account");
    program_data
}

pub fn install_token_account(
    svm: &mut LiteSVM,
    key: anchor_lang::prelude::Pubkey,
    mint: anchor_lang::prelude::Pubkey,
    owner: anchor_lang::prelude::Pubkey,
    amount: u64,
) {
    let mut data = vec![0_u8; 165];
    data.get_mut(..32)
        .expect("token mint field")
        .copy_from_slice(mint.as_ref());
    data.get_mut(32..64)
        .expect("token owner field")
        .copy_from_slice(owner.as_ref());
    data.get_mut(64..72)
        .expect("token amount field")
        .copy_from_slice(&amount.to_le_bytes());
    *data.get_mut(108).expect("token state field") = 1;
    svm.set_account(
        address(key.to_bytes()),
        Account {
            lamports: 10_000_000,
            data,
            owner: address(anchor_spl::token::ID.to_bytes()),
            executable: false,
            rent_epoch: 0,
        },
    )
    .expect("install token account");
}

pub fn token_amount(svm: &LiteSVM, key: anchor_lang::prelude::Pubkey) -> u64 {
    let account = svm
        .get_account(&address(key.to_bytes()))
        .expect("token account exists");
    let bytes: [u8; 8] = account
        .data
        .get(64..72)
        .expect("token amount field")
        .try_into()
        .expect("token amount width");
    u64::from_le_bytes(bytes)
}

pub fn set_token_amount(svm: &mut LiteSVM, key: anchor_lang::prelude::Pubkey, amount: u64) {
    let mut account = svm
        .get_account(&address(key.to_bytes()))
        .expect("token account exists");
    account
        .data
        .get_mut(64..72)
        .expect("token amount field")
        .copy_from_slice(&amount.to_le_bytes());
    svm.set_account(address(key.to_bytes()), account)
        .expect("update token amount");
}

pub fn send(
    svm: &mut LiteSVM,
    payer: &Keypair,
    additional_signers: &[&Keypair],
    instructions: &[Instruction],
) -> Result<(), String> {
    send_with_compute(svm, payer, additional_signers, instructions).map(|_| ())
}

pub fn send_with_compute(
    svm: &mut LiteSVM,
    payer: &Keypair,
    additional_signers: &[&Keypair],
    instructions: &[Instruction],
) -> Result<u64, String> {
    let blockhash = svm.latest_blockhash();
    let message = Message::new_with_blockhash(instructions, Some(&payer.pubkey()), &blockhash);
    let mut signers = Vec::with_capacity(
        additional_signers
            .len()
            .checked_add(1)
            .expect("test signer count fits"),
    );
    signers.push(payer);
    signers.extend_from_slice(additional_signers);
    svm.send_transaction(Transaction::new(&signers, message, blockhash))
        .map(|metadata| metadata.compute_units_consumed)
        .map_err(|error| format!("{error:?}"))
}

pub fn v0_transaction_size(payer: &Keypair, instruction: &Instruction) -> Result<u64, String> {
    let mut addresses = Vec::new();
    for account in &instruction.accounts {
        if !account.is_signer && !addresses.contains(&account.pubkey) {
            addresses.push(account.pubkey);
        }
    }
    let lookup = AddressLookupTableAccount {
        key: Address::new_unique(),
        addresses,
    };
    let message = v0::Message::try_compile(
        &payer.pubkey(),
        std::slice::from_ref(instruction),
        &[lookup],
        Default::default(),
    )
    .map_err(|error| format!("compile v0 transaction: {error}"))?;
    let transaction = VersionedTransaction::try_new(VersionedMessage::V0(message), &[payer])
        .map_err(|error| format!("sign v0 transaction: {error}"))?;
    bincode::serialized_size(&transaction)
        .map_err(|error| format!("serialize v0 transaction: {error}"))
}

pub struct Fixture {
    pub svm: LiteSVM,
    pub authority: Keypair,
    pub mint: anchor_lang::prelude::Pubkey,
    pub protocol: anchor_lang::prelude::Pubkey,
    pub escrow: anchor_lang::prelude::Pubkey,
    pub verifier: anchor_lang::prelude::Pubkey,
    pub orchestrator: anchor_lang::prelude::Pubkey,
    pub stake_vault: anchor_lang::prelude::Pubkey,
    pub rate_manager_config: anchor_lang::prelude::Pubkey,
    pub whitelist_config: anchor_lang::prelude::Pubkey,
    pub dispute_config: anchor_lang::prelude::Pubkey,
}

impl Fixture {
    pub fn new() -> Self {
        let mut svm = LiteSVM::new();
        svm.add_program_from_file(address(zkp2p_solana::ID.to_bytes()), program_binary())
            .expect("load SBF program");
        let authority = Keypair::new_from_array([1_u8; 32]);
        let program_data = authorize_program_upgrade(&mut svm, authority.pubkey());
        svm.airdrop(&authority.pubkey(), 50_000_000_000)
            .expect("fund authority");

        let mint_address = Address::new_unique();
        let mint = anchor_pubkey(mint_address);
        let mut mint_data = vec![0_u8; 82];
        *mint_data.get_mut(44).expect("mint decimals") = 6;
        *mint_data.get_mut(45).expect("mint initialized") = 1;
        svm.set_account(
            mint_address,
            Account {
                lamports: 10_000_000,
                data: mint_data,
                owner: address(anchor_spl::token::ID.to_bytes()),
                executable: false,
                rent_epoch: 0,
            },
        )
        .expect("install mint");

        let fixture = Self {
            svm,
            authority,
            mint,
            protocol: pda(&[PROTOCOL_SEED]),
            escrow: pda(&[ESCROW_CONFIG_SEED]),
            verifier: pda(&[VERIFIER_CONFIG_SEED]),
            orchestrator: pda(&[ORCHESTRATOR_CONFIG_SEED]),
            stake_vault: pda(&[STAKE_VAULT_CONFIG_SEED]),
            rate_manager_config: pda(&[RATE_MANAGER_CONFIG_SEED]),
            whitelist_config: pda(&[WHITELIST_CONFIG_SEED]),
            dispute_config: pda(&[DISPUTE_CONFIG_SEED]),
        };
        let initialize = Instruction {
            program_id: address(zkp2p_solana::ID.to_bytes()),
            accounts: zkp2p_solana::accounts::InitializeProtocol {
                authority: anchor_pubkey(fixture.authority.pubkey()),
                program: zkp2p_solana::ID,
                program_data,
                protocol: fixture.protocol,
                stake_mint: fixture.mint,
                escrow_config: fixture.escrow,
                verifier_config: fixture.verifier,
                orchestrator_config: fixture.orchestrator,
                stake_vault_config: fixture.stake_vault,
                rate_manager_config: fixture.rate_manager_config,
                whitelist_config: fixture.whitelist_config,
                dispute_config: fixture.dispute_config,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: zkp2p_solana::instruction::InitializeProtocol {
                args: InitializeProtocolArgs {
                    domain_chain_id: 1,
                    protocol_fee: 10_000_000_000_000_000,
                    protocol_fee_recipient: anchor_pubkey(fixture.authority.pubkey()),
                    intent_expiration_period: 1_800,
                    max_intents_per_deposit: 20,
                    controller_change_delay: 86_400,
                    initial_witnesses: vec![[7; 20]],
                    required_signatures: 1,
                },
            }
            .data(),
        };
        let mut fixture = fixture;
        send(&mut fixture.svm, &fixture.authority, &[], &[initialize])
            .expect("initialize protocol");
        fixture
    }

    pub fn fund(&mut self, keypair: &Keypair) {
        self.svm
            .airdrop(&keypair.pubkey(), 5_000_000_000)
            .expect("fund fixture signer");
    }

    pub fn send(&mut self, instructions: &[Instruction]) -> Result<(), String> {
        send(&mut self.svm, &self.authority, &[], instructions)
    }

    pub fn program_instruction<T, A>(&self, accounts: A, data: T) -> Instruction
    where
        T: InstructionData,
        A: ToAccountMetas,
    {
        Instruction {
            program_id: address(zkp2p_solana::ID.to_bytes()),
            accounts: accounts.to_account_metas(None),
            data: data.data(),
        }
    }
}
