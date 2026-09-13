import type { RequestOptions } from '../request'
import request from '../request'

export interface BudgetAmounts {
  dailyLimitUsd: string
  weeklyLimitUsd: string
  dailyUsedUsd: string
  weeklyUsedUsd: string
  dailyResetsAt: string | null
  weeklyResetsAt: string | null
}
export interface UserRecord extends BudgetAmounts {
  id: string
  username: string
  role: 'admin' | 'user'
  enabled: boolean
  allGroups: boolean
  groups: { id: string, name: string, color: string, enabled: boolean }[]
  maxConcurrency: number
  requestsPerMinute: number
  keyCount: number
  dailyRemainingUsd: string | null
  weeklyRemainingUsd: string | null
}
export interface UserPolicy {
  username: string
  role: 'admin' | 'user'
  enabled: boolean
  allGroups: boolean
  groupIds: string[]
  maxConcurrency: number
  requestsPerMinute: number
  dailyLimitUsd: string
  weeklyLimitUsd: string
  password?: string
}
export function getUsers(options: RequestOptions = {}) {
  return request<UserRecord[]>({ url: '/api/admin/users', method: 'GET', ...options })
}
export function saveUser(data: UserPolicy, create: boolean) {
  return request<UserRecord>({ url: `/api/admin/users/${create ? 'create' : 'update'}`, method: 'POST', data })
}
export function resetUserPassword(id: string, password: string) {
  return request<void>({ url: '/api/admin/users/reset-password', method: 'POST', data: { id, password } })
}
export function resetUserBudget(id: string, operationId: string) {
  return request<{ resetAt: string }>({
    url: '/api/admin/users/reset-budget',
    method: 'POST',
    data: { id, operationId },
  })
}
export function deleteUser(id: string) {
  return request<void>({
    url: '/api/admin/users/delete',
    method: 'POST',
    data: { id },
  })
}
export function getMyProfile(options: RequestOptions = {}) {
  return request<UserRecord>({ url: '/api/user/profile', method: 'GET', ...options })
}
export function changeMyPassword(currentPassword: string, newPassword: string) {
  return request<void>({ url: '/api/user/password', method: 'POST', data: { currentPassword, newPassword } })
}
