import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import { Keypair } from '@solana/web3.js';
import { X402AgentClient } from '@pr402/client';
import { readFileSync } from 'node:fs';
import { z } from 'zod';
import { facilitatorBase } from '../config';
import { registerToolLoose } from '../register-tool';
import { jsonObject } from '../schemas';

export function registerBuyerTools(server: McpServer): void {
  registerToolLoose(
    server,
    'pr402_get_capabilities',
    {
      description: 'Fetch GET /capabilities from the configured pr402 facilitator.',
      inputSchema: {},
    },
    async () => {
      const res = await fetch(`${facilitatorBase()}/capabilities`);
      return { content: [{ type: 'text' as const, text: await res.text() }] };
    }
  );

  registerToolLoose(
    server,
    'pr402_build_exact_payment',
    {
      description:
        'POST /build-exact-payment-tx — unsigned tx + verifyBodyTemplate.',
      inputSchema: {
        payer: z.string().describe('Buyer base58 pubkey'),
        accepted: jsonObject.describe('One accepts[] line from HTTP 402'),
        resource: jsonObject
          .optional()
          .describe('Resource object from HTTP 402'),
        autoWrapSol: z
          .boolean()
          .optional()
          .describe('Inject WSOL wrap instructions when true'),
      },
    },
    async (args) => {
      const res = await fetch(`${facilitatorBase()}/build-exact-payment-tx`, {
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
            type: 'text' as const,
            text: JSON.stringify(
              { status: res.status, body: safeJson(text) },
              null,
              2
            ),
          },
        ],
      };
    }
  );

  registerToolLoose(
    server,
    'pr402_pay_http_resource',
    {
      description:
        'Fetch a 402-gated URL via @pr402/client fetchWithAutoPay. Set PR402_PAYER_KEYPAIR_JSON.',
      inputSchema: {
        url: z.string().describe('Paid resource URL'),
        preferredMint: z
          .string()
          .describe('Base58 mint to pay with (must match accepts[].asset)'),
      },
    },
    async (args) => {
      const kpPath = process.env.PR402_PAYER_KEYPAIR_JSON;
      if (!kpPath) {
        return {
          content: [
            {
              type: 'text' as const,
              text: 'PR402_PAYER_KEYPAIR_JSON env var is required.',
            },
          ],
          isError: true,
        };
      }
      const secret = Uint8Array.from(
        JSON.parse(readFileSync(kpPath, 'utf8')) as number[]
      );
      const wallet = Keypair.fromSecretKey(secret);
      const client = new X402AgentClient(wallet);
      const res = await client.fetchWithAutoPay(
        String(args.url),
        String(args.preferredMint)
      );
      const body = await res.text();
      return {
        content: [
          {
            type: 'text' as const,
            text: JSON.stringify(
              { status: res.status, body: safeJson(body) },
              null,
              2
            ),
          },
        ],
      };
    }
  );
}

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}
