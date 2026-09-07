// ============================================================================
//  token-vault — a beginner-friendly Solana program written with Anchor.
//
//  What it does:
//    Every user can open a personal "vault" for one SPL token mint,
//    deposit tokens into it, and withdraw them later.
//    Nobody else can touch their tokens — not even the program author.
//
//  Three instructions:
//    1. initialize_vault  -> create the vault (once per user, per mint)
//    2. deposit(amount)   -> move tokens from the user's wallet into the vault
//    3. withdraw(amount)  -> move tokens back out
// ============================================================================

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

// The on-chain address of this program. `anchor keys sync` overwrites it
// with your real key after the first build.
declare_id!("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");

#[program]
pub mod token_vault {
    use super::*;

    /// Creates the vault. Two accounts are born here:
    ///   - `vault`               : a small data account that remembers who owns
    ///                             what, and how much is inside.
    ///   - `vault_token_account` : the actual SPL token account that holds the
    ///                             tokens. Its authority is the `vault` PDA,
    ///                             which means only this program can move them.
    pub fn initialize_vault(ctx: Context<InitializeVault>) -> Result<()> {
        let vault = &mut ctx.accounts.vault;

        vault.owner = ctx.accounts.owner.key();
        vault.mint = ctx.accounts.mint.key();
        vault.amount = 0;
        // Anchor found the canonical bump for us while deriving the PDA.
        // We store it so future instructions don't have to search for it again.
        vault.bump = ctx.bumps.vault;

        msg!("Vault created for owner {}", vault.owner);
        Ok(())
    }

    /// Moves `amount` tokens from the user's own token account into the vault.
    /// The *user* signs this transfer, so it's a plain CPI — no PDA signing.
    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        require!(amount > 0, VaultError::ZeroAmount);

        // Describe the accounts the SPL Token program needs for a transfer.
        let cpi_accounts = Transfer {
            from: ctx.accounts.owner_token_account.to_account_info(),
            to: ctx.accounts.vault_token_account.to_account_info(),
            authority: ctx.accounts.owner.to_account_info(),
        };

        // A CPI ("cross-program invocation") is one program calling another.
        // Here our program asks the SPL Token program to do the actual move.
        let cpi_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
        );
        token::transfer(cpi_ctx, amount)?;

        // Keep our own bookkeeping in sync. checked_add prevents overflow.
        let vault = &mut ctx.accounts.vault;
        vault.amount = vault
            .amount
            .checked_add(amount)
            .ok_or(VaultError::MathOverflow)?;

        msg!("Deposited {} — vault now holds {}", amount, vault.amount);
        Ok(())
    }

    /// Moves `amount` tokens back out of the vault to the user.
    /// The tokens are owned by a PDA, so *the program* signs on its behalf.
    pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
        require!(amount > 0, VaultError::ZeroAmount);
        require!(
            ctx.accounts.vault.amount >= amount,
            VaultError::InsufficientFunds
        );

        // Rebuild the exact seeds that produced the vault PDA. Passing them to
        // `new_with_signer` is how a program proves "this PDA is mine to sign for".
        let owner_key = ctx.accounts.owner.key();
        let mint_key = ctx.accounts.mint.key();
        let bump = ctx.accounts.vault.bump;
        let seeds: &[&[u8]] = &[b"vault", owner_key.as_ref(), mint_key.as_ref(), &[bump]];
        let signer_seeds = &[seeds];

        let cpi_accounts = Transfer {
            from: ctx.accounts.vault_token_account.to_account_info(),
            to: ctx.accounts.owner_token_account.to_account_info(),
            authority: ctx.accounts.vault.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer_seeds,
        );
        token::transfer(cpi_ctx, amount)?;

        let vault = &mut ctx.accounts.vault;
        vault.amount = vault
            .amount
            .checked_sub(amount)
            .ok_or(VaultError::MathOverflow)?;

        msg!("Withdrew {} — vault now holds {}", amount, vault.amount);
        Ok(())
    }
}

// ============================================================================
//  ACCOUNT CONTEXTS
//  Solana programs are stateless: every account an instruction touches must be
//  listed up front. These structs are that list — and Anchor turns each
//  attribute below into a runtime security check.
// ============================================================================

#[derive(Accounts)]
pub struct InitializeVault<'info> {
    /// The user creating the vault. `mut` because they pay the rent.
    #[account(mut)]
    pub owner: Signer<'info>,

    /// Which SPL token this vault is for (e.g. USDC).
    pub mint: Account<'info, Mint>,

    /// The vault's data account, derived deterministically from
    /// ["vault", owner, mint] — so one user gets exactly one vault per mint.
    #[account(
        init,
        payer = owner,
        space = 8 + Vault::INIT_SPACE, // 8 bytes of Anchor discriminator + fields
        seeds = [b"vault", owner.key().as_ref(), mint.key().as_ref()],
        bump
    )]
    pub vault: Account<'info, Vault>,

    /// The token account that physically holds the deposited tokens.
    /// `token::authority = vault` is the key line: the PDA is the owner,
    /// and a PDA has no private key, so only this program can move funds.
    #[account(
        init,
        payer = owner,
        seeds = [b"vault-token", owner.key().as_ref(), mint.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = vault
    )]
    pub vault_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    pub mint: Account<'info, Mint>,

    /// `has_one` re-checks that the stored owner/mint match the accounts
    /// passed in — cheap insurance against someone swapping accounts.
    #[account(
        mut,
        seeds = [b"vault", owner.key().as_ref(), mint.key().as_ref()],
        bump = vault.bump,
        has_one = owner,
        has_one = mint
    )]
    pub vault: Account<'info, Vault>,

    #[account(
        mut,
        seeds = [b"vault-token", owner.key().as_ref(), mint.key().as_ref()],
        bump
    )]
    pub vault_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = owner_token_account.mint == mint.key() @ VaultError::WrongMint,
        constraint = owner_token_account.owner == owner.key() @ VaultError::WrongOwner
    )]
    pub owner_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    pub mint: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [b"vault", owner.key().as_ref(), mint.key().as_ref()],
        bump = vault.bump,
        has_one = owner,
        has_one = mint
    )]
    pub vault: Account<'info, Vault>,

    #[account(
        mut,
        seeds = [b"vault-token", owner.key().as_ref(), mint.key().as_ref()],
        bump
    )]
    pub vault_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = owner_token_account.mint == mint.key() @ VaultError::WrongMint,
        constraint = owner_token_account.owner == owner.key() @ VaultError::WrongOwner
    )]
    pub owner_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

// ============================================================================
//  STATE
// ============================================================================

/// `InitSpace` makes Anchor compute the byte size of this struct for us:
/// 32 (owner) + 32 (mint) + 8 (amount) + 1 (bump) = 73 bytes, plus the
/// 8-byte discriminator Anchor adds in front of every account.
#[account]
#[derive(InitSpace)]
pub struct Vault {
    pub owner: Pubkey,
    pub mint: Pubkey,
    pub amount: u64,
    pub bump: u8,
}

// ============================================================================
//  ERRORS
// ============================================================================

#[error_code]
pub enum VaultError {
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("Not enough tokens in the vault")]
    InsufficientFunds,
    #[msg("Arithmetic overflow")]
    MathOverflow,
    #[msg("Token account is for a different mint")]
    WrongMint,
    #[msg("Token account belongs to someone else")]
    WrongOwner,
}
