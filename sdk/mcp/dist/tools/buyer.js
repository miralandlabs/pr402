"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.registerBuyerTools = registerBuyerTools;
const web3_js_1 = require("@solana/web3.js");
const client_1 = require("@pr402/client");
const node_fs_1 = require("node:fs");
const config_1 = require("../config");
function registerBuyerTools(server) {
    const s = server;
    s.registerTool('pr402_get_capabilities', {
        description: 'Fetch GET /capabilities from the configured pr402 facilitator.',
        inputSchema: { type: 'object', properties: {} },
    }, async () => {
        const res = await fetch(`${(0, config_1.facilitatorBase)()}/capabilities`);
        return { content: [{ type: 'text', text: await res.text() }] };
    });
    s.registerTool('pr402_build_exact_payment', {
        description: 'POST /build-exact-payment-tx — unsigned tx + verifyBodyTemplate.',
        inputSchema: {
            type: 'object',
            properties: {
                payer: { type: 'string' },
                accepted: { type: 'object' },
                resource: { type: 'object' },
                autoWrapSol: { type: 'boolean' },
            },
            required: ['payer', 'accepted'],
        },
    }, async (args) => {
        const res = await fetch(`${(0, config_1.facilitatorBase)()}/build-exact-payment-tx`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                payer: args.payer,
                accepted: args.accepted,
                resource: args.resource,
                skipSourceBalanceCheck: true,
                autoWrapSol: args.autoWrapSol,
            }),
        });
        const text = await res.text();
        return {
            content: [
                {
                    type: 'text',
                    text: JSON.stringify({ status: res.status, body: safeJson(text) }, null, 2),
                },
            ],
        };
    });
    s.registerTool('pr402_pay_http_resource', {
        description: 'Fetch a 402-gated URL via @pr402/client fetchWithAutoPay. Set PR402_PAYER_KEYPAIR_JSON.',
        inputSchema: {
            type: 'object',
            properties: {
                url: { type: 'string' },
                preferredMint: { type: 'string' },
            },
            required: ['url', 'preferredMint'],
        },
    }, async (args) => {
        const kpPath = process.env.PR402_PAYER_KEYPAIR_JSON;
        if (!kpPath) {
            return {
                content: [
                    {
                        type: 'text',
                        text: 'PR402_PAYER_KEYPAIR_JSON env var is required.',
                    },
                ],
                isError: true,
            };
        }
        const secret = Uint8Array.from(JSON.parse((0, node_fs_1.readFileSync)(kpPath, 'utf8')));
        const wallet = web3_js_1.Keypair.fromSecretKey(secret);
        const client = new client_1.X402AgentClient(wallet);
        const res = await client.fetchWithAutoPay(String(args.url), String(args.preferredMint));
        const body = await res.text();
        return {
            content: [
                {
                    type: 'text',
                    text: JSON.stringify({ status: res.status, body: safeJson(body) }, null, 2),
                },
            ],
        };
    });
}
function safeJson(text) {
    try {
        return JSON.parse(text);
    }
    catch {
        return text;
    }
}
