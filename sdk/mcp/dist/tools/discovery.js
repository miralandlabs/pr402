"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.registerDiscoveryTools = registerDiscoveryTools;
const discovery_1 = require("@pr402/discovery");
const zod_1 = require("zod");
const config_1 = require("../config");
const register_tool_1 = require("../register-tool");
function registerDiscoveryTools(server) {
    (0, register_tool_1.registerToolLoose)(server, 'pr402_search_resources', {
        description: 'Search GET /api/v1/facilitator/resources — payable API endpoints (not merchant origins).',
        inputSchema: {
            q: zod_1.z.string().optional().describe('Search query'),
            category: zod_1.z.string().optional(),
            scheme: zod_1.z.enum(['exact', 'sla-escrow']).optional(),
            tag: zod_1.z.string().optional(),
            limit: zod_1.z.number().optional(),
        },
    }, async (args) => {
        const data = await (0, discovery_1.searchResources)((0, config_1.facilitatorBase)(), {
            q: args.q,
            category: args.category,
            scheme: args.scheme,
            tag: args.tag,
            limit: args.limit,
        });
        return {
            content: [{ type: 'text', text: JSON.stringify(data, null, 2) }],
        };
    });
    (0, register_tool_1.registerToolLoose)(server, 'pr402_probe_resource', {
        description: 'Unpaid GET (or POST) to a resourceUrl — expect HTTP 402 with valid PaymentRequired JSON.',
        inputSchema: {
            resourceUrl: zod_1.z.string(),
            httpMethod: zod_1.z.string().optional().describe('Default GET'),
        },
    }, async (args) => {
        const result = await (0, discovery_1.probeResource)(String(args.resourceUrl), args.httpMethod ? String(args.httpMethod) : 'GET');
        return {
            content: [{ type: 'text', text: JSON.stringify(result, null, 2) }],
        };
    });
}
