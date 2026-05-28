#!/usr/bin/env node
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
const mcp_js_1 = require("@modelcontextprotocol/sdk/server/mcp.js");
const stdio_js_1 = require("@modelcontextprotocol/sdk/server/stdio.js");
const buyer_1 = require("./tools/buyer");
const seller_1 = require("./tools/seller");
const index_1 = require("./resources/index");
async function main() {
    const server = new mcp_js_1.McpServer({
        name: 'pr402-mcp-server',
        version: '0.1.0',
    });
    (0, buyer_1.registerBuyerTools)(server);
    (0, seller_1.registerSellerTools)(server);
    (0, index_1.registerResources)(server);
    const transport = new stdio_js_1.StdioServerTransport();
    await server.connect(transport);
}
main().catch((err) => {
    console.error(err);
    process.exit(1);
});
