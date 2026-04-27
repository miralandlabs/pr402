import { 
    Connection, 
    Keypair, 
    VersionedTransaction 
} from '@solana/web3.js';
import fetch from 'node-fetch';
import bs58 from 'bs58';

/**
 * PR402 AGENTIC SETTLEMENT BLUEPRINT
 * 
 * This script demonstrates the 'Golden Path' for an autonomous agent:
 * 1. Discover capabilities
 * 2. Receive HTTP 402 challenge (mocked)
 * 3. Build unsigned transaction via pr402 facilitator
 * 4. Sign transaction locally
 * 5. Verify & Settle
 */

// CONFIGURATION
const FACILITATOR_URL = 'https://agent.pay402.me';
const RPC_URL = 'https://api.mainnet-beta.solana.com';
const PAYER_SK = process.env.PAYER_PRIVATE_KEY; // Base58

async function runAgentFlow() {
    if (!PAYER_SK) {
        console.error('❌ PAYER_PRIVATE_KEY environment variable is required');
        process.exit(1);
    }

    const payer = Keypair.fromSecretKey(bs58.decode(PAYER_SK));
    const connection = new Connection(RPC_URL);

    console.log(`🤖 Agent Payer: ${payer.publicKey.toBase58()}`);

    // --- STEP 1: DISCOVER ---
    console.log('🔍 Discovering facilitator capabilities...');
    const capabilities = await fetch(`${FACILITATOR_URL}/api/v1/facilitator/capabilities`).then(r => r.json());
    console.log(`✅ Chain ID: ${capabilities.chainId}`);

    // --- STEP 2: RECEIVE CHALLENGE (MOCK) ---
    // In a real scenario, this comes from an HTTP 402 response header or body
    const mockPaymentRequired = {
        paymentRequirements: {
            accepted: [
                {
                    scheme: 'exact',
                    network: 'solana',
                    asset: 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v', // USDC
                    amount: '1000000', // 1.00 USDC
                    payTo: '7x...your_vault_pda...', // Resolved via discovery
                    extra: {
                        merchantWallet: 'SelleR...pubkey...'
                    }
                }
            ]
        }
    };

    const choice = mockPaymentRequired.paymentRequirements.accepted[0];
    console.log(`💳 Chosen rail: ${choice.scheme} (${choice.amount} units)`);

    // --- STEP 3: BUILD ---
    console.log('🛠 Building unsigned transaction...');
    const buildResponse = await fetch(`${FACILITATOR_URL}/api/v1/facilitator/build-exact-payment-tx`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
            payer: payer.publicKey.toBase58(),
            accepted: choice,
            resource: 'api.example.com/v1/inference'
        })
    }).then(r => r.json());

    if (buildResponse.error) {
        throw new Error(`Build failed: ${buildResponse.error}`);
    }

    // --- STEP 4: SIGN ---
    console.log('✍ Signing transaction...');
    const txBuffer = Buffer.from(buildResponse.transaction, 'base64');
    const transaction = VersionedTransaction.deserialize(txBuffer);
    
    // Sign the transaction
    transaction.sign([payer]);

    // Prepare the final payload using the template provided by the facilitator
    const finalPayload = buildResponse.verifyBodyTemplate;
    finalPayload.paymentPayload.payload.transaction = Buffer.from(transaction.serialize()).toString('base64');

    // --- STEP 5: VERIFY & SETTLE ---
    console.log('🛡 Verifying proof...');
    const verifyResult = await fetch(`${FACILITATOR_URL}/api/v1/facilitator/verify`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(finalPayload)
    }).then(r => r.json());

    if (!verifyResult.valid) {
        throw new Error(`Verification failed: ${verifyResult.error}`);
    }
    console.log('✅ Proof is valid.');

    console.log('🚀 Settling on-chain...');
    const settleResult = await fetch(`${FACILITATOR_URL}/api/v1/facilitator/settle`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(finalPayload)
    }).then(r => r.json());

    console.log(`✨ Settlement Complete! Tx: ${settleResult.transaction}`);
}

runAgentFlow().catch(err => {
    console.error(`💥 Agent Crash: ${err.message}`);
});
