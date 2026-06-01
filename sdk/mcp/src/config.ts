const DEFAULT_FACILITATOR = 'https://ipay.sh';

export function facilitatorBase(): string {
  const raw = (process.env.PR402_FACILITATOR_URL || DEFAULT_FACILITATOR).replace(
    /\/$/,
    ''
  );
  return raw.endsWith('/api/v1/facilitator')
    ? raw
    : `${raw}/api/v1/facilitator`;
}

export function facilitatorOrigin(): string {
  const base = facilitatorBase();
  return base.replace(/\/api\/v1\/facilitator$/, '');
}
