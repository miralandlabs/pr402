"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.facilitatorBase = facilitatorBase;
exports.facilitatorOrigin = facilitatorOrigin;
const DEFAULT_FACILITATOR = 'https://preview.ipay.sh';
function facilitatorBase() {
    const raw = (process.env.PR402_FACILITATOR_URL || DEFAULT_FACILITATOR).replace(/\/$/, '');
    return raw.endsWith('/api/v1/facilitator')
        ? raw
        : `${raw}/api/v1/facilitator`;
}
function facilitatorOrigin() {
    const base = facilitatorBase();
    return base.replace(/\/api\/v1\/facilitator$/, '');
}
