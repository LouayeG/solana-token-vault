# Understanding the token vault, line by line

Read this once with the code open next to it. If you can explain every section
below to someone else, you understand the program well enough to post it.

---

## 0. The mental model you need first

Three ideas do most of the work in Solana:

**1. Programs are stateless.** Your Rust code owns no data. It reads and writes
*accounts* that are passed into it. This is why every instruction starts with a
list of accounts instead of just arguments.

**2. Every account must be declared up front.** A Solana transaction says, in
advance, exactly which accounts it will touch. That's what makes the runtime
fast (it can run non-overlapping transactions in parallel) — and it's why the
`#[derive(Accounts)]` structs exist.

**3. A PDA is an address with no private key.** A Program Derived Address is
derived from some seeds plus the program ID, and it deliberately falls *off* the
ed25519 curve, so no keypair can ever produce it. Only the owning program can
"sign" for it. That is the entire security mechanism of this vault.

---

## 1. Imports and program ID

```rust
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

declare_id!("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");
```

- `anchor_lang::prelude::*` pulls in `Context`, `Account`, `Signer`, `Result`,
  the macros — everything Anchor needs.
- `anchor_spl::token` is Anchor's typed wrapper around the SPL Token program.
  `Mint` and `TokenAccount` let Anchor deserialize and validate those accounts
  for you instead of you parsing raw bytes.
- `declare_id!` is your program's on-chain address. The placeholder is Anchor's
  default; `anchor keys sync` replaces it with your real one after the first build.

---

## 2. `initialize_vault` — creating the two accounts

```rust
pub fn initialize_vault(ctx: Context<InitializeVault>) -> Result<()> {
    let vault = &mut ctx.accounts.vault;
    vault.owner = ctx.accounts.owner.key();
    vault.mint = ctx.accounts.mint.key();
    vault.amount = 0;
    vault.bump = ctx.bumps.vault;
    Ok(())
}
```

`ctx` carries two things: `ctx.accounts` (the validated accounts) and
`ctx.bumps` (the bump bytes Anchor found while deriving each PDA).

**Why store the bump?** Deriving a PDA means hashing seeds and, if the result
lands on the curve, decrementing a bump byte from 255 and trying again. That
search costs compute. Storing the *canonical* bump lets later instructions say
`bump = vault.bump` and skip the search — and it pins the vault to one single
valid address, which closes a real class of bug.

Note how short the function body is. Almost all the interesting work happened in
the account context before this function was ever entered — that's the Anchor style.

### The context

```rust
#[account(
    init,
    payer = owner,
    space = 8 + Vault::INIT_SPACE,
    seeds = [b"vault", owner.key().as_ref(), mint.key().as_ref()],
    bump
)]
pub vault: Account<'info, Vault>,
```

- `init` → create the account via a CPI to the System Program.
- `payer = owner` → the user funds the rent deposit.
- `space` → `8` bytes of Anchor discriminator (a hash tag identifying the
  account type, so you can't pass a `Vault` where a `Mint` is expected) plus the
  struct size that `#[derive(InitSpace)]` computed: 32 + 32 + 8 + 1 = 73 bytes.
- `seeds` + `bump` → make this a PDA. Because the seeds include both the owner
  and the mint, **each user gets exactly one vault per token**, and its address
  can be recomputed by anyone off-chain without a lookup table.

```rust
#[account(
    init,
    payer = owner,
    seeds = [b"vault-token", owner.key().as_ref(), mint.key().as_ref()],
    bump,
    token::mint = mint,
    token::authority = vault
)]
pub vault_token_account: Account<'info, TokenAccount>,
```

This is the account that actually holds the tokens. The line that matters is
`token::authority = vault`. The vault PDA is the authority — and since a PDA has
no private key, the *only* way tokens leave is through the `withdraw` instruction
in this file. Read that sentence again; it's the whole design.

---

## 3. `deposit` — a plain CPI

```rust
let cpi_accounts = Transfer {
    from: ctx.accounts.owner_token_account.to_account_info(),
    to: ctx.accounts.vault_token_account.to_account_info(),
    authority: ctx.accounts.owner.to_account_info(),
};
let cpi_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts);
token::transfer(cpi_ctx, amount)?;
```

Our program does not move tokens itself — it *asks* the SPL Token program to.
That's a **CPI**, one program invoking another inside the same transaction.

The authority here is `owner`, a real wallet that already signed the transaction,
so `CpiContext::new` is enough. The signature comes along for free.

Then the bookkeeping:

```rust
vault.amount = vault.amount.checked_add(amount).ok_or(VaultError::MathOverflow)?;
```

`checked_add` returns `None` instead of wrapping around on overflow. In a
financial program, plain `+` is how you get a headline written about you.

---

## 4. `withdraw` — where PDA signing happens

```rust
let seeds: &[&[u8]] = &[b"vault", owner_key.as_ref(), mint_key.as_ref(), &[bump]];
let signer_seeds = &[seeds];

let cpi_ctx = CpiContext::new_with_signer(
    ctx.accounts.token_program.to_account_info(),
    cpi_accounts,
    signer_seeds,
);
token::transfer(cpi_ctx, amount)?;
```

The tokens are owned by the vault PDA, and no keypair exists for it. So the
program presents the **seeds** instead. The runtime re-derives the address from
those seeds plus the calling program's ID; if it matches the account being used
as authority, the signature is granted.

That is the one idea people usually take longest to absorb, and it's the reason
Solana programs can custody funds safely. Note the seed list is identical to the
one in the `#[account(...)]` attribute, with the bump appended — it has to be
byte-for-byte the same, or the derivation produces a different address and the
transfer fails.

The guard above it matters too:

```rust
require!(ctx.accounts.vault.amount >= amount, VaultError::InsufficientFunds);
```

Check before you transfer, not after.

---

## 5. The constraints are the security model

```rust
#[account(
    mut,
    seeds = [b"vault", owner.key().as_ref(), mint.key().as_ref()],
    bump = vault.bump,
    has_one = owner,
    has_one = mint
)]
pub vault: Account<'info, Vault>,
```

Every attribute is a check that runs before your function body:

- `seeds` + `bump` — this account really is *this* owner's vault for *this* mint.
  If an attacker passes their own pubkey as `owner`, the derived address won't
  match the vault holding your tokens, and the transaction aborts.
- `has_one = owner` — the `owner` field stored inside the account equals the
  `owner` account passed in.
- `Signer<'info>` on `owner` — they actually signed.

And on the user's token account:

```rust
constraint = owner_token_account.mint == mint.key() @ VaultError::WrongMint,
constraint = owner_token_account.owner == owner.key() @ VaultError::WrongOwner
```

Missing account checks are the single most common source of exploits in Solana
programs. Anchor's value is that these checks are declarative and visible in one
place instead of scattered through the logic — or forgotten.

---

## 6. What to try next (this is what makes it *yours*)

1. Add a `close_vault` instruction that returns the rent to the owner.
2. Add a time lock: store an `unlock_at: i64` and compare it to
   `Clock::get()?.unix_timestamp` in `withdraw`.
3. Let the owner nominate a second signer who can also withdraw.
4. Replace the manual `vault.amount` bookkeeping with a read of the token
   account balance — then argue about which is better and why.

Do at least one of these before you post. "I built the tutorial" is fine;
"I built the tutorial and then extended it, and here's what broke" is better.

---

## Glossary

| Term | Meaning |
|---|---|
| **PDA** | Program Derived Address — an address off the ed25519 curve, so only its program can sign for it |
| **Bump** | The byte (255 downward) that pushes a derived address off the curve; the first one that works is the *canonical* bump |
| **CPI** | Cross-Program Invocation — one program calling another mid-transaction |
| **Discriminator** | The 8-byte tag Anchor prefixes to accounts so types can't be confused |
| **Mint** | The SPL account that defines a token (supply, decimals, authority) |
| **Token account** | Holds a balance of one mint for one owner |
| **Rent** | The SOL deposit an account holds to stay alive on chain; refundable on close |
