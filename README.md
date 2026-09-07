# Solana Token Vault

An educational SPL token vault built in Rust with [Anchor](https://www.anchor-lang.com/). A user can create one vault per token mint, deposit tokens, and withdraw them later. The deposited tokens live in a token account controlled by a Program Derived Address (PDA), so there is no private key that can move them outside the program's rules.

> [!WARNING]
> This project is for learning and has not been independently audited. Do not use it to custody valuable assets in production.

## How it works

Each owner/mint pair has two deterministic accounts:

```text
Vault PDA       = ["vault", owner public key, mint public key]
Vault token PDA = ["vault-token", owner public key, mint public key]
```

The **vault PDA** stores the owner, mint, and PDA bump. The **vault token PDA** is an SPL Token account that holds the deposited tokens and names the vault PDA as its authority — and its balance is the single source of truth (there is no separate tracked counter to drift out of sync).

The program exposes four instructions:

| Instruction | Behavior |
| --- | --- |
| `initialize_vault` | Creates both PDAs and records the owner and mint. |
| `deposit(amount)` | Uses a cross-program invocation (CPI) to transfer tokens from the owner's token account into the vault token account. The owner signs. |
| `withdraw(amount)` | Checks ownership and the vault token account balance, then transfers tokens back. The program signs for the vault PDA using its seeds. |
| `close_vault` | Returns any remaining tokens to the owner, closes the vault token account, and closes the data account — refunding all rent to the owner. |

Anchor account constraints enforce that the signer owns the vault, all accounts use the expected mint, and the supplied PDAs match the expected seeds. Amounts must be non-zero, and the vault token account balance is the single source of truth for withdrawals.

## Transaction flow

```text
Initialize: owner -> creates Vault PDA + Vault Token PDA
Deposit:    owner's token account -> Vault Token PDA
Withdraw:   Vault Token PDA -> owner's token account
                                      ^
                    Vault PDA signs with program seeds
Close:      Vault Token PDA -> owner (leftovers), then both PDAs closed (rent -> owner)
```

## Prerequisites

- Rust and Cargo
- Solana CLI
- Anchor CLI `0.31.1`
- Node.js 18 or newer and npm

On Windows, run the Solana and Anchor toolchain inside WSL. Follow the official installation instructions for [Solana](https://solana.com/docs/intro/installation) and [Anchor](https://www.anchor-lang.com/docs/installation).

## Run locally

```bash
npm install
anchor build
anchor keys sync
anchor build
anchor test
```

The first `anchor build` creates a program keypair. `anchor keys sync` writes that generated address into both `Anchor.toml` and `declare_id!` in the Rust source so they agree.

The integration suite covers initialization, deposit, withdrawal, an overdraw attempt, an unauthorized withdrawal attempt, withdrawing tokens sent directly to the vault, and closing the vault.

## Project structure

```text
.
|-- Anchor.toml                         Anchor workspace and localnet settings
|-- Cargo.toml                          Rust workspace configuration
|-- programs/token-vault/
|   |-- Cargo.toml                      On-chain program crate
|   `-- src/lib.rs                      Instructions, accounts, state, and errors
|-- tests/token-vault.ts                Local-validator integration tests
|-- EXPLAINER.md                        Detailed walkthrough of the design
|-- package.json                        JavaScript tooling and dependencies
`-- tsconfig.json                       TypeScript test configuration
```

For a deeper explanation of PDAs, CPI signing, account constraints, and every major code section, read [EXPLAINER.md](EXPLAINER.md).

## Security notes

- The vault is scoped to one owner and one mint.
- Withdrawals require the recorded owner to sign.
- The SPL Token program performs all token transfers.
- The vault token account balance is the single source of truth, so tokens sent directly to it remain withdrawable by the owner.
- There is no emergency recovery path, upgrade policy, or production audit — this is an educational program, not production-hardened.

## License

Licensed under the [MIT License](LICENSE).
