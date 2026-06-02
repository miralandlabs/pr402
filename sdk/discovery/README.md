# @pr402/discovery

Search pr402 payable resources and probe live HTTP 402 before paying.

```typescript
import { searchResources, probeResource } from '@pr402/discovery';

const hits = await searchResources('https://preview.ipay.sh', { q: 'wallet risk' });
const probe = await probeResource(hits.entries[0].resourceUrl);
```

See [DISCOVERY.md](../../docs/DISCOVERY.md).
