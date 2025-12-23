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
import { Multisig } from "../target/types/multisig";

const { SystemProgram, LAMPORTS_PER_SOL, PublicKey, Keypair } = anchor.web3;

describe("multisig", () => {
    // Configure the client to use the local cluster.
    anchor.setProvider(anchor.AnchorProvider.env());

    const provider = anchor.getProvider() as anchor.AnchorProvider;
    const program = anchor.workspace.multisig as Program<Multisig>;

    let signer1: Keypair;
    let signer2: Keypair;
    let signer3: Keypair;
    let recipient: Keypair;

    before(async () => {
        // Create signers
        signer1 = Keypair.generate();
        signer2 = Keypair.generate();
        signer3 = Keypair.generate();
        recipient = Keypair.generate();

        // Airdrop SOL to signers
        const airdropAmount = 2 * LAMPORTS_PER_SOL;
        await provider.connection.requestAirdrop(
            signer1.publicKey,
            airdropAmount
        );
        await provider.connection.requestAirdrop(
            signer2.publicKey,
            airdropAmount
        );
        await provider.connection.requestAirdrop(
            signer3.publicKey,
            airdropAmount
        );
        await provider.connection.requestAirdrop(
            recipient.publicKey,
            airdropAmount
        );

        // Wait for airdrops to confirm
        await new Promise((resolve) => setTimeout(resolve, 1000));
    });

    it("creates a multisig vault", async () => {
        const creator = provider.wallet.publicKey;
        const vaultId = new anchor.BN(1);
        const signers = [signer1.publicKey, signer2.publicKey, signer3.publicKey];
        const threshold = 2;

        const [vault] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("multisig_vault"),
                creator.toBuffer(),
                vaultId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        await program.methods
            .createVault(vaultId, signers, threshold)
            .accounts({
                creator: creator,
                vault: vault,
                systemProgram: SystemProgram.programId,
            })
            .rpc();

        const vaultAccount = await program.account.multisigVault.fetch(vault);

        assert.ok(vaultAccount.vaultId.eq(vaultId));
        assert.ok(vaultAccount.threshold === threshold);
        assert.ok(vaultAccount.signers.length === 3);
        assert.ok(vaultAccount.signers[0].equals(signer1.publicKey));
        assert.ok(vaultAccount.signers[1].equals(signer2.publicKey));
        assert.ok(vaultAccount.signers[2].equals(signer3.publicKey));
        assert.ok(vaultAccount.creator.equals(creator));
    });

    it("proposes a SOL transfer", async () => {
        const creator = provider.wallet.publicKey;
        const vaultId = new anchor.BN(1);
        const proposalId = new anchor.BN(1);
        const transferAmount = new anchor.BN(0.5 * LAMPORTS_PER_SOL);

        const [vault] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("multisig_vault"),
                creator.toBuffer(),
                vaultId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        const [proposal] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("transfer_proposal"),
                vault.toBuffer(),
                proposalId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        // Fund the vault with SOL (use separate SOL PDA)
        const [vaultSolPda] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("vault_sol"),
                creator.toBuffer(),
                vaultId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        const fundTx = new anchor.web3.Transaction().add(
            SystemProgram.transfer({
                fromPubkey: provider.wallet.publicKey,
                toPubkey: vaultSolPda,
                lamports: LAMPORTS_PER_SOL,
            })
        );
        await provider.sendAndConfirm(fundTx);

        // Propose transfer
        await program.methods
            .proposeTransfer(proposalId, recipient.publicKey, transferAmount, null)
            .accounts({
                vault: vault,
                proposal: proposal,
                proposer: signer1.publicKey,
                systemProgram: SystemProgram.programId,
            })
            .signers([signer1])
            .rpc();

        const proposalAccount = await program.account.transferProposal.fetch(
            proposal
        );

        assert.ok(proposalAccount.vault.equals(vault));
        assert.ok(proposalAccount.proposer.equals(signer1.publicKey));
        assert.ok(proposalAccount.recipient.equals(recipient.publicKey));
        assert.ok(proposalAccount.amount.eq(transferAmount));
        assert.ok(proposalAccount.tokenMint === null);
        assert.ok(proposalAccount.executed === false);
        assert.ok(proposalAccount.approvals[0] === true); // Proposer auto-approved
        assert.ok(proposalAccount.approvals[1] === false);
        assert.ok(proposalAccount.approvals[2] === false);
    });

    it("approves a transfer proposal", async () => {
        const creator = provider.wallet.publicKey;
        const vaultId = new anchor.BN(1);
        const proposalId = new anchor.BN(1);

        const [vault] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("multisig_vault"),
                creator.toBuffer(),
                vaultId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        const [proposal] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("transfer_proposal"),
                vault.toBuffer(),
                proposalId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        // Signer2 approves
        await program.methods
            .approveTransfer()
            .accounts({
                vault: vault,
                proposal: proposal,
                approver: signer2.publicKey,
            })
            .signers([signer2])
            .rpc();

        const proposalAccount = await program.account.transferProposal.fetch(
            proposal
        );

        assert.ok(proposalAccount.approvals[0] === true); // Signer1
        assert.ok(proposalAccount.approvals[1] === true); // Signer2
        assert.ok(proposalAccount.approvals[2] === false); // Signer3
    });

    it("executes a SOL transfer when threshold is met", async () => {
        const creator = provider.wallet.publicKey;
        const vaultId = new anchor.BN(1);
        const proposalId = new anchor.BN(1);

        const [vault] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("multisig_vault"),
                creator.toBuffer(),
                vaultId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        const [proposal] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("transfer_proposal"),
                vault.toBuffer(),
                proposalId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        const [vaultSolPda] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("vault_sol"),
                creator.toBuffer(),
                vaultId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        const recipientBalanceBefore = await provider.connection.getBalance(
            recipient.publicKey
        );

        // Execute transfer (threshold is 2, we have 2 approvals)
        await program.methods
            .executeSolTransfer()
            .accounts({
                vault: vault,
                proposal: proposal,
                vaultSolAccount: vaultSolPda,
                recipient: recipient.publicKey,
                systemProgram: SystemProgram.programId,
            })
            .rpc();

        const recipientBalanceAfter = await provider.connection.getBalance(
            recipient.publicKey
        );

        const proposalAccount = await program.account.transferProposal.fetch(
            proposal
        );

        assert.ok(proposalAccount.executed === true);
        assert.ok(
            recipientBalanceAfter ===
                recipientBalanceBefore + 0.5 * LAMPORTS_PER_SOL
        );
    });

    it("creates a vault and proposes SPL token transfer", async () => {
        const creator = provider.wallet.publicKey;
        const vaultId = new anchor.BN(2);
        const proposalId = new anchor.BN(1);
        const signers = [signer1.publicKey, signer2.publicKey];
        const threshold = 2;

        // Create token mint
        const mint = await createMint(
            provider.connection,
            provider.wallet.payer,
            provider.wallet.publicKey,
            null,
            9
        );

        const [vault] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("multisig_vault"),
                creator.toBuffer(),
                vaultId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        // Create vault
        await program.methods
            .createVault(vaultId, signers, threshold)
            .accounts({
                creator: creator,
                vault: vault,
                systemProgram: SystemProgram.programId,
            })
            .rpc();

        // Create vault token account
        const [vaultPda] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("multisig_vault"),
                creator.toBuffer(),
                vaultId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        const vaultTokenAccount = await getOrCreateAssociatedTokenAccount(
            provider.connection,
            provider.wallet.payer,
            mint,
            vaultPda,
            true
        );

        // Mint tokens to vault
        const mintAmount = 1000 * 10 ** 9; // 1000 tokens
        await mintTo(
            provider.connection,
            provider.wallet.payer,
            mint,
            vaultTokenAccount.address,
            provider.wallet.publicKey,
            mintAmount
        );

        // Create recipient token account
        const recipientTokenAccount = await getOrCreateAssociatedTokenAccount(
            provider.connection,
            provider.wallet.payer,
            mint,
            recipient.publicKey,
            true
        );

        const [proposal] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("transfer_proposal"),
                vault.toBuffer(),
                proposalId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        const transferAmount = new anchor.BN(500 * 10 ** 9); // 500 tokens

        // Propose transfer
        await program.methods
            .proposeTransfer(
                proposalId,
                recipient.publicKey,
                transferAmount,
                mint
            )
            .accounts({
                vault: vault,
                proposal: proposal,
                proposer: signer1.publicKey,
                systemProgram: SystemProgram.programId,
            })
            .signers([signer1])
            .rpc();

        const proposalAccount = await program.account.transferProposal.fetch(
            proposal
        );

        assert.ok(proposalAccount.tokenMint !== null);
        assert.ok(proposalAccount.tokenMint.equals(mint));
        assert.ok(proposalAccount.approvals[0] === true);
    });

    it("executes an SPL token transfer when threshold is met", async () => {
        const creator = provider.wallet.publicKey;
        const vaultId = new anchor.BN(2);
        const proposalId = new anchor.BN(1);

        const [vault] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("multisig_vault"),
                creator.toBuffer(),
                vaultId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        const [proposal] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("transfer_proposal"),
                vault.toBuffer(),
                proposalId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        const proposalAccountBefore = await program.account.transferProposal.fetch(
            proposal
        );
        const mint = proposalAccountBefore.tokenMint as PublicKey;

        const [vaultPda] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("multisig_vault"),
                creator.toBuffer(),
                vaultId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        const vaultTokenAccount = await getAssociatedTokenAddress(
            mint,
            vaultPda,
            true
        );
        const recipientTokenAccount = await getAssociatedTokenAddress(
            mint,
            recipient.publicKey,
            true
        );

        const recipientBalanceBefore = (
            await getAccount(provider.connection, recipientTokenAccount)
        ).amount;

        // Signer2 approves
        await program.methods
            .approveTransfer()
            .accounts({
                vault: vault,
                proposal: proposal,
                approver: signer2.publicKey,
            })
            .signers([signer2])
            .rpc();

        // Execute transfer
        await program.methods
            .executeSplTransfer()
            .accounts({
                vault: vault,
                vaultPda: vaultPda,
                proposal: proposal,
                mint: mint,
                vaultTokenAccount: vaultTokenAccount,
                recipientTokenAccount: recipientTokenAccount,
                tokenProgram: TOKEN_PROGRAM_ID,
                associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
            })
            .rpc();

        const recipientBalanceAfter = (
            await getAccount(provider.connection, recipientTokenAccount)
        ).amount;

        const proposalAccountAfter = await program.account.transferProposal.fetch(
            proposal
        );

        assert.ok(proposalAccountAfter.executed === true);
        assert.ok(
            recipientBalanceAfter ===
                recipientBalanceBefore + BigInt(500 * 10 ** 9)
        );
    });

    it("fails to execute transfer with insufficient approvals", async () => {
        const creator = provider.wallet.publicKey;
        const vaultId = new anchor.BN(3);
        const proposalId = new anchor.BN(1);
        const signers = [signer1.publicKey, signer2.publicKey, signer3.publicKey];
        const threshold = 3; // Requires all 3 signers

        const [vault] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("multisig_vault"),
                creator.toBuffer(),
                vaultId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        // Create vault
        await program.methods
            .createVault(vaultId, signers, threshold)
            .accounts({
                creator: creator,
                vault: vault,
                systemProgram: SystemProgram.programId,
            })
            .rpc();

        // Fund vault (use separate SOL PDA)
        const [vaultSolPda] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("vault_sol"),
                creator.toBuffer(),
                vaultId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        const fundTx = new anchor.web3.Transaction().add(
            SystemProgram.transfer({
                fromPubkey: provider.wallet.publicKey,
                toPubkey: vaultSolPda,
                lamports: LAMPORTS_PER_SOL,
            })
        );
        await provider.sendAndConfirm(fundTx);

        const [proposal] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("transfer_proposal"),
                vault.toBuffer(),
                proposalId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        const transferAmount = new anchor.BN(0.1 * LAMPORTS_PER_SOL);

        // Propose transfer
        await program.methods
            .proposeTransfer(proposalId, recipient.publicKey, transferAmount, null)
            .accounts({
                vault: vault,
                proposal: proposal,
                proposer: signer1.publicKey,
                systemProgram: SystemProgram.programId,
            })
            .signers([signer1])
            .rpc();

        // Only signer2 approves (we have 2 approvals, but need 3)
        await program.methods
            .approveTransfer()
            .accounts({
                vault: vault,
                proposal: proposal,
                approver: signer2.publicKey,
            })
            .signers([signer2])
            .rpc();

        // Try to execute - should fail
        try {
            await program.methods
                .executeSolTransfer()
                .accounts({
                    vault: vault,
                    proposal: proposal,
                    vaultSolAccount: vaultSolPda,
                    recipient: recipient.publicKey,
                    systemProgram: SystemProgram.programId,
                })
                .rpc();
            assert.fail("Should have failed with insufficient approvals");
        } catch (err) {
            assert.ok(err.toString().includes("InsufficientApprovals"));
        }
    });

    it("fails to create vault with duplicate signers", async () => {
        const creator = provider.wallet.publicKey;
        const vaultId = new anchor.BN(4);
        const signers = [
            signer1.publicKey,
            signer1.publicKey, // Duplicate
            signer2.publicKey,
        ];
        const threshold = 2;

        const [vault] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("multisig_vault"),
                creator.toBuffer(),
                vaultId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        try {
            await program.methods
                .createVault(vaultId, signers, threshold)
                .accounts({
                    creator: creator,
                    vault: vault,
                    systemProgram: SystemProgram.programId,
                })
                .rpc();
            assert.fail("Should have failed with duplicate signers");
        } catch (err) {
            assert.ok(err.toString().includes("DuplicateSigners"));
        }
    });

    it("fails to create vault with invalid threshold", async () => {
        const creator = provider.wallet.publicKey;
        const vaultId = new anchor.BN(5);
        const signers = [signer1.publicKey, signer2.publicKey];
        const threshold = 3; // Higher than number of signers

        const [vault] = PublicKey.findProgramAddressSync(
            [
                Buffer.from("multisig_vault"),
                creator.toBuffer(),
                vaultId.toArrayLike(Buffer, "le", 8),
            ],
            program.programId
        );

        try {
            await program.methods
                .createVault(vaultId, signers, threshold)
                .accounts({
                    creator: creator,
                    vault: vault,
                    systemProgram: SystemProgram.programId,
                })
                .rpc();
            assert.fail("Should have failed with invalid threshold");
        } catch (err) {
            assert.ok(
                err.toString().includes("ThresholdTooHigh") ||
                    err.toString().includes("InvalidThreshold")
            );
        }
    });
});

