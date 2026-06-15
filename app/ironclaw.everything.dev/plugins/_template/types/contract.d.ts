import { z } from "every-plugin/zod";
export declare const ItemSchema: z.ZodObject<{
    id: z.ZodString;
    title: z.ZodString;
    createdAt: z.ZodString;
}, z.core.$strip>;
export declare const SearchResultSchema: z.ZodObject<{
    item: z.ZodObject<{
        id: z.ZodString;
        title: z.ZodString;
        createdAt: z.ZodString;
    }, z.core.$strip>;
    score: z.ZodNumber;
}, z.core.$strip>;
export declare const BackgroundEventSchema: z.ZodObject<{
    id: z.ZodString;
    index: z.ZodNumber;
    timestamp: z.ZodNumber;
}, z.core.$strip>;
export declare const contract: {
    getById: import("@orpc/contract").ContractProcedure<z.ZodObject<{
        id: z.ZodString;
    }, z.core.$strip>, z.ZodObject<{
        item: z.ZodObject<{
            id: z.ZodString;
            title: z.ZodString;
            createdAt: z.ZodString;
        }, z.core.$strip>;
        userId: z.ZodString;
    }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
        UNAUTHORIZED: {
            status: number;
            message: string;
        };
        NOT_FOUND: {
            status: number;
            message: string;
        };
    }>>, Record<never, never>>;
    search: import("@orpc/contract").ContractProcedure<z.ZodObject<{
        query: z.ZodString;
        limit: z.ZodDefault<z.ZodNumber>;
    }, z.core.$strip>, import("@orpc/contract").Schema<AsyncIteratorObject<{
        item: {
            id: string;
            title: string;
            createdAt: string;
        };
        score: number;
    }, unknown, void>, import("@orpc/shared").AsyncIteratorClass<{
        item: {
            id: string;
            title: string;
            createdAt: string;
        };
        score: number;
    }, unknown, void>>, import("@orpc/contract").MergedErrorMap<Record<never, never>, Record<never, never>>, Record<never, never>>;
    ping: import("@orpc/contract").ContractProcedure<import("@orpc/contract").Schema<unknown, unknown>, z.ZodObject<{
        status: z.ZodLiteral<"ok">;
        timestamp: z.ZodString;
    }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, Record<never, never>>, Record<never, never>>;
    listenBackground: import("@orpc/contract").ContractProcedure<z.ZodObject<{
        maxResults: z.ZodOptional<z.ZodNumber>;
        lastEventId: z.ZodOptional<z.ZodString>;
    }, z.core.$strip>, import("@orpc/contract").Schema<AsyncIteratorObject<{
        id: string;
        index: number;
        timestamp: number;
    }, unknown, void>, import("@orpc/shared").AsyncIteratorClass<{
        id: string;
        index: number;
        timestamp: number;
    }, unknown, void>>, import("@orpc/contract").MergedErrorMap<Record<never, never>, Record<never, never>>, Record<never, never>>;
    enqueueBackground: import("@orpc/contract").ContractProcedure<z.ZodObject<{
        id: z.ZodOptional<z.ZodString>;
    }, z.core.$strip>, z.ZodObject<{
        ok: z.ZodBoolean;
    }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, Record<never, never>>, Record<never, never>>;
};
export type ContractType = typeof contract;
