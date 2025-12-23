use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, TransferChecked};

declare_id!("4FKK3U22YDwotz1yHk8Ye6TkQ32whRdnHCv34eRBuLJ9");

#[program]
pub mod ico {
    use super::*;

    /// Create a new presale pool and deposit `tokens_for_sale` into the pool vault.
    pub fn create_presale_pool(
        ctx: Context<CreatePresalePool>,
        pool_id: u64,
        token_price_lamports: u64,
        soft_cap: u64,
        hard_cap: u64,
        min_contribution: u64,
        max_contribution: u64,
        start_timestamp: i64,
        end_timestamp: i64,
        tokens_for_sale: u64,
    ) -> Result<()> {
        let presale = &mut ctx.accounts.presale;
        let authority = &ctx.accounts.authority;

        require!(soft_cap < hard_cap, IcoError::SoftcapHigherThanHardcap);
        require!(
            min_contribution <= max_contribution,
            IcoError::MaxContributionLessThanMinContribution
        );
        require!(
            start_timestamp < end_timestamp,
            IcoError::EndTimestampBeforeStart
        );
        require!(token_price_lamports > 0, IcoError::InvalidPrice);

        presale.authority = authority.key();
        presale.token_mint = ctx.accounts.token_mint.key();
        presale.funds_receiver = ctx.accounts.funds_receiver.key();
        presale.soft_cap = soft_cap;
        presale.hard_cap = hard_cap;
        presale.min_contribution = min_contribution;
        presale.max_contribution = max_contribution;
        presale.start_timestamp = start_timestamp;
        presale.end_timestamp = end_timestamp;
        presale.total_contributions = 0;
        presale.token_price_lamports = token_price_lamports;
        presale.pool_id = pool_id;
        presale.bump = ctx.bumps.presale;

        // Transfer the tokens that will be sold into the presale vault.
        if tokens_for_sale > 0 {
            let cpi_accounts = TransferChecked {
                from: ctx
                    .accounts
                    .authority_token_account
                    .to_account_info(),
                mint: ctx.accounts.token_mint.to_account_info(),
                to: ctx.accounts.presale_vault.to_account_info(),
                authority: authority.to_account_info(),
            };
            let cpi_ctx = CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                cpi_accounts,
            );
            token::transfer_checked(
                cpi_ctx,
                tokens_for_sale,
                ctx.accounts.token_mint.decimals,
            )?;
        }

        Ok(())
    }

    /// Contribute SOL into a presale pool according to its parameters.
    pub fn contribute(ctx: Context<Contribute>, amount: u64) -> Result<()> {
        let presale = &mut ctx.accounts.presale;
        let profile = &mut ctx.accounts.profile;
        let contributor = &ctx.accounts.contributor;

        let clock = Clock::get()?;
        let now = clock.unix_timestamp;

        // Time window checks.
        require!(
            now >= presale.start_timestamp,
            IcoError::SaleNotStartedYet
        );
        require!(now <= presale.end_timestamp, IcoError::SaleEnded);

        // Min / max contribution checks.
        require!(
            amount >= presale.min_contribution,
            IcoError::ContributionBelowMinimum
        );

        let new_contribution = profile
            .contributed
            .checked_add(amount)
            .ok_or(IcoError::MathOverflow)?;
        require!(
            new_contribution <= presale.max_contribution,
            IcoError::ContributionAboveMaximum
        );

        let new_total = presale
            .total_contributions
            .checked_add(amount)
            .ok_or(IcoError::MathOverflow)?;
        require!(new_total <= presale.hard_cap, IcoError::HardcapExceeded);

        // Initialize profile on first contribution.
        if profile.contributed == 0 {
            profile.presale = presale.key();
            profile.contributor = contributor.key();
            profile.bump = ctx.bumps.profile;
            profile.claimed = false;
        }

        profile.contributed = new_contribution;
        presale.total_contributions = new_total;

        // Transfer SOL from contributor to the presale pool PDA.
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: contributor.to_account_info(),
                to: presale.to_account_info(),
            },
        );
        system_program::transfer(cpi_ctx, amount)?;

        emit!(Contributed {
            presale: presale.key(),
            contributor: contributor.key(),
            amount,
        });

        Ok(())
    }

    /// Claim: if soft cap not reached – refund SOL; otherwise receive tokens.
    pub fn claim(ctx: Context<Claim>) -> Result<()> {
        let presale = &mut ctx.accounts.presale;
        let profile = &mut ctx.accounts.profile;
        let contributor = &ctx.accounts.contributor;

        require!(!profile.claimed, IcoError::AlreadyClaimed);
        let contributed = profile.contributed;
        require!(contributed > 0, IcoError::NothingToClaim);

        // If soft cap not reached, refund SOL.
        if presale.total_contributions < presale.soft_cap {
            // Move lamports directly from the presale PDA to the contributor.
            // This avoids needing the presale PDA to sign a system_program::transfer CPI.
            **presale.to_account_info().try_borrow_mut_lamports()? -= contributed;
            **contributor
                .to_account_info()
                .try_borrow_mut_lamports()? += contributed;

            profile.claimed = true;

            emit!(Refunded {
                presale: presale.key(),
                contributor: contributor.key(),
                amount: contributed,
            });

            return Ok(());
        }

        // Successful sale: send tokens.
        let price = presale.token_price_lamports;
        require!(price > 0, IcoError::InvalidPrice);

        let decimals = ctx.accounts.token_mint.decimals;
        let ten_pow_decimals = 10u64
            .checked_pow(decimals as u32)
            .ok_or(IcoError::MathOverflow)?;

        // tokens_to_send = contributed * 10^decimals / price_lamports_per_token
        let numerator = contributed
            .checked_mul(ten_pow_decimals)
            .ok_or(IcoError::MathOverflow)?;
        let tokens_to_send = numerator
            .checked_div(price)
            .ok_or(IcoError::MathOverflow)?;

        require!(tokens_to_send > 0, IcoError::NothingToClaim);
        require!(
            ctx.accounts.presale_vault.amount >= tokens_to_send,
            IcoError::NotEnoughTokensInVault
        );

        let signer_seeds: &[&[u8]] =
            &[b"state", &presale.pool_id.to_le_bytes(), &[presale.bump]];
        let signers = &[&signer_seeds[..]];

        let cpi_accounts = TransferChecked {
            from: ctx.accounts.presale_vault.to_account_info(),
            mint: ctx.accounts.token_mint.to_account_info(),
            to: ctx.accounts.contributor_ata.to_account_info(),
            authority: presale.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signers,
        );
        token::transfer_checked(
            cpi_ctx,
            tokens_to_send,
            ctx.accounts.token_mint.decimals,
        )?;

        profile.claimed = true;

        emit!(TokensClaimed {
            presale: presale.key(),
            contributor: contributor.key(),
            contribution: contributed,
            amount: tokens_to_send,
        });

        Ok(())
    }

    /// Admin-only: withdraw SOL from the pool to the receiver address if soft cap reached.
    pub fn admin_withdraw(ctx: Context<AdminWithdraw>, amount: u64) -> Result<()> {
        let presale = &mut ctx.accounts.presale;

        require!(
            presale.total_contributions >= presale.soft_cap,
            IcoError::SoftcapNotReached
        );

        let available = amount.min(presale.get_lamports());
        let rent = Rent::get()?;
        let min_balance = rent.minimum_balance(8 + PresalePool::INIT_SPACE);
        require!(available > min_balance, IcoError::NothingToWithdraw);

        let withdraw_amount = available - min_balance;

        presale.sub_lamports(withdraw_amount)?;
        ctx.accounts
            .funds_receiver
            .add_lamports(withdraw_amount)?;

        Ok(())
    }

    /// Admin-only: emergency withdraw of tokens from the vault before the sale starts.
    pub fn emergency_withdraw_token(
        ctx: Context<EmergencyWithdrawToken>,
        amount: u64,
    ) -> Result<()> {
        let presale = &ctx.accounts.presale;
        let clock = Clock::get()?;

        require!(
            clock.unix_timestamp < presale.start_timestamp,
            IcoError::EmergencyWithdrawOnlyBeforeStart
        );

        let actual_amount = amount.min(ctx.accounts.presale_vault.amount);
        require!(actual_amount > 0, IcoError::NothingToWithdraw);

        let signer_seeds: &[&[u8]] =
            &[b"state", &presale.pool_id.to_le_bytes(), &[presale.bump]];
        let signers = &[&signer_seeds[..]];

        let cpi_accounts = TransferChecked {
            from: ctx.accounts.presale_vault.to_account_info(),
            mint: ctx.accounts.token_mint.to_account_info(),
            to: ctx.accounts.receiver_ata.to_account_info(),
            authority: ctx.accounts.presale.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signers,
        );
        token::transfer_checked(
            cpi_ctx,
            actual_amount,
            ctx.accounts.token_mint.decimals,
        )?;

        Ok(())
    }
}

#[account]
#[derive(InitSpace)]
pub struct PresalePool {
    /// Admin / creator of the pool.
    pub authority: Pubkey,
    /// SPL token mint sold in this presale.
    pub token_mint: Pubkey,
    /// Where SOL goes if the soft cap is reached.
    pub funds_receiver: Pubkey,

    /// Minimum total raised for the sale to be valid (lamports).
    pub soft_cap: u64,
    /// Maximum total raised (lamports).
    pub hard_cap: u64,

    /// Min / max contribution per user (lamports).
    pub min_contribution: u64,
    pub max_contribution: u64,

    /// Sale window.
    pub start_timestamp: i64,
    pub end_timestamp: i64,

    /// Total SOL contributed so far (lamports).
    pub total_contributions: u64,

    /// Price in lamports per full token (10^decimals units).
    pub token_price_lamports: u64,

    /// Pool id used in PDA derivation.
    pub pool_id: u64,
    /// PDA bump.
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct ContributorProfile {
    /// Presale this profile belongs to.
    pub presale: Pubkey,
    /// Contributor address.
    pub contributor: Pubkey,
    /// Total contributed SOL (lamports).
    pub contributed: u64,
    /// Whether claim/refund has already been made.
    pub claimed: bool,
    /// PDA bump.
    pub bump: u8,
}

#[derive(Accounts)]
#[instruction(pool_id: u64)]
pub struct CreatePresalePool<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + PresalePool::INIT_SPACE,
        seeds = [b"state".as_ref(), &pool_id.to_le_bytes()],
        bump
    )]
    pub presale: Account<'info, PresalePool>,

    /// Pool admin.
    #[account(mut)]
    pub authority: Signer<'info>,

    /// Where raised SOL will be sent on `admin_withdraw`.
    /// CHECK: stored as a Pubkey in `PresalePool`.
    #[account(mut)]
    pub funds_receiver: AccountInfo<'info>,

    /// SPL token mint being sold.
    pub token_mint: Account<'info, Mint>,

    /// PDA token account that will hold sale tokens.
    #[account(
        init,
        payer = authority,
        token::mint = token_mint,
        token::authority = presale,
        seeds = [b"vault".as_ref(), presale.key().as_ref()],
        bump
    )]
    pub presale_vault: Account<'info, TokenAccount>,

    /// Admin's token account from which tokens are deposited into the vault.
    #[account(
        mut,
        constraint = authority_token_account.mint == token_mint.key(),
        constraint = authority_token_account.owner == authority.key()
    )]
    pub authority_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Contribute<'info> {
    /// Contributor paying SOL.
    #[account(mut)]
    pub contributor: Signer<'info>,

    /// Presale pool PDA.
    #[account(
        mut,
        seeds = [b"state".as_ref(), &presale.pool_id.to_le_bytes()],
        bump = presale.bump
    )]
    pub presale: Account<'info, PresalePool>,

    /// Contributor profile PDA, one per (contributor, presale).
    #[account(
        init_if_needed,
        payer = contributor,
        space = 8 + ContributorProfile::INIT_SPACE,
        seeds = [
            b"contributor-profile".as_ref(),
            contributor.key().as_ref(),
            presale.key().as_ref()
        ],
        bump
    )]
    pub profile: Account<'info, ContributorProfile>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Claim<'info> {
    /// Contributor receiving refund or tokens.
    #[account(mut)]
    pub contributor: Signer<'info>,

    #[account(
        mut,
        seeds = [b"state".as_ref(), &presale.pool_id.to_le_bytes()],
        bump = presale.bump
    )]
    pub presale: Account<'info, PresalePool>,

    #[account(
        mut,
        seeds = [
            b"contributor-profile".as_ref(),
            contributor.key().as_ref(),
            presale.key().as_ref()
        ],
        bump = profile.bump
    )]
    pub profile: Account<'info, ContributorProfile>,

    #[account(address = presale.token_mint)]
    pub token_mint: Account<'info, Mint>,

    #[account(
        mut,
        token::mint = token_mint,
        token::authority = presale
    )]
    pub presale_vault: Account<'info, TokenAccount>,

    #[account(
        init_if_needed,
        payer = contributor,
        associated_token::mint = token_mint,
        associated_token::authority = contributor
    )]
    pub contributor_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct AdminWithdraw<'info> {
    #[account(
        mut,
        has_one = authority,
        has_one = funds_receiver,
        seeds = [b"state".as_ref(), &presale.pool_id.to_le_bytes()],
        bump = presale.bump
    )]
    pub presale: Account<'info, PresalePool>,

    /// Admin / authority of the pool.
    pub authority: Signer<'info>,

    /// Destination for withdrawn SOL.
    /// CHECK: checked by `has_one = funds_receiver`.
    #[account(mut)]
    pub funds_receiver: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct EmergencyWithdrawToken<'info> {
    #[account(
        mut,
        has_one = authority,
        has_one = token_mint,
        seeds = [b"state".as_ref(), &presale.pool_id.to_le_bytes()],
        bump = presale.bump
    )]
    pub presale: Account<'info, PresalePool>,

    /// Admin / authority.
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(address = presale.token_mint)]
    pub token_mint: Account<'info, Mint>,

    #[account(
        mut,
        token::mint = token_mint,
        token::authority = presale
    )]
    pub presale_vault: Account<'info, TokenAccount>,

    /// Receiver of emergency-withdrawn tokens.
    /// CHECK: arbitrary receiver, only its pubkey is used for ATA derivation.
    #[account(mut)]
    pub receiver: AccountInfo<'info>,

    #[account(
        init_if_needed,
        payer = authority,
        associated_token::mint = token_mint,
        associated_token::authority = receiver
    )]
    pub receiver_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[error_code]
pub enum IcoError {
    #[msg("Soft cap must be less than hard cap")]
    SoftcapHigherThanHardcap,
    #[msg("Max contribution must be >= min contribution")]
    MaxContributionLessThanMinContribution,
    #[msg("End timestamp must be after start timestamp")]
    EndTimestampBeforeStart,
    #[msg("Sale has not started yet")]
    SaleNotStartedYet,
    #[msg("Sale has already ended")]
    SaleEnded,
    #[msg("Contribution below minimum")]
    ContributionBelowMinimum,
    #[msg("Contribution above maximum allowed per user")]
    ContributionAboveMaximum,
    #[msg("Hard cap exceeded")]
    HardcapExceeded,
    #[msg("Soft cap not reached")]
    SoftcapNotReached,
    #[msg("Nothing to claim")]
    NothingToClaim,
    #[msg("Already claimed")]
    AlreadyClaimed,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Invalid token price")]
    InvalidPrice,
    #[msg("Not enough tokens in presale vault")]
    NotEnoughTokensInVault,
    #[msg("Nothing to withdraw")]
    NothingToWithdraw,
    #[msg("Emergency withdraw allowed only before sale starts")]
    EmergencyWithdrawOnlyBeforeStart,
}

#[event]
pub struct Contributed {
    pub presale: Pubkey,
    pub contributor: Pubkey,
    pub amount: u64,
}

#[event]
pub struct TokensClaimed {
    pub presale: Pubkey,
    pub contributor: Pubkey,
    pub contribution: u64,
    pub amount: u64,
}

#[event]
pub struct Refunded {
    pub presale: Pubkey,
    pub contributor: Pubkey,
    pub amount: u64,
}