import type { RequestOptions } from '../request'
import type { AccountGroupRef } from './account-groups'
import request from '../request'

export type ApiKeyRoutingScope = 'groups' | 'inherit'

export interface ApiKey {
  id: string
  userId: string
  name: string
  label: string | null
  prefix: string
  enabled: boolean
  maxConcurrency: number
  requestsPerMinute: number
  dailyLimitUsd: string
  weeklyLimitUsd: string
  dailyUsedUsd: string
  weeklyUsedUsd: string
  dailyResetsAt: string | null
  weeklyResetsAt: string | null
  createdAt: string
  updatedAt: string
  lastUsedAt: string | null
  routingScope: ApiKeyRoutingScope
  groups: AccountGroupRef[]
  providerKinds: string[]
}

export interface ApiKeyListResponse {
  items: ApiKey[]
  nextCursor: string | null
  total: number
}

export interface ApiKeyCreateResponse {
  id: string
  prefix: string
  plaintextKey: string
}

export interface ApiKeyRevealResponse {
  id: string
  plaintextKey: string
}

export interface ApiKeyMutationResponse {
  id: string
}

// 请求参数类型：仅定义 API 边界的形状，调用方不依赖显式声明。
interface ApiKeyListParams {
  cursor?: string
  limit: number
  search?: string
  sortBy?: string
  sortDirection?: string
}

export interface ApiKeyWriteParam {
  name: string
  label: string | null
  groupIds: string[]
  maxConcurrency: number
  requestsPerMinute: number
  dailyLimitUsd: string
  weeklyLimitUsd: string
}

interface ApiKeyUpdateParam extends ApiKeyWriteParam {
  id: string
}

interface ApiKeyCreateParam extends ApiKeyWriteParam {
  userId?: string
  customKey?: string
}

interface ApiKeyIdParam {
  id: string
}

export function getApiKeys(data: ApiKeyListParams, options: RequestOptions = {}, scope: 'admin' | 'user' = 'admin') {
  return request<ApiKeyListResponse>({
    url: `/api/${scope}/client-keys`,
    method: 'GET',
    params: data,
    ...options,
  })
}

export function createApiKey(data: ApiKeyCreateParam, scope: 'admin' | 'user' = 'admin') {
  return request<ApiKeyCreateResponse>({
    url: `/api/${scope}/client-keys/create`,
    method: 'POST',
    data,
  })
}

export function updateApiKey(data: ApiKeyUpdateParam, scope: 'admin' | 'user' = 'admin') {
  return request<ApiKeyMutationResponse>({
    url: `/api/${scope}/client-keys/update`,
    method: 'POST',
    data: scope === 'user' ? { id: data.id, name: data.name, label: data.label } : data,
  })
}

export function revealApiKey(data: ApiKeyIdParam, scope: 'admin' | 'user' = 'admin') {
  return request<ApiKeyRevealResponse>({
    url: `/api/${scope}/client-keys/reveal`,
    method: 'GET',
    params: data,
  })
}

export function deleteApiKey(data: ApiKeyIdParam, scope: 'admin' | 'user' = 'admin') {
  return request<ApiKeyMutationResponse>({
    url: `/api/${scope}/client-keys/delete`,
    method: 'POST',
    data,
  })
}

export function disableApiKey(data: ApiKeyIdParam, scope: 'admin' | 'user' = 'admin') {
  return request<ApiKeyMutationResponse>({
    url: `/api/${scope}/client-keys/disable`,
    method: 'POST',
    data,
  })
}

export function enableApiKey(data: ApiKeyIdParam, scope: 'admin' | 'user' = 'admin') {
  return request<ApiKeyMutationResponse>({
    url: `/api/${scope}/client-keys/enable`,
    method: 'POST',
    data,
  })
}
