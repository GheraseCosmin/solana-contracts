
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import {
    ASSOCIATED_TOKEN_PROGRAM_ID,
    createMint,
    getAccount,
    getAssociatedTokenAddress,
    getOrCreateAssociatedTokenAccount,
    mintTo,
    TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { strict as assert } from "assert";
import { Ico } from "../target/types/ico";

const { SystemProgram, LAMPORTS_PER_SOL, PublicKey, Keypair } = anchor.web3;

describe("ico", () => {
  // Configure the client to use the local cluster.
  anchor.setProvider(anchor.AnchorProvider.env());

  const provider = anchor.getProvider() as anchor.AnchorProvider;
  const program = anchor.workspace.ico as Program<Ico>;

  it("contribute updates presale and profile state", async () => {
    const mint = await createMint(
      provider.connection,
      provider.wallet.payer,
      provider.wallet.publicKey,
      null,
      9
    );

    const authorityAta = await getOrCreateAssociatedTokenAccount(
      provider.connection,
      provider.wallet.payer,
      mint,
      provider.wallet.publicKey
    );

    const tokensForSaleNumber = 10 * 10 ** 9;
    await mintTo(
      provider.connection,
      provider.wallet.payer,
      mint,
      authorityAta.address,
      provider.wallet.publicKey,
      tokensForSaleNumber
    );

    const poolId = new anchor.BN(1);
    const tokenPriceLamports = new anchor.BN(LAMPORTS_PER_SOL);
    const softCap = new anchor.BN(0.5 * LAMPORTS_PER_SOL);
    const hardCap = new anchor.BN(2 * LAMPORTS_PER_SOL);
    const minContribution = new anchor.BN(0.1 * LAMPORTS_PER_SOL);
    const maxContribution = new anchor.BN(2 * LAMPORTS_PER_SOL);

    const now = Math.floor(Date.now() / 1000);
    const startTimestamp = new anchor.BN(now - 60);
    const endTimestamp = new anchor.BN(now + 3600);

    const [presalePda] = PublicKey.findProgramAddressSync(
      [Buffer.from("state"), poolId.toArrayLike(Buffer, "le", 8)],
      program.programId
    );
    const [presaleVaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), presalePda.toBuffer()],
      program.programId
    );

    await program.methods
      .createPresalePool(
        poolId,
        tokenPriceLamports,
        softCap,
        hardCap,
        minContribution,
        maxContribution,
        startTimestamp,
        endTimestamp,
        new anchor.BN(tokensForSaleNumber)
      )
      .accounts({
        authority: provider.wallet.publicKey,
        fundsReceiver: provider.wallet.publicKey,
        tokenMint: mint,
        authorityTokenAccount: authorityAta.address,
      })
      .rpc();

    const contributor = Keypair.generate();
    const airdropSig = await provider.connection.requestAirdrop(
      contributor.publicKey,
      2 * LAMPORTS_PER_SOL
    );
    await provider.connection.confirmTransaction(airdropSig);

    const [profilePda] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("contributor-profile"),
        contributor.publicKey.toBuffer(),
        presalePda.toBuffer(),
      ],
      program.programId
    );

    const contribution = new anchor.BN(1 * LAMPORTS_PER_SOL);

    await program.methods
      .contribute(contribution)
      .accountsStrict({
        contributor: contributor.publicKey,
        presale: presalePda,
        profile: profilePda,
        systemProgram: SystemProgram.programId,
      })
      .signers([contributor])
      .rpc();

    const presaleAccount = await program.account.presalePool.fetch(presalePda);
    const profileAccount = await program.account.contributorProfile.fetch(
      profilePda
    );

    assert.ok(presaleAccount.totalContributions.eq(contribution));
    assert.ok(profileAccount.contributed.eq(contribution));
  });

  it("claim tokens sends bought tokens to contributor", async () => {
    const mint = await createMint(
      provider.connection,
      provider.wallet.payer,
      provider.wallet.publicKey,
      null,
      9
    );

    const authorityAta = await getOrCreateAssociatedTokenAccount(
      provider.connection,
      provider.wallet.payer,
      mint,
      provider.wallet.publicKey
    );

    const tokensForSaleNumber = 10 * 10 ** 9; // 10 tokens, 9 decimals
    await mintTo(
      provider.connection,
      provider.wallet.payer,
      mint,
      authorityAta.address,
      provider.wallet.publicKey,
      tokensForSaleNumber
    );

    const poolId = new anchor.BN(2);
    const tokenPriceLamports = new anchor.BN(LAMPORTS_PER_SOL); // 1 token = 1 SOL
    const softCap = new anchor.BN(0.5 * LAMPORTS_PER_SOL);
    const hardCap = new anchor.BN(2 * LAMPORTS_PER_SOL);
    const minContribution = new anchor.BN(0.1 * LAMPORTS_PER_SOL);
    const maxContribution = new anchor.BN(2 * LAMPORTS_PER_SOL);

    const now = Math.floor(Date.now() / 1000);
    const startTimestamp = new anchor.BN(now - 60);
    const endTimestamp = new anchor.BN(now + 3600);

    const [presalePda] = PublicKey.findProgramAddressSync(
      [Buffer.from("state"), poolId.toArrayLike(Buffer, "le", 8)],
      program.programId
    );
    const [presaleVaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), presalePda.toBuffer()],
      program.programId
    );

    await program.methods
      .createPresalePool(
        poolId,
        tokenPriceLamports,
        softCap,
        hardCap,
        minContribution,
        maxContribution,
        startTimestamp,
        endTimestamp,
        new anchor.BN(tokensForSaleNumber)
      )
      .accounts({
        authority: provider.wallet.publicKey,
        fundsReceiver: provider.wallet.publicKey,
        tokenMint: mint,
        authorityTokenAccount: authorityAta.address,
      })
      .rpc();

    const contributor = Keypair.generate();
    const airdropSig = await provider.connection.requestAirdrop(
      contributor.publicKey,
      2 * LAMPORTS_PER_SOL
    );
    await provider.connection.confirmTransaction(airdropSig);

    const [profilePda] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("contributor-profile"),
        contributor.publicKey.toBuffer(),
        presalePda.toBuffer(),
      ],
      program.programId
    );

    const contribution = new anchor.BN(1 * LAMPORTS_PER_SOL);

    await program.methods
      .contribute(contribution)
      .accountsStrict({
        contributor: contributor.publicKey,
        presale: presalePda,
        profile: profilePda,
        systemProgram: SystemProgram.programId,
      })
      .signers([contributor])
      .rpc();

    const contributorAta = await getAssociatedTokenAddress(
      mint,
      contributor.publicKey
    );

    await program.methods
      .claim()
      .accountsStrict({
        contributor: contributor.publicKey,
        presale: presalePda,
        profile: profilePda,
        tokenMint: mint,
        presaleVault: presaleVaultPda,
        contributorAta: contributorAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .signers([contributor])
      .rpc();

    const contributorAtaAccount = await getAccount(
      provider.connection,
      contributorAta
    );

    assert.ok(contributorAtaAccount.amount > BigInt(0));
  });

  it("admin withdraw sends SOL to funds receiver", async () => {
    const mint = await createMint(
      provider.connection,
      provider.wallet.payer,
      provider.wallet.publicKey,
      null,
      9
    );

    const authorityAta = await getOrCreateAssociatedTokenAccount(
      provider.connection,
      provider.wallet.payer,
      mint,
      provider.wallet.publicKey
    );

    const tokensForSaleNumber = 10 * 10 ** 9;
    await mintTo(
      provider.connection,
      provider.wallet.payer,
      mint,
      authorityAta.address,
      provider.wallet.publicKey,
      tokensForSaleNumber
    );

    const poolId = new anchor.BN(3);
    const tokenPriceLamports = new anchor.BN(LAMPORTS_PER_SOL);
    const softCap = new anchor.BN(0.5 * LAMPORTS_PER_SOL);
    const hardCap = new anchor.BN(2 * LAMPORTS_PER_SOL);
    const minContribution = new anchor.BN(0.1 * LAMPORTS_PER_SOL);
    const maxContribution = new anchor.BN(2 * LAMPORTS_PER_SOL);

    const now = Math.floor(Date.now() / 1000);
    const startTimestamp = new anchor.BN(now - 60);
    const endTimestamp = new anchor.BN(now + 3600);

    const [presalePda] = PublicKey.findProgramAddressSync(
      [Buffer.from("state"), poolId.toArrayLike(Buffer, "le", 8)],
      program.programId
    );
    const [presaleVaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), presalePda.toBuffer()],
      program.programId
    );

    await program.methods
      .createPresalePool(
        poolId,
        tokenPriceLamports,
        softCap,
        hardCap,
        minContribution,
        maxContribution,
        startTimestamp,
        endTimestamp,
        new anchor.BN(tokensForSaleNumber)
      )
      .accounts({
        authority: provider.wallet.publicKey,
        fundsReceiver: provider.wallet.publicKey,
        tokenMint: mint,
        authorityTokenAccount: authorityAta.address,
      })
      .rpc();

    const contributor = Keypair.generate();
    const airdropSig = await provider.connection.requestAirdrop(
      contributor.publicKey,
      2 * LAMPORTS_PER_SOL
    );
    await provider.connection.confirmTransaction(airdropSig);

    const [profilePda] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("contributor-profile"),
        contributor.publicKey.toBuffer(),
        presalePda.toBuffer(),
      ],
      program.programId
    );

    const contribution = new anchor.BN(1 * LAMPORTS_PER_SOL);

    await program.methods
      .contribute(contribution)
      .accountsStrict({
        contributor: contributor.publicKey,
        presale: presalePda,
        profile: profilePda,
        systemProgram: SystemProgram.programId,
      })
      .signers([contributor])
      .rpc();

    const balanceBefore = await provider.connection.getBalance(
      provider.wallet.publicKey
    );

    await program.methods
      .adminWithdraw(new anchor.BN(LAMPORTS_PER_SOL))
      .accountsStrict({
        presale: presalePda,
        authority: provider.wallet.publicKey,
        fundsReceiver: provider.wallet.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const balanceAfter = await provider.connection.getBalance(
      provider.wallet.publicKey
    );

    assert.ok(balanceAfter > balanceBefore);
  });

  it("refunds when soft cap not reached", async () => {
    const mint = await createMint(
      provider.connection,
      provider.wallet.payer,
      provider.wallet.publicKey,
      null,
      9
    );

    const authorityAta = await getOrCreateAssociatedTokenAccount(
      provider.connection,
      provider.wallet.payer,
      mint,
      provider.wallet.publicKey
    );

    const tokensForSaleNumber = 10 * 10 ** 9;
    await mintTo(
      provider.connection,
      provider.wallet.payer,
      mint,
      authorityAta.address,
      provider.wallet.publicKey,
      tokensForSaleNumber
    );

    const poolId = new anchor.BN(4);
    const tokenPriceLamports = new anchor.BN(LAMPORTS_PER_SOL);
    const softCap = new anchor.BN(2 * LAMPORTS_PER_SOL); // not reached
    const hardCap = new anchor.BN(5 * LAMPORTS_PER_SOL);
    const minContribution = new anchor.BN(0.1 * LAMPORTS_PER_SOL);
    const maxContribution = new anchor.BN(5 * LAMPORTS_PER_SOL);

    const now = Math.floor(Date.now() / 1000);
    const startTimestamp = new anchor.BN(now - 60);
    const endTimestamp = new anchor.BN(now + 3600);

    const [presalePda] = PublicKey.findProgramAddressSync(
      [Buffer.from("state"), poolId.toArrayLike(Buffer, "le", 8)],
      program.programId
    );
    const [presaleVaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), presalePda.toBuffer()],
      program.programId
    );

    await program.methods
      .createPresalePool(
        poolId,
        tokenPriceLamports,
        softCap,
        hardCap,
        minContribution,
        maxContribution,
        startTimestamp,
        endTimestamp,
        new anchor.BN(tokensForSaleNumber)
      )
      .accounts({
        authority: provider.wallet.publicKey,
        fundsReceiver: provider.wallet.publicKey,
        tokenMint: mint,
        authorityTokenAccount: authorityAta.address,
      })
      .rpc();

    const contributor = Keypair.generate();
    const airdropSig = await provider.connection.requestAirdrop(
      contributor.publicKey,
      2 * LAMPORTS_PER_SOL
    );
    await provider.connection.confirmTransaction(airdropSig);

    const [profilePda] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("contributor-profile"),
        contributor.publicKey.toBuffer(),
        presalePda.toBuffer(),
      ],
      program.programId
    );

    const contribution = new anchor.BN(1 * LAMPORTS_PER_SOL); // < soft cap

    await program.methods
      .contribute(contribution)
      .accountsStrict({
        contributor: contributor.publicKey,
        presale: presalePda,
        profile: profilePda,
        systemProgram: SystemProgram.programId,
      })
      .signers([contributor])
      .rpc();

    const balanceBefore = await provider.connection.getBalance(
      contributor.publicKey
    );

    const contributorAta = await getAssociatedTokenAddress(
      mint,
      contributor.publicKey
    );

    await program.methods
      .claim()
      .accountsStrict({
        contributor: contributor.publicKey,
        presale: presalePda,
        profile: profilePda,
        tokenMint: mint,
        presaleVault: presaleVaultPda,
        contributorAta: contributorAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .signers([contributor])
      .rpc();

    const balanceAfter = await provider.connection.getBalance(
      contributor.publicKey
    );

    assert.ok(balanceAfter > balanceBefore);
  });
});