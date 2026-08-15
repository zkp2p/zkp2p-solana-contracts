//! LiteSVM execution tests for canonical initialization and governance transitions.

use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use litesvm::LiteSVM;
use solana_account::Account;
use solana_address::Address;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;
use zkp2p_solana::{
    constants::{
        DISPUTE_CONFIG_SEED, ESCROW_CONFIG_SEED, ORCHESTRATOR_CONFIG_SEED, PROTOCOL_SEED,
        RATE_MANAGER_CONFIG_SEED, STAKE_VAULT_CONFIG_SEED, VERIFIER_CONFIG_SEED,
        WHITELIST_CONFIG_SEED,
    },
    InitializeProtocolArgs, ProtocolConfig,
};

use super::common::{address, authorize_program_upgrade, program_binary};

fn pda(seed: &[u8]) -> anchor_lang::prelude::Pubkey {
    anchor_lang::prelude::Pubkey::find_program_address(&[seed], &zkp2p_solana::ID).0
}

fn send(svm: &mut LiteSVM, payer: &Keypair, instruction: Instruction) -> Result<(), String> {
    let blockhash = svm.latest_blockhash();
    let message = Message::new_with_blockhash(&[instruction], Some(&payer.pubkey()), &blockhash);
    svm.send_transaction(Transaction::new(&[payer], message, blockhash))
        .map(|_| ())
        .map_err(|error| format!("{error:?}"))
}

#[test]
fn initializes_only_latest_components_and_transfers_governance_two_step() {
    let mut svm = LiteSVM::new();
    svm.add_program_from_file(address(zkp2p_solana::ID.to_bytes()), program_binary())
        .expect("load SBF program");
    let authority = Keypair::new();
    let program_data = authorize_program_upgrade(&mut svm, authority.pubkey());
    let attacker = Keypair::new();
    svm.airdrop(&authority.pubkey(), 50_000_000_000)
        .expect("fund authority");
    svm.airdrop(&attacker.pubkey(), 50_000_000_000)
        .expect("fund attacker");

    let mint = Address::new_unique();
    let mut mint_data = vec![0_u8; 82];
    *mint_data.get_mut(44).expect("mint decimals") = 6;
    *mint_data.get_mut(45).expect("mint initialized") = 1;
    svm.set_account(
        mint,
        Account {
            lamports: 10_000_000,
            data: mint_data,
            owner: address(anchor_spl::token::ID.to_bytes()),
            executable: false,
            rent_epoch: 0,
        },
    )
    .expect("install mint");

    let protocol = pda(PROTOCOL_SEED);
    let escrow = pda(ESCROW_CONFIG_SEED);
    let verifier = pda(VERIFIER_CONFIG_SEED);
    let orchestrator = pda(ORCHESTRATOR_CONFIG_SEED);
    let stake_vault = pda(STAKE_VAULT_CONFIG_SEED);
    let rate_manager = pda(RATE_MANAGER_CONFIG_SEED);
    let whitelist = pda(WHITELIST_CONFIG_SEED);
    let dispute = pda(DISPUTE_CONFIG_SEED);
    let accounts = zkp2p_solana::accounts::InitializeProtocol {
        authority: address(authority.pubkey().to_bytes()),
        program: zkp2p_solana::ID,
        program_data,
        protocol,
        stake_mint: anchor_lang::prelude::Pubkey::new_from_array(mint.to_bytes()),
        escrow_config: escrow,
        verifier_config: verifier,
        orchestrator_config: orchestrator,
        stake_vault_config: stake_vault,
        rate_manager_config: rate_manager,
        whitelist_config: whitelist,
        dispute_config: dispute,
        system_program: anchor_lang::system_program::ID,
    };
    let initialize_data = zkp2p_solana::instruction::InitializeProtocol {
        args: InitializeProtocolArgs {
            domain_chain_id: 1,
            protocol_fee: 10_000_000_000_000_000,
            protocol_fee_recipient: anchor_lang::prelude::Pubkey::new_from_array(
                authority.pubkey().to_bytes(),
            ),
            intent_expiration_period: 1_800,
            max_intents_per_deposit: 20,
            controller_change_delay: 86_400,
            initial_witnesses: vec![[7; 20]],
            required_signatures: 1,
        },
    }
    .data();
    let mut attacker_metas = accounts.to_account_metas(None);
    attacker_metas
        .first_mut()
        .expect("initializer authority meta")
        .pubkey = attacker.pubkey();
    let attacker_initialize = Instruction {
        program_id: address(zkp2p_solana::ID.to_bytes()),
        accounts: attacker_metas,
        data: initialize_data.clone(),
    };
    assert!(send(&mut svm, &attacker, attacker_initialize).is_err());
    assert!(svm.get_account(&address(protocol.to_bytes())).is_none());

    let initialize = Instruction {
        program_id: address(zkp2p_solana::ID.to_bytes()),
        accounts: accounts.to_account_metas(None),
        data: initialize_data,
    };
    send(&mut svm, &authority, initialize).expect("initialize protocol");

    for component in [
        protocol,
        escrow,
        verifier,
        orchestrator,
        stake_vault,
        rate_manager,
        whitelist,
        dispute,
    ] {
        assert!(svm.get_account(&address(component.to_bytes())).is_some());
    }
    let protocol_account = svm
        .get_account(&address(protocol.to_bytes()))
        .expect("protocol account");
    let mut data = protocol_account.data.as_slice();
    let root = ProtocolConfig::try_deserialize(&mut data).expect("decode protocol");
    assert_eq!(root.authority.to_bytes(), authority.pubkey().to_bytes());

    let pending = Keypair::new();
    svm.airdrop(&pending.pubkey(), 1_000_000_000)
        .expect("fund pending authority");
    let propose = Instruction {
        program_id: address(zkp2p_solana::ID.to_bytes()),
        accounts: zkp2p_solana::accounts::ProposeProtocolAuthority {
            authority: anchor_lang::prelude::Pubkey::new_from_array(authority.pubkey().to_bytes()),
            protocol,
        }
        .to_account_metas(None),
        data: zkp2p_solana::instruction::ProposeProtocolAuthority {
            pending: Some(anchor_lang::prelude::Pubkey::new_from_array(
                pending.pubkey().to_bytes(),
            )),
        }
        .data(),
    };
    send(&mut svm, &authority, propose).expect("propose authority");

    let accept = Instruction {
        program_id: address(zkp2p_solana::ID.to_bytes()),
        accounts: zkp2p_solana::accounts::AcceptProtocolAuthority {
            pending_authority: anchor_lang::prelude::Pubkey::new_from_array(
                pending.pubkey().to_bytes(),
            ),
            protocol,
        }
        .to_account_metas(None),
        data: zkp2p_solana::instruction::AcceptProtocolAuthority {}.data(),
    };
    send(&mut svm, &pending, accept).expect("accept authority");

    let protocol_account = svm
        .get_account(&address(protocol.to_bytes()))
        .expect("protocol account after handover");
    let mut data = protocol_account.data.as_slice();
    let root = ProtocolConfig::try_deserialize(&mut data).expect("decode transferred protocol");
    assert_eq!(root.authority.to_bytes(), pending.pubkey().to_bytes());
    assert_eq!(root.pending_authority, None);
}
