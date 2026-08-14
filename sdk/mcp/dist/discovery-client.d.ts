export type SearchResourcesOptions = {
    q?: string;
    category?: string;
    scheme?: 'exact' | 'sla-escrow';
    tag?: string;
    limit?: number;
};
export declare function searchResources(facilitatorUrl: string, options?: SearchResourcesOptions): Promise<unknown>;
export declare function probeResource(resourceUrl: string, httpMethod?: string): Promise<unknown>;
