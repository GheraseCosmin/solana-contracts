use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, TransferChecked};

declare_id!("ZnxPrdCiNFeCA79TVCrx5v57CkftWL3yS3LxmToK4UK");

pub fn economy_estimate_rewards(
    total_staked_tokens: u64,
    user_staked_tokens: u64,
    total_rewards: u64,
) -> u64 {
    // parse those into u128 to avoid overflow
    let user_staked_tokens_u128 = user_staked_tokens as u128;
    let total_rewards_u128 = total_rewards as u128;
    let total_staked_tokens_u128 = total_staked_tokens as u128;

    let final_result_u128 =
        (user_staked_tokens_u128 * total_rewards_u128) / total_staked_tokens_u128;

    let final_result_u64 = final_result_u128 as u64;

    final_result_u64
}

#[program]
pub mod staking {
    use super::*;

    // ********* START POOL CREATOR FUNCTIONS **************
    /// Create a new staking pool. Any user can create a pool and becomes its authority.
    pub fn create_pool(
        ctx: Context<CreatePool>,
        pool_id: u64,
        initial_funding_amount: u64,
        claim_cooldown: i64,
    ) -> Result<()> {
        let pool = &mut ctx.accounts.pool;

        // Configure bumps
        let bump = ctx.bumps.pool;
        pool.bump = bump;

        // Configure authority and identity
        pool.pool_id = pool_id;
        pool.creator = *ctx.accounts.creator.key;

        // Set default pool values
        pool.current_tokens_staked = 0;
        pool.current_rewards = initial_funding_amount;
        pool.claim_cooldown = claim_cooldown;
        pool.emergency_mode_enabled = false;

        // Send the tokens from the creator to the pool if initial funding is provided
        if initial_funding_amount > 0 {
            token::transfer_checked(
                ctx.accounts.into_transfer_to_pda_context(),
                initial_funding_amount,
                ctx.accounts.mint.decimals,
            )?;
        }

        Ok(())
    }

    /// Fund rewards pool. Only the pool creator can fund their pool.
    pub fn fund_pool(ctx: Context<UpdatePool>, amount: u64) -> Result<()> {
        let pool = &mut ctx.accounts.pool;

        // Verify the signer is the pool creator
        require!(
            pool.creator == *ctx.accounts.creator.key,
            StakingError::UnauthorizedPoolAccess
        );

        pool.current_rewards += amount;

        // Send the tokens from the creator to the pool
        token::transfer_checked(
            ctx.accounts.into_transfer_to_pda_context(),
            amount,
            ctx.accounts.mint.decimals,
        )?;

        Ok(())
    }

    /// Enable emergency mode where people can withdraw their tokens and the pool creator can withdraw the rewards.
    /// Only the pool creator can enable emergency mode.
    pub fn enable_emergency_mode(ctx: Context<UpdatePool>) -> Result<()> {
        let pool = &mut ctx.accounts.pool;

        // Verify the signer is the pool creator
        require!(
            pool.creator == *ctx.accounts.creator.key,
            StakingError::UnauthorizedPoolAccess
        );

        // Require the mode to have been disabled
        require!(
            !pool.emergency_mode_enabled,
            StakingError::EmergencyModeAlreadyEnabled
        );
        pool.emergency_mode_enabled = true;

        Ok(())
    }

    /// Change pool cooldown period. Only affects new cooldowns.
    /// Only the pool creator can change the cooldown.
    pub fn change_pool_cooldown(ctx: Context<UpdatePool>, new_cooldown: i64) -> Result<()> {
        let pool = &mut ctx.accounts.pool;

        // Verify the signer is the pool creator
        require!(
            pool.creator == *ctx.accounts.creator.key,
            StakingError::UnauthorizedPoolAccess
        );

        pool.claim_cooldown = new_cooldown;

        Ok(())
    }

    // ********* END POOL CREATOR FUNCTIONS **************

    /// Create a staker deposit in a pool.
    pub fn stake(
        ctx: Context<CreateDeposit>,
        deposit_id: u64,
        deposit_amount: u64,
    ) -> Result<()> {
        let deposit = &mut ctx.accounts.deposit;
        let staker_stats = &mut ctx.accounts.staker_stats;
        let pool = &mut ctx.accounts.pool;

        let now = Clock::get()?.unix_timestamp;

        // Depositing tokens is only allowed if the pool is not in emergency mode
        require!(
            pool.emergency_mode_enabled == false,
            StakingError::EmergencyModeEnabled
        );

        deposit.deposit_id = deposit_id;
        deposit.tokens_deposited = deposit_amount;
        deposit.tokens_claimed = 0;
        deposit.unlock_timestamp = now + pool.claim_cooldown;
        deposit.is_withdrawn = false;
        deposit.is_cooldown_active = false;
        deposit.bump = ctx.bumps.deposit;

        // Update stats
        staker_stats.staker = *ctx.accounts.staker.key;
        staker_stats.total_staked += deposit_amount;
        staker_stats.bump = ctx.bumps.staker_stats;

        // Update the pool
        pool.current_tokens_staked += deposit_amount;

        // Send the tokens from the staker to the pool
        token::transfer_checked(
            ctx.accounts.into_transfer_to_pda_context(),
            deposit_amount,
            ctx.accounts.mint.decimals,
        )?;

        Ok(())
    }

    /// Activate cooldown for a deposit to enable unstaking.
    pub fn activate_cooldown(
        ctx: Context<ActivateDepositCooldown>,
        _deposit_id: u64,
    ) -> Result<()> {
        let deposit = &mut ctx.accounts.deposit;
        let pool = &mut ctx.accounts.pool;
        let now = Clock::get()?.unix_timestamp;

        require!(
            deposit.is_withdrawn == false,
            StakingError::DepositAlreadyWithdrawn
        );

        require!(
            deposit.is_cooldown_active == false,
            StakingError::CooldownAlreadyActivated
        );

        deposit.is_cooldown_active = true;
        deposit.unlock_timestamp = now + pool.claim_cooldown;
        Ok(())
    }

    /// Unstake tokens from a pool after cooldown has elapsed.
    pub fn unstake(ctx: Context<UnstakeDeposit>, _deposit_id: u64) -> Result<()> {
        // Extract values from pool and deposit before mutable borrow
        let pool_creator = ctx.accounts.pool.creator;
        let pool_id = ctx.accounts.pool.pool_id;
        let pool_bump = ctx.accounts.pool.bump;
        let emergency_mode_enabled = ctx.accounts.pool.emergency_mode_enabled;
        let pool_total_staked_tokens = ctx.accounts.pool.current_tokens_staked;
        let pool_total_rewards_tokens = ctx.accounts.pool.current_rewards;

        let deposit_is_withdrawn = ctx.accounts.deposit.is_withdrawn;
        let deposit_is_cooldown_active = ctx.accounts.deposit.is_cooldown_active;
        let deposit_unlock_timestamp = ctx.accounts.deposit.unlock_timestamp;
        let user_total_staked_tokens = ctx.accounts.deposit.tokens_deposited;

        let signer_seeds: [&[&[u8]]; 1] = [&[
            b"pool",
            pool_creator.as_ref(),
            &pool_id.to_le_bytes()[..],
            &[pool_bump],
        ]];

        let now = Clock::get()?.unix_timestamp;

        // If the pool has emergency mode turned on, we can ignore the time.
        require!(
            emergency_mode_enabled == false,
            StakingError::EmergencyModeEnabled
        );

        // Require the deposit to not be withdrawn
        require!(
            deposit_is_withdrawn == false,
            StakingError::DepositAlreadyWithdrawn
        );

        require!(
            deposit_is_cooldown_active == true,
            StakingError::ClaimCooldownNotActive
        );

        // Require the user to have waited long enough to unstake
        require!(
            now >= deposit_unlock_timestamp,
            StakingError::ClaimCooldownNotElapsed
        );

        // Calculate the user's rewards based on their share of tokens in the total staked tokens
        let user_rewards = economy_estimate_rewards(
            pool_total_staked_tokens,
            user_total_staked_tokens,
            pool_total_rewards_tokens,
        );

        // Now get mutable borrows for updates
        let deposit = &mut ctx.accounts.deposit;
        let staker_stats = &mut ctx.accounts.staker_stats;
        let pool = &mut ctx.accounts.pool;

        // Mark the deposit as withdrawn
        deposit.is_withdrawn = true;

        // Set the claimed amount in the deposit
        deposit.tokens_claimed = user_rewards;

        // Update stats
        staker_stats.total_staked -= user_total_staked_tokens;

        // Remove the reward tokens from the pool
        pool.current_rewards -= user_rewards;

        // Subtract the user's tokens from the pool
        pool.current_tokens_staked -= user_total_staked_tokens;

        // Get mint decimals before using ctx.accounts
        let mint_decimals = ctx.accounts.mint.decimals;

        // Send their initial deposit back
        token::transfer_checked(
            ctx.accounts
                .into_withdraw_context()
                .with_signer(&signer_seeds),
            user_total_staked_tokens,
            mint_decimals,
        )?;

        // Send the rewards from the pool to the staker
        token::transfer_checked(
            ctx.accounts
                .into_withdraw_context()
                .with_signer(&signer_seeds),
            user_rewards,
            mint_decimals,
        )?;

        Ok(())
    }

    /// Emergency unstake tokens (no rewards). Only works when pool is in emergency mode.
    pub fn unstake_emergency(
        ctx: Context<UnstakeDepositEmergency>,
        _deposit_id: u64,
    ) -> Result<()> {
        // Extract values before any borrows
        let pool_creator = ctx.accounts.pool.creator;
        let pool_id = ctx.accounts.pool.pool_id;
        let pool_bump = ctx.accounts.pool.bump;
        let tokens_deposited = ctx.accounts.deposit.tokens_deposited;
        let mint_decimals = ctx.accounts.mint.decimals;

        let signer_seeds: [&[&[u8]]; 1] = [&[
            b"pool",
            pool_creator.as_ref(),
            &pool_id.to_le_bytes()[..],
            &[pool_bump],
        ]];

        // Send their initial deposit back
        token::transfer_checked(
            ctx.accounts
                .into_withdraw_context()
                .with_signer(&signer_seeds),
            tokens_deposited,
            mint_decimals,
        )?;

        let pool_mut = &mut ctx.accounts.pool;
        let deposit = &mut ctx.accounts.deposit;
        let staker_stats = &mut ctx.accounts.staker_stats;

        let emergency_mode_enabled = pool_mut.emergency_mode_enabled;

        // If the pool has emergency mode turned off, fail
        require!(
            emergency_mode_enabled == true,
            StakingError::EmergencyModeNotEnabled
        );

        // Require the deposit to not be withdrawn
        require!(
            deposit.is_withdrawn == false,
            StakingError::DepositAlreadyWithdrawn
        );

        // Mark the deposit as withdrawn
        deposit.is_withdrawn = true;

        // Update stats
        staker_stats.total_staked -= deposit.tokens_deposited;

        // Subtract the user's tokens from the pool
        pool_mut.current_tokens_staked -= deposit.tokens_deposited;

        Ok(())
    }

    /// Emergency withdraw rewards. Only pool creator can withdraw rewards in emergency mode.
    pub fn withdraw_rewards_emergency(ctx: Context<WithdrawRewardsEmergency>) -> Result<()> {
        // Extract values from pool before mutable borrow
        let pool_creator = ctx.accounts.pool.creator;
        let pool_id = ctx.accounts.pool.pool_id;
        let pool_bump = ctx.accounts.pool.bump;
        let current_rewards_in_pool = ctx.accounts.pool.current_rewards;
        let emergency_mode_enabled = ctx.accounts.pool.emergency_mode_enabled;

        // Verify the signer is the pool creator
        require!(
            pool_creator == *ctx.accounts.creator.key,
            StakingError::UnauthorizedPoolAccess
        );

        require!(
            emergency_mode_enabled,
            StakingError::EmergencyModeNotEnabled
        );

        let signer_seeds: [&[&[u8]]; 1] = [&[
            b"pool",
            pool_creator.as_ref(),
            &pool_id.to_le_bytes()[..],
            &[pool_bump],
        ]];

        // Get mint decimals before using ctx.accounts
        let mint_decimals = ctx.accounts.mint.decimals;

        // Remove the reward tokens from the pool
        let pool = &mut ctx.accounts.pool;
        pool.current_rewards = 0;

        token::transfer_checked(
            ctx.accounts
                .into_withdraw_context()
                .with_signer(&signer_seeds),
            current_rewards_in_pool,
            mint_decimals,
        )?;

        Ok(())
    }
}

#[account]
pub struct StakingPool {
    pub pool_id: u64,                 // 8
    pub creator: Pubkey,              // 32
    pub current_tokens_staked: u64,   // 8
    pub current_rewards: u64,         // 8
    pub claim_cooldown: i64,          // 8
    pub emergency_mode_enabled: bool, // 1
    pub bump: u8,                     // 1
}

#[account]
pub struct StakerDeposit {
    pub deposit_id: u64,          // 8
    pub tokens_deposited: u64,    // 8
    pub tokens_claimed: u64,      // 8
    pub unlock_timestamp: i64,    // 8
    pub is_withdrawn: bool,       // 1
    pub is_cooldown_active: bool, // 1
    pub bump: u8,                 // 1
}

#[account]
pub struct StakerStats {
    pub staker: Pubkey,     // 32
    pub total_staked: u64,  // 8
    pub bump: u8,           // 1
}

#[derive(Accounts)]
#[instruction(pool_id: u64, initial_funding_amount: u64)]
pub struct CreatePool<'info> {
    pub mint: Account<'info, Mint>,
    #[account(mut)]
    pub creator: Signer<'info>,
    #[account(
        init,
        payer = creator,
        space = 8 + // anchor overhead
        8 + // pool_id
        32 + // creator
        8 + // current_tokens_staked
        8 + // current_rewards
        8 + // claim_cooldown
        1 + // emergency_mode_enabled
        1, // bump
        seeds = [b"pool", creator.key().as_ref(), &pool_id.to_le_bytes()],
        bump
    )]
    pub pool: Account<'info, StakingPool>,
    #[account(
        init,
        payer = creator,
        associated_token::mint = mint,
        associated_token::authority = pool
    )]
    pub pool_vault: Account<'info, TokenAccount>,
    #[account(
        mut,
        associated_token::mint = mint,
        associated_token::authority = creator
    )]
    pub creator_ata: Account<'info, TokenAccount>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

impl<'info> CreatePool<'info> {
    fn into_transfer_to_pda_context(
        &self,
    ) -> CpiContext<'_, '_, '_, 'info, TransferChecked<'info>> {
        let cpi_accounts = TransferChecked {
            from: self.creator_ata.to_account_info(),
            mint: self.mint.to_account_info(),
            to: self.pool_vault.to_account_info(),
            authority: self.creator.to_account_info(),
        };
        CpiContext::new(self.token_program.to_account_info(), cpi_accounts)
    }
}

#[derive(Accounts)]
#[instruction(deposit_id: u64)]
pub struct CreateDeposit<'info> {
    pub mint: Account<'info, Mint>,
    #[account(mut)]
    pub staker: Signer<'info>,
    #[account(
        init,
        payer = staker,
        space = 8 + // Anchor allocation
        8 + // deposit_id
        8 + // tokens_deposited
        8 + // tokens_claimed
        8 + // unlock_timestamp
        1 + // is_withdrawn
        1 + // is_cooldown_active
        1, // bump u8
        seeds = [
            b"deposit",
            staker.key().as_ref(),
            pool.key().as_ref(),
            &deposit_id.to_le_bytes(),
        ],
        bump
    )]
    pub deposit: Account<'info, StakerDeposit>,
    #[account(
        init_if_needed, 
        payer = staker,
        space = 8 + // Anchor allocation
        32 + // staker
        8 + // total_staked
        1, // bump u8
        seeds = [b"staker-stats", staker.key().as_ref()],
        bump
    )]
    pub staker_stats: Account<'info, StakerStats>,
    #[account(mut)]
    pub pool: Account<'info, StakingPool>,
    #[account(mut)]
    pub pool_vault: Account<'info, TokenAccount>,
    #[account(mut)]
    pub staker_ata: Account<'info, TokenAccount>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

impl<'info> CreateDeposit<'info> {
    fn into_transfer_to_pda_context(
        &self,
    ) -> CpiContext<'_, '_, '_, 'info, TransferChecked<'info>> {
        let cpi_accounts = TransferChecked {
            from: self.staker_ata.to_account_info(),
            mint: self.mint.to_account_info(),
            to: self.pool_vault.to_account_info(),
            authority: self.staker.to_account_info(),
        };
        CpiContext::new(self.token_program.to_account_info(), cpi_accounts)
    }
}

#[derive(Accounts)]
#[instruction(deposit_id: u64)]
pub struct UnstakeDeposit<'info> {
    pub mint: Account<'info, Mint>,
    #[account(mut)]
    pub staker: Signer<'info>,
    #[account(
        mut, 
        seeds = [
            b"deposit",
            staker.key().as_ref(),
            pool.key().as_ref(),
            &deposit_id.to_le_bytes(),
        ],
        bump = deposit.bump
    )]
    pub deposit: Account<'info, StakerDeposit>,
    #[account(
        mut, 
        seeds = [b"staker-stats", staker.key().as_ref()], 
        bump = staker_stats.bump
    )]
    pub staker_stats: Account<'info, StakerStats>,
    #[account(mut)]
    pub pool: Account<'info, StakingPool>,
    #[account(mut)]
    pub pool_vault: Account<'info, TokenAccount>,
    #[account(mut)]
    pub staker_ata: Account<'info, TokenAccount>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

impl<'info> UnstakeDeposit<'info> {
    fn into_withdraw_context(&self) -> CpiContext<'_, '_, '_, 'info, TransferChecked<'info>> {
        let cpi_accounts = TransferChecked {
            from: self.pool_vault.to_account_info(),
            mint: self.mint.to_account_info(),
            to: self.staker_ata.to_account_info(),
            authority: self.pool.to_account_info(),
        };
        CpiContext::new(self.token_program.to_account_info(), cpi_accounts)
    }
}

#[derive(Accounts)]
#[instruction(deposit_id: u64)]
pub struct ActivateDepositCooldown<'info> {
    #[account(mut)]
    pub staker: Signer<'info>,
    #[account(
        mut, 
        seeds = [
            b"deposit",
            staker.key().as_ref(),
            pool.key().as_ref(),
            &deposit_id.to_le_bytes(),
        ],
        bump = deposit.bump
    )]
    pub deposit: Account<'info, StakerDeposit>,
    #[account(mut)]
    pub pool: Account<'info, StakingPool>,
}

#[derive(Accounts)]
#[instruction(deposit_id: u64)]
pub struct UnstakeDepositEmergency<'info> {
    pub mint: Account<'info, Mint>,
    #[account(mut)]
    pub staker: Signer<'info>,
    #[account(
        mut, 
        seeds = [
            b"deposit",
            staker.key().as_ref(),
            pool.key().as_ref(),
            &deposit_id.to_le_bytes(),
        ],
        bump = deposit.bump
    )]
    pub deposit: Account<'info, StakerDeposit>,
    #[account(
        mut, 
        seeds = [b"staker-stats", staker.key().as_ref()], 
        bump = staker_stats.bump
    )]
    pub staker_stats: Account<'info, StakerStats>,
    #[account(mut)]
    pub pool: Account<'info, StakingPool>,
    #[account(mut)]
    pub pool_vault: Account<'info, TokenAccount>,
    #[account(mut)]
    pub staker_ata: Account<'info, TokenAccount>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

impl<'info> UnstakeDepositEmergency<'info> {
    fn into_withdraw_context(&self) -> CpiContext<'_, '_, '_, 'info, TransferChecked<'info>> {
        let cpi_accounts = TransferChecked {
            from: self.pool_vault.to_account_info(),
            mint: self.mint.to_account_info(),
            to: self.staker_ata.to_account_info(),
            authority: self.pool.to_account_info(),
        };
        CpiContext::new(self.token_program.to_account_info(), cpi_accounts)
    }
}

#[derive(Accounts)]
pub struct WithdrawRewardsEmergency<'info> {
    pub mint: Account<'info, Mint>,
    #[account(mut)]
    pub creator: Signer<'info>,
    #[account(mut)]
    pub pool: Account<'info, StakingPool>,
    #[account(mut)]
    pub pool_vault: Account<'info, TokenAccount>,
    #[account(mut)]
    pub creator_ata: Account<'info, TokenAccount>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

impl<'info> WithdrawRewardsEmergency<'info> {
    fn into_withdraw_context(&self) -> CpiContext<'_, '_, '_, 'info, TransferChecked<'info>> {
        let cpi_accounts = TransferChecked {
            from: self.pool_vault.to_account_info(),
            mint: self.mint.to_account_info(),
            to: self.creator_ata.to_account_info(),
            authority: self.pool.to_account_info(),
        };
        CpiContext::new(self.token_program.to_account_info(), cpi_accounts)
    }
}

#[derive(Accounts)]
pub struct UpdatePool<'info> {
    pub mint: Account<'info, Mint>,
    #[account(mut)]
    pub creator: Signer<'info>,
    #[account(mut)]
    pub pool: Account<'info, StakingPool>,
    #[account(mut)]
    pub pool_vault: Account<'info, TokenAccount>,
    #[account(mut)]
    pub creator_ata: Account<'info, TokenAccount>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

impl<'info> UpdatePool<'info> {
    fn into_transfer_to_pda_context(
        &self,
    ) -> CpiContext<'_, '_, '_, 'info, TransferChecked<'info>> {
        let cpi_accounts = TransferChecked {
            from: self.creator_ata.to_account_info(),
            mint: self.mint.to_account_info(),
            to: self.pool_vault.to_account_info(),
            authority: self.creator.to_account_info(),
        };
        CpiContext::new(self.token_program.to_account_info(), cpi_accounts)
    }
}

#[error_code]
pub enum StakingError {
    #[msg("Invalid token decimals")]
    InvalidTokenDecimals,
    #[msg("Emergency mode already enabled")]
    EmergencyModeAlreadyEnabled,
    #[msg("Emergency mode is enabled")]
    EmergencyModeEnabled,
    #[msg("Not enough tokens to unstake")]
    NotEnoughTokensToUnstake,
    #[msg("Claim cooldown has not elapsed")]
    ClaimCooldownNotElapsed,
    #[msg("Claim cooldown is not active")]
    ClaimCooldownNotActive,
    #[msg("Cooldown already activated")]
    CooldownAlreadyActivated,
    #[msg("Emergency mode is not enabled")]
    EmergencyModeNotEnabled,
    #[msg("Deposit already withdrawn")]
    DepositAlreadyWithdrawn,
    #[msg("Unauthorized pool access")]
    UnauthorizedPoolAccess,
}

