use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, TransferChecked};

declare_id!("A3ThhSfoxnsQHEMToLZBKoxsPZ2CcBQSw8sGFFE45CXE");

#[program]
pub mod vesting {
    use super::*;

    /// Create a new vesting schedule and lock tokens in a vault.
    pub fn create_vesting(
        ctx: Context<CreateVesting>,
        cliff_duration: i64,
        interval_duration: i64,
        unlock_percentage: u8,
        total_amount: u64,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let now = clock.unix_timestamp;

        // Input validation
        require!(cliff_duration > 0, VestingError::InvalidCliffDuration);
        require!(interval_duration > 0, VestingError::InvalidIntervalDuration);
        require!(
            unlock_percentage > 0 && unlock_percentage <= 100,
            VestingError::InvalidUnlockPercentage
        );
        require!(total_amount > 0, VestingError::InvalidAmount);

        // Calculate cliff end timestamp
        let cliff_end_timestamp = now
            .checked_add(cliff_duration)
            .ok_or(VestingError::MathOverflow)?;

        let vesting = &mut ctx.accounts.vesting_schedule;
        vesting.creator = ctx.accounts.creator.key();
        vesting.beneficiary = ctx.accounts.beneficiary.key();
        vesting.token_mint = ctx.accounts.token_mint.key();
        vesting.vault = ctx.accounts.vault.key();
        vesting.total_amount = total_amount;
        vesting.unlocked_amount = 0;
        vesting.cliff_end_timestamp = cliff_end_timestamp;
        vesting.interval_duration = interval_duration;
        vesting.unlock_percentage = unlock_percentage;
        vesting.last_unlock_timestamp = cliff_end_timestamp;
        vesting.created_at = now;
        vesting.bump = ctx.bumps.vesting_schedule;

        // Transfer tokens from creator to vault
        let cpi_accounts = TransferChecked {
            from: ctx.accounts.creator_token_account.to_account_info(),
            mint: ctx.accounts.token_mint.to_account_info(),
            to: ctx.accounts.vault.to_account_info(),
            authority: ctx.accounts.creator.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
        );
        token::transfer_checked(
            cpi_ctx,
            total_amount,
            ctx.accounts.token_mint.decimals,
        )?;

        emit!(VestingCreated {
            vesting_schedule: vesting.key(),
            creator: vesting.creator,
            beneficiary: vesting.beneficiary,
            token_mint: vesting.token_mint,
            total_amount,
            cliff_end_timestamp,
            interval_duration,
            unlock_percentage,
        });

        Ok(())
    }

    /// Unlock vested tokens to the beneficiary.
    pub fn unlock(ctx: Context<Unlock>) -> Result<()> {
        let clock = Clock::get()?;
        let now = clock.unix_timestamp;

        // Read vesting schedule first (immutable borrow)
        let cliff_end_timestamp = ctx.accounts.vesting_schedule.cliff_end_timestamp;
        let interval_duration = ctx.accounts.vesting_schedule.interval_duration;
        let unlock_percentage = ctx.accounts.vesting_schedule.unlock_percentage;
        let total_amount = ctx.accounts.vesting_schedule.total_amount;
        let unlocked_amount = ctx.accounts.vesting_schedule.unlocked_amount;
        let last_unlock_timestamp = ctx.accounts.vesting_schedule.last_unlock_timestamp;
        let creator_key = ctx.accounts.vesting_schedule.creator;
        let beneficiary_key = ctx.accounts.vesting_schedule.beneficiary;
        let bump = ctx.accounts.vesting_schedule.bump;
        let decimals = ctx.accounts.token_mint.decimals;

        // Check that cliff has passed
        require!(
            now >= cliff_end_timestamp,
            VestingError::CliffNotPassed
        );

        // Calculate how many intervals have passed since cliff ended
        let time_since_cliff = now
            .checked_sub(cliff_end_timestamp)
            .ok_or(VestingError::MathOverflow)?;
        let total_intervals_passed_i64 = time_since_cliff
            .checked_div(interval_duration)
            .ok_or(VestingError::MathOverflow)?;
        
        // Convert to u64 (intervals can't be negative)
        let total_intervals_passed = total_intervals_passed_i64.max(0) as u64;

        // For first unlock, require at least one interval to have passed
        if unlocked_amount == 0 {
            require!(
                total_intervals_passed >= 1,
                VestingError::IntervalNotPassed
            );
        } else {
            // For subsequent unlocks, check time since last unlock
            let time_since_last_unlock = now
                .checked_sub(last_unlock_timestamp)
                .ok_or(VestingError::MathOverflow)?;
            require!(
                time_since_last_unlock >= interval_duration,
                VestingError::IntervalNotPassed
            );
        }

        // Calculate how many intervals have been unlocked so far
        let percentage_per_interval = unlock_percentage as u64;
        let intervals_unlocked_so_far = if unlocked_amount == 0 {
            0u64
        } else {
            // Calculate: unlocked_amount / (total_amount * unlock_percentage / 100)
            let amount_per_interval = total_amount
                .checked_mul(percentage_per_interval)
                .ok_or(VestingError::MathOverflow)?
                .checked_div(100)
                .ok_or(VestingError::MathOverflow)?;
            unlocked_amount
                .checked_div(amount_per_interval)
                .unwrap_or(0)
        };

        // Calculate how many new intervals can be unlocked
        let new_intervals_to_unlock = total_intervals_passed
            .checked_sub(intervals_unlocked_so_far)
            .ok_or(VestingError::MathOverflow)?;

        require!(new_intervals_to_unlock > 0, VestingError::NothingToUnlock);

        // Calculate amount to unlock: only one interval at a time
        let amount_per_interval = total_amount
            .checked_mul(percentage_per_interval)
            .ok_or(VestingError::MathOverflow)?
            .checked_div(100)
            .ok_or(VestingError::MathOverflow)?;

        // Unlock only one interval worth of tokens
        let amount_to_unlock = amount_per_interval.min(
            total_amount
                .checked_sub(unlocked_amount)
                .ok_or(VestingError::MathOverflow)?
        );

        require!(amount_to_unlock > 0, VestingError::NothingToUnlock);

        // Ensure vault has enough tokens
        require!(
            ctx.accounts.vault.amount >= amount_to_unlock,
            VestingError::InsufficientVaultBalance
        );

        // Transfer tokens from vault to beneficiary
        let signer_seeds: &[&[u8]] = &[
            b"vesting-schedule",
            creator_key.as_ref(),
            beneficiary_key.as_ref(),
            &[bump],
        ];
        let signers = &[&signer_seeds[..]];

        let cpi_accounts = TransferChecked {
            from: ctx.accounts.vault.to_account_info(),
            mint: ctx.accounts.token_mint.to_account_info(),
            to: ctx.accounts.beneficiary_ata.to_account_info(),
            authority: ctx.accounts.vesting_schedule.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signers,
        );
        token::transfer_checked(cpi_ctx, amount_to_unlock, decimals)?;

        // Update vesting schedule (now we can mutably borrow)
        let vesting = &mut ctx.accounts.vesting_schedule;
        vesting.unlocked_amount = unlocked_amount
            .checked_add(amount_to_unlock)
            .ok_or(VestingError::MathOverflow)?;
        vesting.last_unlock_timestamp = now;

        emit!(TokensUnlocked {
            vesting_schedule: vesting.key(),
            beneficiary: vesting.beneficiary,
            amount: amount_to_unlock,
            remaining: vesting
                .total_amount
                .checked_sub(vesting.unlocked_amount)
                .unwrap_or(0),
        });

        Ok(())
    }

    /// Calculate the amount of tokens available for unlock without actually unlocking.
    /// Result is logged as a message that can be parsed by clients.
    pub fn get_unlockable_amount(ctx: Context<GetUnlockableAmount>) -> Result<()> {
        let vesting = &ctx.accounts.vesting_schedule;
        let clock = Clock::get()?;
        let now = clock.unix_timestamp;

        let unlockable_amount = if now < vesting.cliff_end_timestamp {
            // If cliff hasn't passed, nothing is unlockable
            0u64
        } else {
            // Calculate how many intervals have passed since cliff ended
            let time_since_cliff = now
                .checked_sub(vesting.cliff_end_timestamp)
                .ok_or(VestingError::MathOverflow)?;
            let intervals_passed_i64 = time_since_cliff
                .checked_div(vesting.interval_duration)
                .ok_or(VestingError::MathOverflow)?;
            
            // Convert to u64 (intervals can't be negative)
            let intervals_passed = intervals_passed_i64.max(0) as u64;

            // Calculate total unlockable amount based on intervals
            let percentage_per_interval = vesting.unlock_percentage as u64;
            let total_percentage_unlockable = intervals_passed
                .checked_mul(percentage_per_interval)
                .ok_or(VestingError::MathOverflow)?;

            // Calculate unlockable amount: (total_amount * total_percentage_unlockable) / 100
            let unlockable_amount = vesting
                .total_amount
                .checked_mul(total_percentage_unlockable)
                .ok_or(VestingError::MathOverflow)?
                .checked_div(100)
                .ok_or(VestingError::MathOverflow)?;

            // Ensure we don't unlock more than total amount
            let max_unlockable = unlockable_amount.min(vesting.total_amount);

            // Calculate how much can be unlocked now (subtract already unlocked)
            max_unlockable
                .checked_sub(vesting.unlocked_amount)
                .unwrap_or(0)
        };

        // Log the result as JSON for clients to parse
        msg!("{{\"unlockable_amount\":{}}}", unlockable_amount);
        Ok(())
    }
}

#[account]
#[derive(InitSpace)]
pub struct VestingSchedule {
    /// Creator of the vesting schedule
    pub creator: Pubkey,
    /// Beneficiary who receives the tokens
    pub beneficiary: Pubkey,
    /// Token mint being vested
    pub token_mint: Pubkey,
    /// Vault PDA that holds the locked tokens
    pub vault: Pubkey,
    /// Total amount of tokens locked
    pub total_amount: u64,
    /// Amount already unlocked
    pub unlocked_amount: u64,
    /// Timestamp when cliff period ends
    pub cliff_end_timestamp: i64,
    /// Duration of each unlock interval in seconds
    pub interval_duration: i64,
    /// Percentage unlocked per interval (0-100)
    pub unlock_percentage: u8,
    /// Timestamp of last unlock
    pub last_unlock_timestamp: i64,
    /// Timestamp when vesting was created
    pub created_at: i64,
    /// PDA bump
    pub bump: u8,
}

#[derive(Accounts)]
pub struct CreateVesting<'info> {
    #[account(
        init,
        payer = creator,
        space = 8 + VestingSchedule::INIT_SPACE,
        seeds = [
            b"vesting-schedule",
            creator.key().as_ref(),
            beneficiary.key().as_ref()
        ],
        bump
    )]
    pub vesting_schedule: Account<'info, VestingSchedule>,

    /// Creator who locks the tokens
    #[account(mut)]
    pub creator: Signer<'info>,

    /// Beneficiary who will receive the tokens
    /// CHECK: stored as Pubkey in VestingSchedule
    pub beneficiary: AccountInfo<'info>,

    /// Token mint being vested
    pub token_mint: Account<'info, Mint>,

    /// Creator's token account from which tokens are transferred
    #[account(
        mut,
        constraint = creator_token_account.mint == token_mint.key(),
        constraint = creator_token_account.owner == creator.key()
    )]
    pub creator_token_account: Account<'info, TokenAccount>,

    /// Vault PDA that will hold the locked tokens
    #[account(
        init,
        payer = creator,
        token::mint = token_mint,
        token::authority = vesting_schedule,
        seeds = [
            b"vault",
            vesting_schedule.key().as_ref()
        ],
        bump
    )]
    pub vault: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Unlock<'info> {
    #[account(
        mut,
        has_one = beneficiary,
        has_one = token_mint,
        has_one = vault,
        seeds = [
            b"vesting-schedule",
            vesting_schedule.creator.as_ref(),
            beneficiary.key().as_ref()
        ],
        bump = vesting_schedule.bump
    )]
    pub vesting_schedule: Account<'info, VestingSchedule>,

    /// Beneficiary who receives the unlocked tokens
    #[account(mut)]
    pub beneficiary: Signer<'info>,

    #[account(address = vesting_schedule.token_mint)]
    pub token_mint: Account<'info, Mint>,

    #[account(
        mut,
        constraint = vault.mint == token_mint.key(),
        constraint = vault.owner == vesting_schedule.key()
    )]
    pub vault: Account<'info, TokenAccount>,

    #[account(
        init_if_needed,
        payer = beneficiary,
        associated_token::mint = token_mint,
        associated_token::authority = beneficiary
    )]
    pub beneficiary_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct GetUnlockableAmount<'info> {
    #[account(
        seeds = [
            b"vesting-schedule",
            vesting_schedule.creator.as_ref(),
            vesting_schedule.beneficiary.as_ref()
        ],
        bump = vesting_schedule.bump
    )]
    pub vesting_schedule: Account<'info, VestingSchedule>,
}

#[error_code]
pub enum VestingError {
    #[msg("Invalid cliff duration")]
    InvalidCliffDuration,
    #[msg("Invalid interval duration")]
    InvalidIntervalDuration,
    #[msg("Invalid unlock percentage (must be 1-100)")]
    InvalidUnlockPercentage,
    #[msg("Invalid amount")]
    InvalidAmount,
    #[msg("Cliff period has not passed yet")]
    CliffNotPassed,
    #[msg("Interval duration has not passed since last unlock")]
    IntervalNotPassed,
    #[msg("Nothing to unlock")]
    NothingToUnlock,
    #[msg("Insufficient balance in vault")]
    InsufficientVaultBalance,
    #[msg("Math overflow")]
    MathOverflow,
}

#[event]
pub struct VestingCreated {
    pub vesting_schedule: Pubkey,
    pub creator: Pubkey,
    pub beneficiary: Pubkey,
    pub token_mint: Pubkey,
    pub total_amount: u64,
    pub cliff_end_timestamp: i64,
    pub interval_duration: i64,
    pub unlock_percentage: u8,
}

#[event]
pub struct TokensUnlocked {
    pub vesting_schedule: Pubkey,
    pub beneficiary: Pubkey,
    pub amount: u64,
    pub remaining: u64,
}

