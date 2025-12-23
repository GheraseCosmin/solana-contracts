use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, TransferChecked};

declare_id!("7SmvmUGRK9sx9eVXspVWyQeaTPqjTPa5xQui3kgg6AMk");

#[program]
pub mod multisig {
    use super::*;

    /// Create a new multisig vault with specified signers and threshold
    pub fn create_vault(
        ctx: Context<CreateVault>,
        vault_id: u64,
        signers: Vec<Pubkey>,
        threshold: u8,
    ) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        let creator = &ctx.accounts.creator;

        // Validate signers
        require!(!signers.is_empty(), MultisigError::EmptySigners);
        require!(signers.len() <= 5, MultisigError::TooManySigners);
        require!(threshold > 0, MultisigError::InvalidThreshold);
        require!(
            threshold as usize <= signers.len(),
            MultisigError::ThresholdTooHigh
        );

        // Check for duplicate signers
        let mut unique_signers = signers.clone();
        unique_signers.sort();
        unique_signers.dedup();
        require!(
            unique_signers.len() == signers.len(),
            MultisigError::DuplicateSigners
        );

        vault.vault_id = vault_id;
        vault.signers = signers;
        vault.threshold = threshold;
        vault.vault_bump = ctx.bumps.vault;
        vault.creator = creator.key();

        Ok(())
    }

    /// Propose a transfer from the vault
    pub fn propose_transfer(
        ctx: Context<ProposeTransfer>,
        proposal_id: u64,
        recipient: Pubkey,
        amount: u64,
        token_mint: Option<Pubkey>,
    ) -> Result<()> {
        let vault = &ctx.accounts.vault;
        let proposal = &mut ctx.accounts.proposal;
        let proposer = &ctx.accounts.proposer;

        // Verify proposer is a signer
        require!(
            vault.signers.contains(proposer.key),
            MultisigError::InvalidSigner
        );

        // Find proposer index
        let proposer_index = vault
            .signers
            .iter()
            .position(|&s| s == proposer.key())
            .ok_or(MultisigError::InvalidSigner)?;

        // Initialize proposal
        proposal.vault = vault.key();
        proposal.proposer = proposer.key();
        proposal.recipient = recipient;
        proposal.amount = amount;
        proposal.token_mint = token_mint;
        proposal.proposal_id = proposal_id;
        proposal.executed = false;

        // Initialize approvals vector
        proposal.approvals = vec![false; vault.signers.len()];
        proposal.approvals[proposer_index] = true; // Auto-approve proposer

        Ok(())
    }

    /// Approve a transfer proposal
    pub fn approve_transfer(ctx: Context<ApproveTransfer>) -> Result<()> {
        let vault = &ctx.accounts.vault;
        let proposal = &mut ctx.accounts.proposal;
        let approver = &ctx.accounts.approver;

        // Verify proposal hasn't been executed
        require!(!proposal.executed, MultisigError::AlreadyExecuted);

        // Verify approver is a signer
        require!(
            vault.signers.contains(approver.key),
            MultisigError::InvalidSigner
        );

        // Find approver index
        let approver_index = vault
            .signers
            .iter()
            .position(|&s| s == approver.key())
            .ok_or(MultisigError::InvalidSigner)?;

        // Verify approver hasn't already approved
        require!(
            !proposal.approvals[approver_index],
            MultisigError::AlreadyApproved
        );

        // Mark approval
        proposal.approvals[approver_index] = true;

        Ok(())
    }

    /// Execute a SOL transfer proposal if threshold is met
    pub fn execute_sol_transfer(ctx: Context<ExecuteSolTransfer>) -> Result<()> {
        let vault = &ctx.accounts.vault;
        let proposal = &mut ctx.accounts.proposal;

        // Verify proposal hasn't been executed
        require!(!proposal.executed, MultisigError::AlreadyExecuted);

        // Verify this is a SOL transfer
        require!(
            proposal.token_mint.is_none(),
            MultisigError::TokenMintMismatch
        );

        // Count approvals
        let approval_count = proposal
            .approvals
            .iter()
            .filter(|&&approved| approved)
            .count();

        // Verify threshold is met
        require!(
            approval_count >= vault.threshold as usize,
            MultisigError::InsufficientApprovals
        );

        // Mark as executed before transfer to prevent reentrancy
        proposal.executed = true;

        let vault_id_bytes = vault.vault_id.to_le_bytes();
        let sol_seeds = &[
            b"vault_sol",
            vault.creator.as_ref(),
            vault_id_bytes.as_ref(),
            &[ctx.bumps.vault_sol_account],
        ];
        let signer = &[&sol_seeds[..]];

        let cpi_accounts = system_program::Transfer {
            from: ctx.accounts.vault_sol_account.to_account_info(),
            to: ctx.accounts.recipient.to_account_info(),
        };

        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            cpi_accounts,
            signer,
        );

        system_program::transfer(cpi_ctx, proposal.amount)?;

        Ok(())
    }

    /// Execute an SPL token transfer proposal if threshold is met
    pub fn execute_spl_transfer(ctx: Context<ExecuteSplTransfer>) -> Result<()> {
        let vault = &ctx.accounts.vault;
        let proposal = &mut ctx.accounts.proposal;
        let mint = &ctx.accounts.mint;
        let vault_token_account = &ctx.accounts.vault_token_account;
        let recipient_token_account = &ctx.accounts.recipient_token_account;

        // Verify proposal hasn't been executed
        require!(!proposal.executed, MultisigError::AlreadyExecuted);

        // Verify this is an SPL transfer
        let token_mint = proposal
            .token_mint
            .ok_or(MultisigError::TokenMintMismatch)?;

        // Verify token mint matches
        require!(
            mint.key() == token_mint,
            MultisigError::TokenMintMismatch
        );

        // Count approvals
        let approval_count = proposal
            .approvals
            .iter()
            .filter(|&&approved| approved)
            .count();

        // Verify threshold is met
        require!(
            approval_count >= vault.threshold as usize,
            MultisigError::InsufficientApprovals
        );

        // Verify vault token account owner
        require!(
            vault_token_account.owner == vault.key(),
            MultisigError::InvalidTokenAccount
        );

        // Mark as executed before transfer to prevent reentrancy
        proposal.executed = true;

        let vault_id_bytes = vault.vault_id.to_le_bytes();
        let seeds = &[
            b"multisig_vault",
            vault.creator.as_ref(),
            vault_id_bytes.as_ref(),
            &[vault.vault_bump],
        ];
        let signer = &[&seeds[..]];

        let cpi_accounts = TransferChecked {
            from: vault_token_account.to_account_info(),
            mint: mint.to_account_info(),
            to: recipient_token_account.to_account_info(),
            authority: ctx.accounts.vault_pda.to_account_info(),
        };

        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer,
        );

        token::transfer_checked(cpi_ctx, proposal.amount, mint.decimals)?;

        Ok(())
    }
}

#[account]
#[derive(InitSpace)]
pub struct MultisigVault {
    pub vault_id: u64,
    #[max_len(5)]
    pub signers: Vec<Pubkey>,
    pub threshold: u8,
    pub vault_bump: u8,
    pub creator: Pubkey,
}

#[account]
#[derive(InitSpace)]
pub struct TransferProposal {
    pub vault: Pubkey,
    pub proposer: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
    pub token_mint: Option<Pubkey>,
    #[max_len(5)]
    pub approvals: Vec<bool>,
    pub executed: bool,
    pub proposal_id: u64,
}

#[derive(Accounts)]
#[instruction(vault_id: u64, signers: Vec<Pubkey>, threshold: u8)]
pub struct CreateVault<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,

    #[account(
        init,
        payer = creator,
        space = 8 + MultisigVault::INIT_SPACE,
        seeds = [b"multisig_vault", creator.key().as_ref(), vault_id.to_le_bytes().as_ref()],
        bump
    )]
    pub vault: Account<'info, MultisigVault>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(proposal_id: u64)]
pub struct ProposeTransfer<'info> {
    #[account(
        seeds = [b"multisig_vault", vault.creator.as_ref(), vault.vault_id.to_le_bytes().as_ref()],
        bump = vault.vault_bump
    )]
    pub vault: Account<'info, MultisigVault>,

    #[account(
        init,
        payer = proposer,
        space = 8 + TransferProposal::INIT_SPACE,
        seeds = [b"transfer_proposal", vault.key().as_ref(), proposal_id.to_le_bytes().as_ref()],
        bump
    )]
    pub proposal: Account<'info, TransferProposal>,

    #[account(mut)]
    pub proposer: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ApproveTransfer<'info> {
    #[account(
        seeds = [b"multisig_vault", vault.creator.as_ref(), vault.vault_id.to_le_bytes().as_ref()],
        bump = vault.vault_bump
    )]
    pub vault: Account<'info, MultisigVault>,

    #[account(
        mut,
        seeds = [b"transfer_proposal", vault.key().as_ref(), proposal.proposal_id.to_le_bytes().as_ref()],
        bump,
        has_one = vault
    )]
    pub proposal: Account<'info, TransferProposal>,

    pub approver: Signer<'info>,
}

#[derive(Accounts)]
pub struct ExecuteSolTransfer<'info> {
    #[account(
        seeds = [b"multisig_vault", vault.creator.as_ref(), vault.vault_id.to_le_bytes().as_ref()],
        bump = vault.vault_bump
    )]
    pub vault: Account<'info, MultisigVault>,

    #[account(
        mut,
        seeds = [b"transfer_proposal", vault.key().as_ref(), proposal.proposal_id.to_le_bytes().as_ref()],
        bump,
        has_one = vault
    )]
    pub proposal: Account<'info, TransferProposal>,

    /// CHECK: SOL account for vault (separate PDA without data)
    #[account(
        mut,
        seeds = [b"vault_sol", vault.creator.as_ref(), vault.vault_id.to_le_bytes().as_ref()],
        bump
    )]
    pub vault_sol_account: AccountInfo<'info>,

    /// CHECK: Recipient account
    #[account(mut)]
    pub recipient: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ExecuteSplTransfer<'info> {
    #[account(
        seeds = [b"multisig_vault", vault.creator.as_ref(), vault.vault_id.to_le_bytes().as_ref()],
        bump = vault.vault_bump
    )]
    pub vault: Account<'info, MultisigVault>,

    /// CHECK: PDA signer for vault
    #[account(
        seeds = [b"multisig_vault", vault.creator.as_ref(), vault.vault_id.to_le_bytes().as_ref()],
        bump = vault.vault_bump
    )]
    pub vault_pda: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [b"transfer_proposal", vault.key().as_ref(), proposal.proposal_id.to_le_bytes().as_ref()],
        bump,
        has_one = vault
    )]
    pub proposal: Account<'info, TransferProposal>,

    pub mint: Account<'info, Mint>,

    #[account(mut)]
    pub vault_token_account: Account<'info, TokenAccount>,

    #[account(mut)]
    pub recipient_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

#[error_code]
pub enum MultisigError {
    #[msg("Signers list cannot be empty")]
    EmptySigners,
    #[msg("Too many signers (maximum 5)")]
    TooManySigners,
    #[msg("Invalid threshold")]
    InvalidThreshold,
    #[msg("Threshold cannot exceed number of signers")]
    ThresholdTooHigh,
    #[msg("Duplicate signers found")]
    DuplicateSigners,
    #[msg("Invalid signer")]
    InvalidSigner,
    #[msg("Proposal already executed")]
    AlreadyExecuted,
    #[msg("Signer already approved this proposal")]
    AlreadyApproved,
    #[msg("Insufficient approvals to execute")]
    InsufficientApprovals,
    #[msg("Token mint mismatch")]
    TokenMintMismatch,
    #[msg("Invalid token account")]
    InvalidTokenAccount,
}
