import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import { searchResources, probeResource } from '@pr402/discovery';
import { z } from 'zod';
import { facilitatorBase } from '../config';
import { registerToolLoose } from '../register-tool';

export function registerDiscoveryTools(server: McpServer): void {
  registerToolLoose(
    server,
    'pr402_search_resources',
    {
      description:
        'Search GET /api/v1/facilitator/resources — payable API endpoints (not merchant origins).',
      inputSchema: {
        q: z.string().optional().describe('Search query'),
        category: z.string().optional(),
        scheme: z.enum(['exact', 'sla-escrow']).optional(),
        tag: z.string().optional(),
        limit: z.number().optional(),
      },
    },
    async (args) => {
      const data = await searchResources(facilitatorBase(), {
        q: args.q as string | undefined,
        category: args.category as string | undefined,
        scheme: args.scheme as 'exact' | 'sla-escrow' | undefined,
        tag: args.tag as string | undefined,
        limit: args.limit as number | undefined,
      });
      return {
        content: [{ type: 'text' as const, text: JSON.stringify(data, null, 2) }],
      };
    }
  );

  registerToolLoose(
    server,
    'pr402_probe_resource',
    {
      description:
        'Unpaid GET (or POST) to a resourceUrl — expect HTTP 402 with valid PaymentRequired JSON.',
      inputSchema: {
        resourceUrl: z.string(),
        httpMethod: z.string().optional().describe('Default GET'),
      },
    },
    async (args) => {
      const result = await probeResource(
        String(args.resourceUrl),
        args.httpMethod ? String(args.httpMethod) : 'GET'
      );
      return {
        content: [{ type: 'text' as const, text: JSON.stringify(result, null, 2) }],
      };
    }
  );
}
