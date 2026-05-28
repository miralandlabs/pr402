"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.registerToolLoose = registerToolLoose;
/**
 * registerTool wrapper — avoids TS2589 from deep MCP SDK + Zod inference.
 * Runtime validation still uses Zod raw shapes.
 */
function registerToolLoose(server, name, config, handler) {
    // Cast server to any — fully bypasses TS2589 from deep MCP SDK + Zod inference.
    server.registerTool(name, config, handler);
}
