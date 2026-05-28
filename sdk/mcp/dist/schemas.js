"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.jsonObject = void 0;
const zod_1 = require("zod");
/** Opaque JSON object (402 accepts line, resource, PaymentRequired). */
exports.jsonObject = zod_1.z.any();
