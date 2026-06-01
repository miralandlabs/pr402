#!/usr/bin/env node
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { registerBuyerTools } from './tools/buyer';
import { registerSellerTools } from './tools/seller';
import { registerResources } from './resources/index';

const { version } = JSON.parse(
  readFileSync(join(__dirname, '..', 'package.json'), 'utf8')
) as { version: string };

async function main(): Promise<void> {
  const server = new McpServer({
    name: 'pr402-mcp-server',
    version,
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
