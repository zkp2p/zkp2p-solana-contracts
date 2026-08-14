//! AddressGroupRegistry and whitelist-policy instructions.

use anchor_lang::prelude::*;
use solana_keccak_hasher as keccak;

use crate::{
    constants::{
        ADDRESS_GROUP_SEED, DEPOSIT_WHITELIST_MEMBER_SEED, DEPOSIT_WHITELIST_SEED,
        GROUP_MEMBER_SEED, MAX_GROUPS_PER_DEPOSIT, WHITELIST_CONFIG_SEED,
    },
    error::Zkp2pError,
    state::{
        AddressGroup, Deposit, DepositWhitelist, DepositWhitelistMember, GroupMember,
        WhitelistConfig,
    },
};

/// Accounts for permissionlessly creating one curated address group.
#[derive(Accounts)]
pub struct CreateAddressGroup<'info> {
    /// Initial curator and rent payer.
    #[account(mut)]
    pub curator: Signer<'info>,
    /// Address-group component state.
    #[account(mut, seeds = [WHITELIST_CONFIG_SEED], bump = whitelist_config.bump)]
    pub whitelist_config: Account<'info, WhitelistConfig>,
    /// New monotonic group PDA.
    #[account(
        init,
        payer = curator,
        space = 8 + AddressGroup::INIT_SPACE,
        seeds = [ADDRESS_GROUP_SEED, whitelist_config.key().as_ref(), &whitelist_config.next_group_id.to_le_bytes()],
        bump
    )]
    pub group: Account<'info, AddressGroup>,
    /// System program.
    pub system_program: Program<'info, System>,
}

/// Creates a group with deterministic identity and two-step curator authority.
pub fn handle_create_address_group(
    ctx: Context<CreateAddressGroup>,
    name: String,
    public: bool,
) -> Result<()> {
    require!(!name.is_empty(), Zkp2pError::ZeroValue);
    require!(name.len() <= 64, Zkp2pError::AmountAboveMaximum);
    let nonce = ctx.accounts.whitelist_config.next_group_id;
    let id = keccak::hashv(&[
        ctx.accounts.whitelist_config.key().as_ref(),
        &nonce.to_le_bytes(),
    ])
    .to_bytes();
    let group = &mut ctx.accounts.group;
    group.whitelist_config = ctx.accounts.whitelist_config.key();
    group.nonce = nonce;
    group.id = id;
    group.curator = ctx.accounts.curator.key();
    group.pending_curator = None;
    group.public = public;
    group.resolver = None;
    group.name = name;
    group.bump = ctx.bumps.group;
    ctx.accounts.whitelist_config.next_group_id =
        nonce.checked_add(1).ok_or(Zkp2pError::ArithmeticOverflow)?;
    Ok(())
}

/// Accounts for curator-controlled group configuration.
#[derive(Accounts)]
pub struct ConfigureAddressGroup<'info> {
    /// Current curator.
    #[account(address = group.curator)]
    pub curator: Signer<'info>,
    /// Group state.
    #[account(
        mut,
        seeds = [ADDRESS_GROUP_SEED, group.whitelist_config.as_ref(), &group.nonce.to_le_bytes()],
        bump = group.bump
    )]
    pub group: Account<'info, AddressGroup>,
    /// Resolver program supplied only when installing a nonzero resolver.
    /// CHECK: Handler validates exact key and executable status.
    pub resolver_program: Option<UncheckedAccount<'info>>,
}

/// Updates public admission, resolver, or starts/cancels curator handover.
pub fn handle_configure_address_group(
    ctx: Context<ConfigureAddressGroup>,
    public: Option<bool>,
    resolver: Option<Option<Pubkey>>,
    pending_curator: Option<Option<Pubkey>>,
) -> Result<()> {
    let group = &mut ctx.accounts.group;
    if let Some(value) = public {
        group.public = value;
    }
    if let Some(value) = resolver {
        if let Some(key) = value {
            require!(key != Pubkey::default(), Zkp2pError::ZeroAddress);
            let program = ctx
                .accounts
                .resolver_program
                .as_ref()
                .ok_or(Zkp2pError::Unauthorized)?;
            require_keys_eq!(program.key(), key, Zkp2pError::Unauthorized);
            require!(program.executable, Zkp2pError::Unauthorized);
        } else {
            require!(
                ctx.accounts.resolver_program.is_none(),
                Zkp2pError::Unauthorized
            );
        }
        group.resolver = value;
    } else {
        require!(
            ctx.accounts.resolver_program.is_none(),
            Zkp2pError::Unauthorized
        );
    }
    if let Some(value) = pending_curator {
        if let Some(key) = value {
            require!(
                key != Pubkey::default() && key != group.curator,
                Zkp2pError::ZeroAddress
            );
        }
        group.pending_curator = value;
    }
    Ok(())
}

/// Accounts for accepting a pending curator handover.
#[derive(Accounts)]
pub struct AcceptGroupCurator<'info> {
    /// Proposed curator.
    pub pending_curator: Signer<'info>,
    /// Group state.
    #[account(
        mut,
        seeds = [ADDRESS_GROUP_SEED, group.whitelist_config.as_ref(), &group.nonce.to_le_bytes()],
        bump = group.bump,
        constraint = group.pending_curator == Some(pending_curator.key()) @ Zkp2pError::Unauthorized
    )]
    pub group: Account<'info, AddressGroup>,
}

/// Completes two-step curator rotation.
pub fn handle_accept_group_curator(ctx: Context<AcceptGroupCurator>) -> Result<()> {
    ctx.accounts.group.curator = ctx.accounts.pending_curator.key();
    ctx.accounts.group.pending_curator = None;
    Ok(())
}

/// Accounts for curator-controlled explicit membership.
#[derive(Accounts)]
pub struct SetGroupMember<'info> {
    /// Current curator and rent payer.
    #[account(mut, address = group.curator)]
    pub curator: Signer<'info>,
    /// Group state.
    #[account(
        seeds = [ADDRESS_GROUP_SEED, group.whitelist_config.as_ref(), &group.nonce.to_le_bytes()],
        bump = group.bump
    )]
    pub group: Account<'info, AddressGroup>,
    /// Subject account.
    /// CHECK: Identity only; no data is read.
    pub member_address: UncheckedAccount<'info>,
    /// Explicit membership PDA.
    #[account(
        init_if_needed,
        payer = curator,
        space = 8 + GroupMember::INIT_SPACE,
        seeds = [GROUP_MEMBER_SEED, group.key().as_ref(), member_address.key().as_ref()],
        bump
    )]
    pub member: Account<'info, GroupMember>,
    /// System program.
    pub system_program: Program<'info, System>,
}

/// Adds or removes one explicit group member.
pub fn handle_set_group_member(ctx: Context<SetGroupMember>, active: bool) -> Result<()> {
    require!(
        ctx.accounts.member_address.key() != Pubkey::default(),
        Zkp2pError::ZeroAddress
    );
    let member = &mut ctx.accounts.member;
    member.group = ctx.accounts.group.key();
    member.member = ctx.accounts.member_address.key();
    member.active = active;
    member.bump = ctx.bumps.member;
    Ok(())
}

/// Accounts for public self-service group membership.
#[derive(Accounts)]
pub struct SetSelfGroupMember<'info> {
    /// Joining or leaving member and rent payer.
    #[account(mut)]
    pub member_address: Signer<'info>,
    /// Public group.
    #[account(
        seeds = [ADDRESS_GROUP_SEED, group.whitelist_config.as_ref(), &group.nonce.to_le_bytes()],
        bump = group.bump,
        constraint = group.public @ Zkp2pError::Unauthorized
    )]
    pub group: Account<'info, AddressGroup>,
    /// Membership PDA.
    #[account(
        init_if_needed,
        payer = member_address,
        space = 8 + GroupMember::INIT_SPACE,
        seeds = [GROUP_MEMBER_SEED, group.key().as_ref(), member_address.key().as_ref()],
        bump
    )]
    pub member: Account<'info, GroupMember>,
    /// System program.
    pub system_program: Program<'info, System>,
}

/// Joins or leaves a public group.
pub fn handle_set_self_group_member(ctx: Context<SetSelfGroupMember>, active: bool) -> Result<()> {
    let member = &mut ctx.accounts.member;
    member.group = ctx.accounts.group.key();
    member.member = ctx.accounts.member_address.key();
    member.active = active;
    member.bump = ctx.bumps.member;
    Ok(())
}

/// Accounts for creating one deposit's persistent whitelist policy.
#[derive(Accounts)]
pub struct InitializeDepositWhitelist<'info> {
    /// Deposit owner or delegate and rent payer.
    #[account(mut)]
    pub authority: Signer<'info>,
    /// Existing deposit.
    #[account(
        constraint = deposit.depositor == authority.key() || deposit.delegate == Some(authority.key()) @ Zkp2pError::Unauthorized
    )]
    pub deposit: Account<'info, Deposit>,
    /// New policy PDA.
    #[account(
        init,
        payer = authority,
        space = 8 + DepositWhitelist::INIT_SPACE,
        seeds = [DEPOSIT_WHITELIST_SEED, deposit.key().as_ref()],
        bump
    )]
    pub deposit_whitelist: Account<'info, DepositWhitelist>,
    /// System program.
    pub system_program: Program<'info, System>,
}

/// Initializes a latest-stack whitelist; legacy bootstrap is permanently consumed.
pub fn handle_initialize_deposit_whitelist(
    ctx: Context<InitializeDepositWhitelist>,
    enabled: bool,
) -> Result<()> {
    let whitelist = &mut ctx.accounts.deposit_whitelist;
    whitelist.deposit = ctx.accounts.deposit.key();
    whitelist.enabled = enabled;
    whitelist.bootstrapped = true;
    whitelist.allowed_groups = Vec::new();
    whitelist.bump = ctx.bumps.deposit_whitelist;
    Ok(())
}

/// Accounts for changing one existing deposit whitelist.
#[derive(Accounts)]
pub struct ConfigureDepositWhitelist<'info> {
    /// Deposit owner or delegate.
    pub authority: Signer<'info>,
    /// Existing deposit.
    #[account(
        constraint = deposit.depositor == authority.key() || deposit.delegate == Some(authority.key()) @ Zkp2pError::Unauthorized
    )]
    pub deposit: Account<'info, Deposit>,
    /// Existing whitelist policy.
    #[account(
        mut,
        seeds = [DEPOSIT_WHITELIST_SEED, deposit.key().as_ref()],
        bump = deposit_whitelist.bump,
        constraint = deposit_whitelist.deposit == deposit.key() @ Zkp2pError::DepositNotFound
    )]
    pub deposit_whitelist: Account<'info, DepositWhitelist>,
}

/// Enables or disables whitelist enforcement.
pub fn handle_set_whitelist_enabled(
    ctx: Context<ConfigureDepositWhitelist>,
    enabled: bool,
) -> Result<()> {
    ctx.accounts.deposit_whitelist.enabled = enabled;
    Ok(())
}

/// Accounts for adding or removing one validated group from a deposit policy.
#[derive(Accounts)]
pub struct SetDepositAllowedGroup<'info> {
    /// Deposit owner or delegate.
    pub authority: Signer<'info>,
    /// Existing deposit.
    #[account(
        constraint = deposit.depositor == authority.key() || deposit.delegate == Some(authority.key()) @ Zkp2pError::Unauthorized
    )]
    pub deposit: Account<'info, Deposit>,
    /// Existing whitelist policy.
    #[account(
        mut,
        seeds = [DEPOSIT_WHITELIST_SEED, deposit.key().as_ref()],
        bump = deposit_whitelist.bump
    )]
    pub deposit_whitelist: Account<'info, DepositWhitelist>,
    /// Existing group whose ID is being configured.
    #[account(
        seeds = [ADDRESS_GROUP_SEED, group.whitelist_config.as_ref(), &group.nonce.to_le_bytes()],
        bump = group.bump
    )]
    pub group: Account<'info, AddressGroup>,
}

/// Adds or removes one group while preserving uniqueness and the ten-group cap.
pub fn handle_set_deposit_allowed_group(
    ctx: Context<SetDepositAllowedGroup>,
    allowed: bool,
) -> Result<()> {
    let group_id = ctx.accounts.group.id;
    let groups = &mut ctx.accounts.deposit_whitelist.allowed_groups;
    let position = groups.iter().position(|candidate| candidate == &group_id);
    match (allowed, position) {
        (true, None) => {
            require!(
                groups.len() < MAX_GROUPS_PER_DEPOSIT,
                Zkp2pError::TooManyGroups
            );
            groups.push(group_id);
        }
        (false, Some(index)) => {
            groups.remove(index);
        }
        _ => return err!(Zkp2pError::AlreadyInState),
    }
    Ok(())
}

/// Accounts for one direct deposit whitelist membership.
#[derive(Accounts)]
pub struct SetDepositWhitelistMember<'info> {
    /// Deposit owner or delegate and rent payer.
    #[account(mut)]
    pub authority: Signer<'info>,
    /// Existing deposit.
    #[account(
        constraint = deposit.depositor == authority.key() || deposit.delegate == Some(authority.key()) @ Zkp2pError::Unauthorized
    )]
    pub deposit: Account<'info, Deposit>,
    /// Existing whitelist policy.
    #[account(
        seeds = [DEPOSIT_WHITELIST_SEED, deposit.key().as_ref()],
        bump = deposit_whitelist.bump
    )]
    pub deposit_whitelist: Account<'info, DepositWhitelist>,
    /// Membership subject.
    /// CHECK: Identity only; no data is read.
    pub taker: UncheckedAccount<'info>,
    /// Direct membership PDA.
    #[account(
        init_if_needed,
        payer = authority,
        space = 8 + DepositWhitelistMember::INIT_SPACE,
        seeds = [DEPOSIT_WHITELIST_MEMBER_SEED, deposit_whitelist.key().as_ref(), taker.key().as_ref()],
        bump
    )]
    pub member: Account<'info, DepositWhitelistMember>,
    /// System program.
    pub system_program: Program<'info, System>,
}

/// Adds or removes one direct taker membership.
pub fn handle_set_deposit_whitelist_member(
    ctx: Context<SetDepositWhitelistMember>,
    active: bool,
) -> Result<()> {
    require!(
        ctx.accounts.taker.key() != Pubkey::default(),
        Zkp2pError::ZeroAddress
    );
    let member = &mut ctx.accounts.member;
    member.deposit_whitelist = ctx.accounts.deposit_whitelist.key();
    member.taker = ctx.accounts.taker.key();
    member.active = active;
    member.bump = ctx.bumps.member;
    Ok(())
}
