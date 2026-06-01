"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.registerBuyerTools = registerBuyerTools;
const web3_js_1 = require("@solana/web3.js");
const client_1 = require("@pr402/client");
const node_fs_1 = require("node:fs");
const zod_1 = require("zod");
const config_1 = require("../config");
const register_tool_1 = require("../register-tool");
const schemas_1 = require("../schemas");
function registerBuyerTools(server) {
    (0, register_tool_1.registerToolLoose)(server, 'pr402_get_capabilities', {
        description: 'Fetch GET /capabilities from the configured pr402 facilitator.',
        inputSchema: {},
    }, async () => {
        const res = await fetch(`${(0, config_1.facilitatorBase)()}/capabilities`);
        return { content: [{ type: 'text', text: await res.text() }] };
    });
    (0, register_tool_1.registerToolLoose)(server, 'pr402_build_exact_payment', {
        description: 'POST /build-exact-payment-tx — unsigned tx + verifyBodyTemplate.',
        inputSchema: {
            payer: zod_1.z.string().describe('Buyer base58 pubkey'),
            accepted: schemas_1.jsonObject.describe('One accepts[] line from HTTP 402'),
            resource: schemas_1.jsonObject
                .optional()
                .describe('Resource object from HTTP 402'),
            autoWrapSol: zod_1.z
                .boolean()
                .optional()
                .describe('Inject WSOL wrap instructions when true'),
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
    (0, register_tool_1.registerToolLoose)(server, 'pr402_pay_http_resource', {
        description: 'Fetch a 402-gated URL via @pr402/client fetchWithAutoPay. Set PR402_PAYER_KEYPAIR_JSON.',
        inputSchema: {
            url: zod_1.z.string().describe('Paid resource URL'),
            preferredMint: zod_1.z
                .string()
                .describe('Base58 mint to pay with (must match accepts[].asset)'),
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
        try {
            const parsed = JSON.parse((0, node_fs_1.readFileSync)(kpPath, 'utf8'));
            if (!Array.isArray(parsed) || parsed.length !== 64) {
                return {
                    content: [
                        {
                            type: 'text',
                            text: 'Keypair file must be a JSON array of 64 bytes.',
                        },
                    ],
                    isError: true,
                };
            }
            const wallet = web3_js_1.Keypair.fromSecretKey(Uint8Array.from(parsed));
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
        }
        catch (err) {
            const message = err instanceof Error ? err.message : String(err);
            return {
                content: [{ type: 'text', text: message }],
                isError: true,
            };
        }
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
