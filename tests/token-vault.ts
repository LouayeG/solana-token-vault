import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { TokenVault } from "../target/types/token_vault";
import {
  TOKEN_PROGRAM_ID,
  createMint,
  createAccount,
  mintTo,
  getAccount,
} from "@solana/spl-token";
import { PublicKey, Keypair, SystemProgram } from "@solana/web3.js";
import { assert } from "chai";

describe("token-vault", () => {
  // Uses the wallet + cluster from Anchor.toml.
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.tokenVault as Program<TokenVault>;
  const owner = provider.wallet as anchor.Wallet;

  let mint: PublicKey;
  let ownerTokenAccount: PublicKey;
  let vaultPda: PublicKey;
  let vaultTokenPda: PublicKey;

  before(async () => {
    // 1. Create a brand new test token with 6 decimals.
    mint = await createMint(
      provider.connection,
      owner.payer,
      owner.publicKey, // mint authority
      null,            // freeze authority
      6
    );

    // 2. Give ourselves a token account and mint 1,000 tokens into it.
    ownerTokenAccount = await createAccount(
      provider.connection,
      owner.payer,
      mint,
      owner.publicKey
    );
    await mintTo(
      provider.connection,
      owner.payer,
      mint,
      ownerTokenAccount,
      owner.payer,
      1_000_000_000 // 1,000 tokens at 6 decimals
    );

    // 3. Derive the same PDAs the Rust program derives.
    [vaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), owner.publicKey.toBuffer(), mint.toBuffer()],
      program.programId
    );
    [vaultTokenPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault-token"), owner.publicKey.toBuffer(), mint.toBuffer()],
      program.programId
    );
  });

  it("initializes the vault", async () => {
    await program.methods
      .initializeVault()
      .accounts({
        owner: owner.publicKey,
        mint,
        vault: vaultPda,
        vaultTokenAccount: vaultTokenPda,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const vault = await program.account.vault.fetch(vaultPda);
    assert.equal(vault.owner.toBase58(), owner.publicKey.toBase58());
    assert.equal(vault.amount.toNumber(), 0);
  });

  it("deposits 100 tokens", async () => {
    await program.methods
      .deposit(new anchor.BN(100_000_000))
      .accounts({
        owner: owner.publicKey,
        mint,
        vault: vaultPda,
        vaultTokenAccount: vaultTokenPda,
        ownerTokenAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    const vault = await program.account.vault.fetch(vaultPda);
    const vaultTokens = await getAccount(provider.connection, vaultTokenPda);
    assert.equal(vault.amount.toNumber(), 100_000_000);
    assert.equal(Number(vaultTokens.amount), 100_000_000);
  });

  it("withdraws 40 tokens", async () => {
    await program.methods
      .withdraw(new anchor.BN(40_000_000))
      .accounts({
        owner: owner.publicKey,
        mint,
        vault: vaultPda,
        vaultTokenAccount: vaultTokenPda,
        ownerTokenAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    const vault = await program.account.vault.fetch(vaultPda);
    assert.equal(vault.amount.toNumber(), 60_000_000);
  });

  it("refuses to withdraw more than the vault holds", async () => {
    let rejected = false;

    try {
      await program.methods
        .withdraw(new anchor.BN(999_000_000))
        .accounts({
          owner: owner.publicKey,
          mint,
          vault: vaultPda,
          vaultTokenAccount: vaultTokenPda,
          ownerTokenAccount,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .rpc();
    } catch (err: any) {
      rejected = true;
      assert.include(err.toString(), "InsufficientFunds");
    }

    assert.isTrue(rejected, "an overdraw should be rejected");
  });

  it("refuses a stranger's withdrawal", async () => {
    const attacker = Keypair.generate();
    const sig = await provider.connection.requestAirdrop(attacker.publicKey, 1e9);
    await provider.connection.confirmTransaction(sig);

    const attackerTokenAccount = await createAccount(
      provider.connection,
      attacker,
      mint,
      attacker.publicKey
    );

    let rejected = false;

    try {
      await program.methods
        .withdraw(new anchor.BN(1_000_000))
        .accounts({
          owner: attacker.publicKey,
          mint,
          vault: vaultPda,
          vaultTokenAccount: vaultTokenPda,
          ownerTokenAccount: attackerTokenAccount,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([attacker])
        .rpc();
    } catch (err) {
      // Expected: the PDA seeds derived from the attacker's key don't match
      // the existing vault account, so the constraint fails.
      rejected = true;
    }

    assert.isTrue(rejected, "an attacker must not be able to drain the vault");
  });
});
