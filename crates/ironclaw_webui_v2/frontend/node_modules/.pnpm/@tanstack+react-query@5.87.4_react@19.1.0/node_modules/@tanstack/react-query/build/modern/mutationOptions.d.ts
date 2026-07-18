import { DefaultError, WithRequired } from '@tanstack/query-core';
import { UseMutationOptions } from './types.js';

declare function mutationOptions<TData = unknown, TError = DefaultError, TVariables = void, TContext = unknown>(options: WithRequired<UseMutationOptions<TData, TError, TVariables, TContext>, 'mutationKey'>): WithRequired<UseMutationOptions<TData, TError, TVariables, TContext>, 'mutationKey'>;
declare function mutationOptions<TData = unknown, TError = DefaultError, TVariables = void, TContext = unknown>(options: Omit<UseMutationOptions<TData, TError, TVariables, TContext>, 'mutationKey'>): Omit<UseMutationOptions<TData, TError, TVariables, TContext>, 'mutationKey'>;

export { mutationOptions };
