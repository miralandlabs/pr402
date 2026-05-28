import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import { facilitatorOrigin } from '../config';

const RESOURCES: { uri: string; path: string; name: string; mimeType: string }[] =
  [
    {
      uri: 'pr402://capabilities',
      path: '/api/v1/facilitator/capabilities',
      name: 'pr402 capabilities',
      mimeType: 'application/json',
    },
    {
      uri: 'pr402://openapi',
      path: '/openapi.json',
      name: 'pr402 OpenAPI',
      mimeType: 'application/json',
    },
    {
      uri: 'pr402://agent-integration',
      path: '/agent-integration.md',
      name: 'pr402 agent integration guide',
      mimeType: 'text/markdown',
    },
    {
      uri: 'pr402://payto-semantics',
      path: '/agent-payTo-semantics.json',
      name: 'pr402 payTo semantics',
      mimeType: 'application/json',
    },
  ];

export function registerResources(server: McpServer): void {
  for (const r of RESOURCES) {
    server.registerResource(
      r.name,
      r.uri,
      { description: `Static ${r.path} from facilitator host`, mimeType: r.mimeType },
      async () => {
        const res = await fetch(`${facilitatorOrigin()}${r.path}`);
        const text = await res.text();
        return {
          contents: [
            {
              uri: r.uri,
              mimeType: r.mimeType,
              text,
            },
          ],
        };
      }
    );
  }
}
