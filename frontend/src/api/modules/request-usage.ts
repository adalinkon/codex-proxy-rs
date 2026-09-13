import type { RequestOptions } from '../request'
import request from '../request'

export type RequestUsageScope = 'users' | 'keys' | 'my-keys' | 'me'
export interface RequestUsage {
  id: string
  currentConcurrency: number | null
  currentRpm: number | null
}

const paths: Record<RequestUsageScope, string> = {
  'users': '/api/admin/users/request-usage',
  'keys': '/api/admin/client-keys/request-usage',
  'my-keys': '/api/user/client-keys/request-usage',
  'me': '/api/user/request-usage',
}

export function getRequestUsage(scope: RequestUsageScope, ids: string[], options: RequestOptions = {}) {
  return request<RequestUsage[]>({
    url: paths[scope],
    method: scope === 'me' ? 'GET' : 'POST',
    ...(scope === 'me' ? {} : { data: { ids } }),
    ...options,
  })
}
