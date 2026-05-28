import { z } from 'zod';

/** Opaque JSON object (402 accepts line, resource, PaymentRequired). */
export const jsonObject = z.any();
