import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import { facilitatorBase } from '../config';

type ToolArgs = Record<string, unknown>;

export function registerSellerTools(server: McpServer): void {
  const s = server as McpServer & {
    registerTool: (
      name: string,
      config: { description?: string; inputSchema?: unknown },
      cb: (args: ToolArgs) => Promise<unknown>
    ) => void;
  };

  s.registerTool(
    'pr402_seller_preview',
    {
      description: 'GET /sellers/{wallet}/preview — multi-rail lifecycle preview.',
      inputSchema: {
        type: 'object',
        properties: { wallet: { type: 'string' } },
        required: ['wallet'],
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

  s.registerTool(
    'pr402_seller_rail_info',
    {
      description: 'GET /sellers/{wallet}/rails/{scheme} — single-rail payTo lookup.',
      inputSchema: {
        type: 'object',
        properties: {
          wallet: { type: 'string' },
          scheme: { type: 'string' },
          asset: { type: 'string' },
        },
        required: ['wallet', 'scheme'],
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

  s.registerTool(
    'pr402_seller_provision_tx',
    {
      description: 'POST /sellers/provision-tx — unsigned CreateVault / ATA tx.',
      inputSchema: {
        type: 'object',
        properties: {
          wallet: { type: 'string' },
          asset: { type: 'string' },
        },
        required: ['wallet', 'asset'],
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

  s.registerTool(
    'pr402_enrich_payment_required',
    {
      description: 'POST /payment-required/enrich — enrich PaymentRequired for HTTP 402.',
      inputSchema: {
        type: 'object',
        properties: {
          paymentRequired: { type: 'object' },
        },
        required: ['paymentRequired'],
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
