import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import { z } from 'zod';
import { facilitatorBase } from '../config';
import { registerToolLoose } from '../register-tool';
import { jsonObject } from '../schemas';

export function registerSellerTools(server: McpServer): void {
  registerToolLoose(
    server,
    'pr402_seller_preview',
    {
      description: 'GET /sellers/{wallet}/preview — multi-rail lifecycle preview.',
      inputSchema: {
        wallet: z.string().describe('Seller base58 pubkey'),
      },
    },
    async (args) => {
      const wallet = String(args.wallet);
      const res = await fetch(
        `${facilitatorBase()}/sellers/${encodeURIComponent(wallet)}/preview`
      );
      return { content: [{ type: 'text' as const, text: await res.text() }] };
    }
  );

  registerToolLoose(
    server,
    'pr402_seller_rail_info',
    {
      description: 'GET /sellers/{wallet}/rails/{scheme} — single-rail payTo lookup.',
      inputSchema: {
        wallet: z.string().describe('Seller base58 pubkey'),
        scheme: z
          .string()
          .describe('Rail scheme: exact or sla-escrow'),
        asset: z
          .string()
          .optional()
          .describe('SPL mint (required query for sla-escrow)'),
      },
    },
    async (args) => {
      const wallet = String(args.wallet);
      const scheme = String(args.scheme);
      const asset = args.asset ? String(args.asset) : '';
      const q = asset ? `?asset=${encodeURIComponent(asset)}` : '';
      const res = await fetch(
        `${facilitatorBase()}/sellers/${encodeURIComponent(wallet)}/rails/${encodeURIComponent(scheme)}${q}`
      );
      return { content: [{ type: 'text' as const, text: await res.text() }] };
    }
  );

  registerToolLoose(
    server,
    'pr402_seller_provision_tx',
    {
      description: 'POST /sellers/provision-tx — unsigned CreateVault / ATA tx.',
      inputSchema: {
        wallet: z.string().describe('Seller base58 pubkey'),
        asset: z
          .string()
          .describe('SOL, USDC, USDT, or base58 SPL mint'),
      },
    },
    async (args) => {
      const res = await fetch(`${facilitatorBase()}/sellers/provision-tx`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          wallet: args.wallet,
          asset: args.asset,
        }),
      });
      return { content: [{ type: 'text' as const, text: await res.text() }] };
    }
  );

  registerToolLoose(
    server,
    'pr402_enrich_payment_required',
    {
      description: 'POST /payment-required/enrich — enrich PaymentRequired for HTTP 402.',
      inputSchema: {
        paymentRequired: jsonObject.describe('Naive PaymentRequired JSON body'),
      },
    },
    async (args) => {
      const res = await fetch(`${facilitatorBase()}/payment-required/enrich`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(args.paymentRequired),
      });
      return { content: [{ type: 'text' as const, text: await res.text() }] };
    }
  );
}
