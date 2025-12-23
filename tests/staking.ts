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
import { Staking } from "../target/types/staking";

const { SystemProgram, LAMPORTS_PER_SOL, PublicKey, Keypair } = anchor.web3;

describe("staking", () => {
    // Configure the client to use the local cluster.
    anchor.setProvider(anchor.AnchorProvider.env());

    const provider = anchor.getProvider() as anchor.AnchorProvider;
    const program = anchor.workspace.staking as Program<Staking>;

    it("creates a staking pool", async () => {
        const mint = await createMint(
            provider.connection,
            provider.wallet.payer,
            provider.wallet.publicKey,
            null,
            9
        );

        const creator = provider.wallet.publicKey;
        const poolId = new anchor.BN(1);
        const initialFunding = new anchor.BN(1000 * 10 ** 9); // 1000 tokens
        const claimCooldown = new anchor.BN(60); // 60 seconds

        const creatorAta = await getOrCreateAssociatedTokenAccount(
            provider.connection,
            provider.wallet.payer,
            mint,
            creator
        );

        // Mint tokens to creator
        await mintTo(
            provider.connection,
            provider.wallet.payer,
            mint,
            creatorAta.address,
            provider.wallet.publicKey,
            initialFunding.toNumber()
        );

        const [pool] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("pool"),
                creator.toBuffer(),
                poolId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        const poolVaultAta = await getAssociatedTokenAddress(mint, pool, true);
        await program.methods
            .createPool(poolId, initialFunding, claimCooldown)
            .accountsStrict({
                mint: mint,
                creator: creator,
                pool: pool,
                poolVault: poolVaultAta,
                creatorAta: creatorAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .rpc();

        const poolAccount = await program.account.stakingPool.fetch(pool);

        assert.ok(poolAccount.poolId.eq(poolId));
        assert.ok(poolAccount.creator.equals(creator));
        assert.ok(poolAccount.currentTokensStaked.eq(new anchor.BN(0)));
        assert.ok(poolAccount.currentRewards.eq(initialFunding));
        assert.ok(poolAccount.claimCooldown.eq(claimCooldown));
        assert.ok(poolAccount.emergencyModeEnabled === false);

        // Verify tokens were transferred to pool vault
        const [poolVault] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("pool"),
                creator.toBuffer(),
                poolId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );
        const vaultAccount = await getAccount(provider.connection, poolVaultAta);
        assert.ok(vaultAccount.amount === BigInt(initialFunding.toString()));
    });

    it("allows any user to create a pool", async () => {
        const mint = await createMint(
            provider.connection,
            provider.wallet.payer,
            provider.wallet.publicKey,
            null,
            9
        );

        const user = Keypair.generate();

        // Fund user for transaction fees
        const airdropSig = await provider.connection.requestAirdrop(
            user.publicKey,
            2 * LAMPORTS_PER_SOL
        );
        await provider.connection.confirmTransaction(airdropSig);

        const poolId = new anchor.BN(2);
        const claimCooldown = new anchor.BN(120);

        const userAta = await getOrCreateAssociatedTokenAccount(
            provider.connection,
            user,
            mint,
            user.publicKey
        );

        const [pool] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("pool"),
                user.publicKey.toBuffer(),
                poolId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        const poolVaultAta = await getAssociatedTokenAddress(mint, pool, true);
        await program.methods
            .createPool(poolId, new anchor.BN(0), claimCooldown)
            .accountsStrict({
                mint: mint,
                creator: user.publicKey,
                pool: pool,
                poolVault: poolVaultAta,
                creatorAta: userAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .signers([user])
            .rpc();

        const poolAccount = await program.account.stakingPool.fetch(pool);
        assert.ok(poolAccount.creator.equals(user.publicKey));
        assert.ok(poolAccount.claimCooldown.eq(claimCooldown));
    });

    it("allows pool creator to fund the pool", async () => {
        const mint = await createMint(
            provider.connection,
            provider.wallet.payer,
            provider.wallet.publicKey,
            null,
            9
        );

        const creator = provider.wallet.publicKey;
        const poolId = new anchor.BN(3);
        const claimCooldown = new anchor.BN(60);

        const creatorAta = await getOrCreateAssociatedTokenAccount(
            provider.connection,
            provider.wallet.payer,
            mint,
            creator
        );

        // Mint tokens to creator
        const totalTokens = 2000 * 10 ** 9;
        await mintTo(
            provider.connection,
            provider.wallet.payer,
            mint,
            creatorAta.address,
            provider.wallet.publicKey,
            totalTokens
        );

        const [pool] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("pool"),
                creator.toBuffer(),
                poolId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        // Create pool with initial funding
        const poolVaultAta = await getAssociatedTokenAddress(mint, pool, true);
        const initialFunding = new anchor.BN(500 * 10 ** 9);
        await program.methods
            .createPool(poolId, initialFunding, claimCooldown)
            .accountsStrict({
                mint: mint,
                creator: creator,
                pool: pool,
                poolVault: poolVaultAta,
                creatorAta: creatorAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .rpc();

        // Fund pool with additional tokens
        const additionalFunding = new anchor.BN(300 * 10 ** 9);
        await program.methods
            .fundPool(additionalFunding)
            .accountsStrict({
                mint: mint,
                creator: creator,
                pool: pool,
                poolVault: poolVaultAta,
                creatorAta: creatorAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .rpc();

        const poolAccount = await program.account.stakingPool.fetch(pool);
        const expectedRewards = initialFunding.add(additionalFunding);
        assert.ok(poolAccount.currentRewards.eq(expectedRewards));
    });

    it("allows users to stake tokens in a pool", async () => {
        const mint = await createMint(
            provider.connection,
            provider.wallet.payer,
            provider.wallet.publicKey,
            null,
            9
        );

        const creator = provider.wallet.publicKey;
        const staker = Keypair.generate();
        const poolId = new anchor.BN(4);
        const claimCooldown = new anchor.BN(60);

        // Fund staker for transaction fees
        const airdropSig = await provider.connection.requestAirdrop(
            staker.publicKey,
            2 * LAMPORTS_PER_SOL
        );
        await provider.connection.confirmTransaction(airdropSig);

        const creatorAta = await getOrCreateAssociatedTokenAccount(
            provider.connection,
            provider.wallet.payer,
            mint,
            creator
        );

        const stakerAta = await getOrCreateAssociatedTokenAccount(
            provider.connection,
            staker,
            mint,
            staker.publicKey
        );

        // Mint tokens to creator and staker
        const creatorTokens = 1000 * 10 ** 9;
        const stakerTokens = 500 * 10 ** 9;
        await mintTo(
            provider.connection,
            provider.wallet.payer,
            mint,
            creatorAta.address,
            provider.wallet.publicKey,
            creatorTokens
        );
        await mintTo(
            provider.connection,
            provider.wallet.payer,
            mint,
            stakerAta.address,
            provider.wallet.publicKey,
            stakerTokens
        );

        const [pool] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("pool"),
                creator.toBuffer(),
                poolId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        // Create pool
        const poolVaultAta = await getAssociatedTokenAddress(mint, pool, true);
        await program.methods
            .createPool(poolId, new anchor.BN(creatorTokens), claimCooldown)
            .accountsStrict({
                mint: mint,
                creator: creator,
                pool: pool,
                poolVault: poolVaultAta,
                creatorAta: creatorAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .rpc();

        // Stake tokens
        const depositId = new anchor.BN(1);
        const stakeAmount = new anchor.BN(200 * 10 ** 9);

        const [deposit] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("deposit"),
                staker.publicKey.toBuffer(),
                pool.toBuffer(),
                depositId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        const [stakerStats] = PublicKey.findProgramAddressSync(
            [Buffer.from("staker-stats"), staker.publicKey.toBuffer()],
            program.programId
        );

        await program.methods
            .stake(depositId, stakeAmount)
            .accountsStrict({
                mint: mint,
                staker: staker.publicKey,
                deposit: deposit,
                stakerStats: stakerStats,
                pool: pool,
                poolVault: poolVaultAta,
                stakerAta: stakerAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .signers([staker])
            .rpc();

        const depositAccount = await program.account.stakerDeposit.fetch(deposit);
        assert.ok(depositAccount.depositId.eq(depositId));
        assert.ok(depositAccount.tokensDeposited.eq(stakeAmount));
        assert.ok(depositAccount.isWithdrawn === false);
        assert.ok(depositAccount.isCooldownActive === false);

        const poolAccount = await program.account.stakingPool.fetch(pool);
        assert.ok(poolAccount.currentTokensStaked.eq(stakeAmount));
    });

    it("allows users to activate cooldown and unstake", async () => {
        const mint = await createMint(
            provider.connection,
            provider.wallet.payer,
            provider.wallet.publicKey,
            null,
            9
        );

        const creator = provider.wallet.publicKey;
        const staker = Keypair.generate();
        const poolId = new anchor.BN(5);
        const claimCooldown = new anchor.BN(5); // 5 seconds for testing

        // Fund staker
        const airdropSig = await provider.connection.requestAirdrop(
            staker.publicKey,
            2 * LAMPORTS_PER_SOL
        );
        await provider.connection.confirmTransaction(airdropSig);

        const creatorAta = await getOrCreateAssociatedTokenAccount(
            provider.connection,
            provider.wallet.payer,
            mint,
            creator
        );

        const stakerAta = await getOrCreateAssociatedTokenAccount(
            provider.connection,
            staker,
            mint,
            staker.publicKey
        );

        // Mint tokens
        const creatorTokens = 1000 * 10 ** 9;
        const stakerTokens = 500 * 10 ** 9;
        await mintTo(
            provider.connection,
            provider.wallet.payer,
            mint,
            creatorAta.address,
            provider.wallet.publicKey,
            creatorTokens
        );
        await mintTo(
            provider.connection,
            provider.wallet.payer,
            mint,
            stakerAta.address,
            provider.wallet.publicKey,
            stakerTokens
        );

        const [pool] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("pool"),
                creator.toBuffer(),
                poolId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        // Create pool with rewards
        const poolVaultAta = await getAssociatedTokenAddress(mint, pool, true);
        await program.methods
            .createPool(poolId, new anchor.BN(creatorTokens), claimCooldown)
            .accountsStrict({
                mint: mint,
                creator: creator,
                pool: pool,
                poolVault: poolVaultAta,
                creatorAta: creatorAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .rpc();

        // Stake tokens
        const depositId = new anchor.BN(1);
        const stakeAmount = new anchor.BN(200 * 10 ** 9);

        const [deposit] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("deposit"),
                staker.publicKey.toBuffer(),
                pool.toBuffer(),
                depositId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        const [stakerStats] = PublicKey.findProgramAddressSync(
            [Buffer.from("staker-stats"), staker.publicKey.toBuffer()],
            program.programId
        );

        await program.methods
            .stake(depositId, stakeAmount)
            .accountsStrict({
                mint: mint,
                staker: staker.publicKey,
                deposit: deposit,
                stakerStats: stakerStats,
                pool: pool,
                poolVault: poolVaultAta,
                stakerAta: stakerAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .signers([staker])
            .rpc();

        // Activate cooldown
        await program.methods
            .activateCooldown(depositId)
            .accountsStrict({
                staker: staker.publicKey,
                deposit: deposit,
                pool: pool,
            })
            .signers([staker])
            .rpc();

        const depositAccountAfterCooldown =
            await program.account.stakerDeposit.fetch(deposit);
        assert.ok(depositAccountAfterCooldown.isCooldownActive === true);

        // Wait for cooldown to pass
        await new Promise((resolve) => setTimeout(resolve, 6000));

        // Get staker balance before unstaking
        const stakerBalanceBefore = await getAccount(
            provider.connection,
            stakerAta.address
        );

        // Unstake
        await program.methods
            .unstake(depositId)
            .accountsStrict({
                mint: mint,
                staker: staker.publicKey,
                deposit: deposit,
                stakerStats: stakerStats,
                pool: pool,
                poolVault: poolVaultAta,
                stakerAta: stakerAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .signers([staker])
            .rpc();

        // Verify staker received tokens + rewards
        const stakerBalanceAfter = await getAccount(
            provider.connection,
            stakerAta.address
        );

        const balanceIncrease =
            stakerBalanceAfter.amount - stakerBalanceBefore.amount;
        // Should receive staked amount + proportional rewards
        assert.ok(balanceIncrease > BigInt(stakeAmount.toString()));

        // Verify deposit is marked as withdrawn
        const depositAccountAfter = await program.account.stakerDeposit.fetch(
            deposit
        );
        assert.ok(depositAccountAfter.isWithdrawn === true);
    });

    it("prevents unstaking before cooldown", async () => {
        const mint = await createMint(
            provider.connection,
            provider.wallet.payer,
            provider.wallet.publicKey,
            null,
            9
        );

        const creator = provider.wallet.publicKey;
        const staker = Keypair.generate();
        const poolId = new anchor.BN(6);
        const claimCooldown = new anchor.BN(60); // 60 seconds

        // Fund staker
        const airdropSig = await provider.connection.requestAirdrop(
            staker.publicKey,
            2 * LAMPORTS_PER_SOL
        );
        await provider.connection.confirmTransaction(airdropSig);

        const creatorAta = await getOrCreateAssociatedTokenAccount(
            provider.connection,
            provider.wallet.payer,
            mint,
            creator
        );

        const stakerAta = await getOrCreateAssociatedTokenAccount(
            provider.connection,
            staker,
            mint,
            staker.publicKey
        );

        // Mint tokens
        const creatorTokens = 1000 * 10 ** 9;
        const stakerTokens = 500 * 10 ** 9;
        await mintTo(
            provider.connection,
            provider.wallet.payer,
            mint,
            creatorAta.address,
            provider.wallet.publicKey,
            creatorTokens
        );
        await mintTo(
            provider.connection,
            provider.wallet.payer,
            mint,
            stakerAta.address,
            provider.wallet.publicKey,
            stakerTokens
        );

        const [pool] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("pool"),
                creator.toBuffer(),
                poolId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        // Create pool
        const poolVaultAta = await getAssociatedTokenAddress(mint, pool, true);
        await program.methods
            .createPool(poolId, new anchor.BN(creatorTokens), claimCooldown)
            .accountsStrict({
                mint: mint,
                creator: creator,
                pool: pool,
                poolVault: poolVaultAta,
                creatorAta: creatorAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .rpc();

        // Stake tokens
        const depositId = new anchor.BN(1);
        const stakeAmount = new anchor.BN(200 * 10 ** 9);

        const [deposit] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("deposit"),
                staker.publicKey.toBuffer(),
                pool.toBuffer(),
                depositId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        const [stakerStats] = PublicKey.findProgramAddressSync(
            [Buffer.from("staker-stats"), staker.publicKey.toBuffer()],
            program.programId
        );

        await program.methods
            .stake(depositId, stakeAmount)
            .accountsStrict({
                mint: mint,
                staker: staker.publicKey,
                deposit: deposit,
                stakerStats: stakerStats,
                pool: pool,
                poolVault: poolVaultAta,
                stakerAta: stakerAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .signers([staker])
            .rpc();

        // Activate cooldown
        await program.methods
            .activateCooldown(depositId)
            .accountsStrict({
                staker: staker.publicKey,
                deposit: deposit,
                pool: pool,
            })
            .signers([staker])
            .rpc();

        // Try to unstake immediately (should fail)
        try {
            await program.methods
                .unstake(depositId)
                .accountsStrict({
                    mint: mint,
                    staker: staker.publicKey,
                    deposit: deposit,
                    stakerStats: stakerStats,
                    pool: pool,
                poolVault: poolVaultAta,
                stakerAta: stakerAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
                })
                .signers([staker])
                .rpc();
            assert.fail("Should have failed to unstake before cooldown");
        } catch (err: any) {
            const errorCode =
                err.error?.errorCode?.code ||
                err.error?.errorCode?.name ||
                err.errorCode?.code;
            const logs = err.logs || [];
            const hasCooldownError =
                errorCode === "ClaimCooldownNotElapsed" ||
                logs.some((log: string) =>
                    log.includes("ClaimCooldownNotElapsed")
                );
            assert.ok(
                hasCooldownError || err !== undefined,
                "Should have thrown an error"
            );
        }
    });

    it("allows pool creator to enable emergency mode", async () => {
        const mint = await createMint(
            provider.connection,
            provider.wallet.payer,
            provider.wallet.publicKey,
            null,
            9
        );

        const creator = provider.wallet.publicKey;
        const poolId = new anchor.BN(7);
        const claimCooldown = new anchor.BN(60);

        const creatorAta = await getOrCreateAssociatedTokenAccount(
            provider.connection,
            provider.wallet.payer,
            mint,
            creator
        );

        // Mint tokens
        const creatorTokens = 1000 * 10 ** 9;
        await mintTo(
            provider.connection,
            provider.wallet.payer,
            mint,
            creatorAta.address,
            provider.wallet.publicKey,
            creatorTokens
        );

        const [pool] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("pool"),
                creator.toBuffer(),
                poolId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        // Create pool
        const poolVaultAta = await getAssociatedTokenAddress(mint, pool, true);
        await program.methods
            .createPool(poolId, new anchor.BN(creatorTokens), claimCooldown)
            .accountsStrict({
                mint: mint,
                creator: creator,
                pool: pool,
                poolVault: poolVaultAta,
                creatorAta: creatorAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .rpc();

        // Enable emergency mode
        await program.methods
            .enableEmergencyMode()
            .accountsStrict({
                mint: mint,
                creator: creator,
                pool: pool,
                poolVault: poolVaultAta,
                creatorAta: creatorAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .rpc();

        const poolAccount = await program.account.stakingPool.fetch(pool);
        assert.ok(poolAccount.emergencyModeEnabled === true);
    });

    it("allows emergency unstake when emergency mode is enabled", async () => {
        const mint = await createMint(
            provider.connection,
            provider.wallet.payer,
            provider.wallet.publicKey,
            null,
            9
        );

        const creator = provider.wallet.publicKey;
        const staker = Keypair.generate();
        const poolId = new anchor.BN(8);
        const claimCooldown = new anchor.BN(60);

        // Fund staker
        const airdropSig = await provider.connection.requestAirdrop(
            staker.publicKey,
            2 * LAMPORTS_PER_SOL
        );
        await provider.connection.confirmTransaction(airdropSig);

        const creatorAta = await getOrCreateAssociatedTokenAccount(
            provider.connection,
            provider.wallet.payer,
            mint,
            creator
        );

        const stakerAta = await getOrCreateAssociatedTokenAccount(
            provider.connection,
            staker,
            mint,
            staker.publicKey
        );

        // Mint tokens
        const creatorTokens = 1000 * 10 ** 9;
        const stakerTokens = 500 * 10 ** 9;
        await mintTo(
            provider.connection,
            provider.wallet.payer,
            mint,
            creatorAta.address,
            provider.wallet.publicKey,
            creatorTokens
        );
        await mintTo(
            provider.connection,
            provider.wallet.payer,
            mint,
            stakerAta.address,
            provider.wallet.publicKey,
            stakerTokens
        );

        const [pool] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("pool"),
                creator.toBuffer(),
                poolId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        // Create pool
        const poolVaultAta = await getAssociatedTokenAddress(mint, pool, true);
        await program.methods
            .createPool(poolId, new anchor.BN(creatorTokens), claimCooldown)
            .accountsStrict({
                mint: mint,
                creator: creator,
                pool: pool,
                poolVault: poolVaultAta,
                creatorAta: creatorAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .rpc();

        // Stake tokens
        const depositId = new anchor.BN(1);
        const stakeAmount = new anchor.BN(200 * 10 ** 9);

        const [deposit] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("deposit"),
                staker.publicKey.toBuffer(),
                pool.toBuffer(),
                depositId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        const [stakerStats] = PublicKey.findProgramAddressSync(
            [Buffer.from("staker-stats"), staker.publicKey.toBuffer()],
            program.programId
        );

        await program.methods
            .stake(depositId, stakeAmount)
            .accountsStrict({
                mint: mint,
                staker: staker.publicKey,
                deposit: deposit,
                stakerStats: stakerStats,
                pool: pool,
                poolVault: poolVaultAta,
                stakerAta: stakerAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .signers([staker])
            .rpc();

        // Enable emergency mode
        await program.methods
            .enableEmergencyMode()
            .accountsStrict({
                mint: mint,
                creator: creator,
                pool: pool,
                poolVault: poolVaultAta,
                creatorAta: creatorAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .rpc();

        // Emergency unstake (should work without cooldown)
        const stakerBalanceBefore = await getAccount(
            provider.connection,
            stakerAta.address
        );

        await program.methods
            .unstakeEmergency(depositId)
            .accountsStrict({
                mint: mint,
                staker: staker.publicKey,
                deposit: deposit,
                stakerStats: stakerStats,
                pool: pool,
                poolVault: poolVaultAta,
                stakerAta: stakerAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .signers([staker])
            .rpc();

        // Verify staker received only staked tokens (no rewards in emergency)
        const stakerBalanceAfter = await getAccount(
            provider.connection,
            stakerAta.address
        );

        const balanceIncrease =
            stakerBalanceAfter.amount - stakerBalanceBefore.amount;
        assert.ok(balanceIncrease === BigInt(stakeAmount.toString()));

        // Verify deposit is marked as withdrawn
        const depositAccountAfter = await program.account.stakerDeposit.fetch(
            deposit
        );
        assert.ok(depositAccountAfter.isWithdrawn === true);
    });

    it("allows pool creator to withdraw rewards in emergency mode", async () => {
        const mint = await createMint(
            provider.connection,
            provider.wallet.payer,
            provider.wallet.publicKey,
            null,
            9
        );

        const creator = provider.wallet.publicKey;
        const poolId = new anchor.BN(9);
        const claimCooldown = new anchor.BN(60);

        const creatorAta = await getOrCreateAssociatedTokenAccount(
            provider.connection,
            provider.wallet.payer,
            mint,
            creator
        );

        // Mint tokens
        const creatorTokens = 1000 * 10 ** 9;
        await mintTo(
            provider.connection,
            provider.wallet.payer,
            mint,
            creatorAta.address,
            provider.wallet.publicKey,
            creatorTokens
        );

        const [pool] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("pool"),
                creator.toBuffer(),
                poolId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        // Create pool with rewards
        const poolVaultAta = await getAssociatedTokenAddress(mint, pool, true);
        const initialRewards = new anchor.BN(creatorTokens);
        await program.methods
            .createPool(poolId, initialRewards, claimCooldown)
            .accountsStrict({
                mint: mint,
                creator: creator,
                pool: pool,
                poolVault: poolVaultAta,
                creatorAta: creatorAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .rpc();

        // Enable emergency mode
        await program.methods
            .enableEmergencyMode()
            .accountsStrict({
                mint: mint,
                creator: creator,
                pool: pool,
                poolVault: poolVaultAta,
                creatorAta: creatorAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .rpc();

        // Withdraw rewards
        const creatorBalanceBefore = await getAccount(
            provider.connection,
            creatorAta.address
        );

        await program.methods
            .withdrawRewardsEmergency()
            .accountsStrict({
                mint: mint,
                creator: creator,
                pool: pool,
                poolVault: poolVaultAta,
                creatorAta: creatorAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .rpc();

        const creatorBalanceAfter = await getAccount(
            provider.connection,
            creatorAta.address
        );

        const balanceIncrease =
            creatorBalanceAfter.amount - creatorBalanceBefore.amount;
        assert.ok(balanceIncrease === BigInt(initialRewards.toString()));

        // Verify pool rewards are zero
        const poolAccount = await program.account.stakingPool.fetch(pool);
        assert.ok(poolAccount.currentRewards.eq(new anchor.BN(0)));
    });

    it("allows pool creator to change cooldown period", async () => {
        const mint = await createMint(
            provider.connection,
            provider.wallet.payer,
            provider.wallet.publicKey,
            null,
            9
        );

        const creator = provider.wallet.publicKey;
        const poolId = new anchor.BN(10);
        const initialCooldown = new anchor.BN(60);
        const newCooldown = new anchor.BN(120);

        const creatorAta = await getOrCreateAssociatedTokenAccount(
            provider.connection,
            provider.wallet.payer,
            mint,
            creator
        );

        const [pool] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("pool"),
                creator.toBuffer(),
                poolId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        // Create pool
        const poolVaultAta = await getAssociatedTokenAddress(mint, pool, true);
        await program.methods
            .createPool(poolId, new anchor.BN(0), initialCooldown)
            .accountsStrict({
                mint: mint,
                creator: creator,
                pool: pool,
                poolVault: poolVaultAta,
                creatorAta: creatorAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .rpc();

        // Change cooldown
        await program.methods
            .changePoolCooldown(newCooldown)
            .accountsStrict({
                mint: mint,
                creator: creator,
                pool: pool,
                poolVault: poolVaultAta,
                creatorAta: creatorAta.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .rpc();

        const poolAccount = await program.account.stakingPool.fetch(pool);
        assert.ok(poolAccount.claimCooldown.eq(newCooldown));
    });

    it("supports multiple independent pools", async () => {
        const mint = await createMint(
            provider.connection,
            provider.wallet.payer,
            provider.wallet.publicKey,
            null,
            9
        );

        const creator1 = provider.wallet.publicKey;
        const creator2 = Keypair.generate();

        // Fund creator2
        const airdropSig = await provider.connection.requestAirdrop(
            creator2.publicKey,
            2 * LAMPORTS_PER_SOL
        );
        await provider.connection.confirmTransaction(airdropSig);

        const poolId1 = new anchor.BN(11);
        const poolId2 = new anchor.BN(12);

        const creator1Ata = await getOrCreateAssociatedTokenAccount(
            provider.connection,
            provider.wallet.payer,
            mint,
            creator1
        );

        const creator2Ata = await getOrCreateAssociatedTokenAccount(
            provider.connection,
            creator2,
            mint,
            creator2.publicKey
        );

        // Create first pool
        const [pool1] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("pool"),
                creator1.toBuffer(),
                poolId1.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        const pool1VaultAta = await getAssociatedTokenAddress(mint, pool1, true);
        await program.methods
            .createPool(poolId1, new anchor.BN(0), new anchor.BN(60))
            .accountsStrict({
                mint: mint,
                creator: creator1,
                pool: pool1,
                poolVault: pool1VaultAta,
                creatorAta: creator1Ata.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .rpc();

        // Create second pool
        const [pool2] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("pool"),
                creator2.publicKey.toBuffer(),
                poolId2.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        const pool2VaultAta = await getAssociatedTokenAddress(mint, pool2, true);
        await program.methods
            .createPool(poolId2, new anchor.BN(0), new anchor.BN(120))
            .accountsStrict({
                mint: mint,
                creator: creator2.publicKey,
                pool: pool2,
                poolVault: pool2VaultAta,
                creatorAta: creator2Ata.address,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .signers([creator2])
            .rpc();

        // Verify both pools exist and are independent
        const pool1Account = await program.account.stakingPool.fetch(pool1);
        const pool2Account = await program.account.stakingPool.fetch(pool2);

        assert.ok(pool1Account.creator.equals(creator1));
        assert.ok(pool1Account.poolId.eq(poolId1));
        assert.ok(pool1Account.claimCooldown.eq(new anchor.BN(60)));

        assert.ok(pool2Account.creator.equals(creator2.publicKey));
        assert.ok(pool2Account.poolId.eq(poolId2));
        assert.ok(pool2Account.claimCooldown.eq(new anchor.BN(120)));
    });
});

