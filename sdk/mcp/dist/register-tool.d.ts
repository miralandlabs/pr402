import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import type { z } from 'zod';
type ToolResult = {
    content: Array<{
        type: 'text';
        text: string;
    }>;
    isError?: boolean;
};
/**
 * registerTool wrapper — avoids TS2589 from deep MCP SDK + Zod inference.
 * Runtime validation still uses Zod raw shapes.
 */
export declare function registerToolLoose(server: McpServer, name: string, config: {
    description: string;
    inputSchema: Record<string, z.ZodTypeAny> | Record<string, never>;
}, handler: (args: Record<string, unknown>) => Promise<ToolResult>): void;
export {};
