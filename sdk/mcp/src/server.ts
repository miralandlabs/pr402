#!/usr/bin/env node
import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { registerBuyerTools } from './tools/buyer';
import { registerSellerTools } from './tools/seller';
import { registerResources } from './resources/index';

async function main(): Promise<void> {
  const server = new McpServer({
    name: 'pr402-mcp-server',
    version: '0.1.0',
  });

  registerBuyerTools(server);
  registerSellerTools(server);
  registerResources(server);

  const transport = new StdioServerTransport();
  await server.connect(transport);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
