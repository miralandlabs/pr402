"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.registerSellerTools = registerSellerTools;
const zod_1 = require("zod");
const config_1 = require("../config");
const register_tool_1 = require("../register-tool");
const schemas_1 = require("../schemas");
function registerSellerTools(server) {
    (0, register_tool_1.registerToolLoose)(server, 'pr402_seller_preview', {
        description: 'GET /sellers/{wallet}/preview — multi-rail lifecycle preview.',
        inputSchema: {
            wallet: zod_1.z.string().describe('Seller base58 pubkey'),
        },
    }, async (args) => {
        const wallet = String(args.wallet);
        const res = await fetch(`${(0, config_1.facilitatorBase)()}/sellers/${encodeURIComponent(wallet)}/preview`);
        return { content: [{ type: 'text', text: await res.text() }] };
    });
    (0, register_tool_1.registerToolLoose)(server, 'pr402_seller_rail_info', {
        description: 'GET /sellers/{wallet}/rails/{scheme} — single-rail payTo lookup.',
        inputSchema: {
            wallet: zod_1.z.string().describe('Seller base58 pubkey'),
            scheme: zod_1.z
                .string()
                .describe('Rail scheme: exact or sla-escrow'),
            asset: zod_1.z
                .string()
                .optional()
                .describe('SPL mint (required query for sla-escrow)'),
        },
    }, async (args) => {
        const wallet = String(args.wallet);
        const scheme = String(args.scheme);
        const asset = args.asset ? String(args.asset) : '';
        const q = asset ? `?asset=${encodeURIComponent(asset)}` : '';
        const res = await fetch(`${(0, config_1.facilitatorBase)()}/sellers/${encodeURIComponent(wallet)}/rails/${encodeURIComponent(scheme)}${q}`);
        return { content: [{ type: 'text', text: await res.text() }] };
    });
    (0, register_tool_1.registerToolLoose)(server, 'pr402_seller_provision_tx', {
        description: 'POST /sellers/provision-tx — unsigned CreateVault / ATA tx.',
        inputSchema: {
            wallet: zod_1.z.string().describe('Seller base58 pubkey'),
            asset: zod_1.z
                .string()
                .describe('SOL, USDC, USDT, or base58 SPL mint'),
        },
    }, async (args) => {
        const res = await fetch(`${(0, config_1.facilitatorBase)()}/sellers/provision-tx`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                wallet: args.wallet,
                asset: args.asset,
            }),
        });
        return { content: [{ type: 'text', text: await res.text() }] };
    });
    (0, register_tool_1.registerToolLoose)(server, 'pr402_enrich_payment_required', {
        description: 'POST /payment-required/enrich — enrich PaymentRequired for HTTP 402.',
        inputSchema: {
            paymentRequired: schemas_1.jsonObject.describe('Naive PaymentRequired JSON body'),
        },
    }, async (args) => {
        const res = await fetch(`${(0, config_1.facilitatorBase)()}/payment-required/enrich`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(args.paymentRequired),
        });
        return { content: [{ type: 'text', text: await res.text() }] };
    });
}
