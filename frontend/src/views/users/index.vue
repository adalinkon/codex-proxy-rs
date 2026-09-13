<script setup lang="ts">
import type { UserPolicy, UserRecord } from '@/api/modules/users'
import type { BaseTableSort } from '@/components/base/BaseTable/columns'
import { KeyRound, Pencil, Plus, Power, RefreshCw, RotateCcw, Search, Trash2 } from '@lucide/vue'
import { useSessionStorage } from '@vueuse/core'
import { computed, onMounted, ref, watch } from 'vue'
import { deleteUser, getUsers, resetUserBudget, resetUserPassword, saveUser } from '@/api/modules/users'
import AccountGroupCheckboxGrid from '@/components/AccountGroupCheckboxGrid.vue'
import AccountGroupMarks from '@/components/AccountGroupMarks.vue'
import BaseButton from '@/components/base/BaseButton.vue'
import BaseCard from '@/components/base/BaseCard.vue'
import BaseConfirmModal from '@/components/base/BaseConfirmModal.vue'
import BaseFormItem from '@/components/base/BaseForm/FormItem.vue'
import BaseIconButton from '@/components/base/BaseIconButton.vue'
import BaseInput from '@/components/base/BaseInput.vue'
import BaseModal from '@/components/base/BaseModal/index.vue'
import BaseNumberInput from '@/components/base/BaseNumberInput.vue'
import BasePageHeader from '@/components/base/BasePageHeader.vue'
import BaseSelect from '@/components/base/BaseSelect.vue'
import BaseSwitch from '@/components/base/BaseSwitch.vue'
import BaseTablePagination from '@/components/base/BaseTable/BaseTablePagination.vue'
import { defineTableColumns } from '@/components/base/BaseTable/columns'
import BaseTable from '@/components/base/BaseTable/index.vue'
import { toast } from '@/components/base/BaseToast'
import BudgetCell from '@/components/BudgetCell.vue'
import RequestLimitsCell from '@/components/RequestLimitsCell.vue'
import { useAccountGroupCatalog } from '@/composables/useAccountGroupCatalog'
import { useAsyncAction } from '@/composables/useAsyncAction'
import { useBudgetRollover } from '@/composables/useBudgetRollover'
import { useRequestState } from '@/composables/useRequestState'
import { useRequestUsage } from '@/composables/useRequestUsage'
import { createOperationId } from '@/utils/uuid'
import ApiKeyStatusBadge from '@/views/api-keys/components/ApiKeyStatusBadge.vue'

const users = ref<UserRecord[]>([])
const request = useRequestState()
const { loading, error: failed } = request
const saveAction = useAsyncAction()
const deleteAction = useAsyncAction()
const resetBudgetAction = useAsyncAction()
const { loading: saving } = saveAction
const { loading: deleting } = deleteAction
const { loading: resettingBudget } = resetBudgetAction
const updatingUserId = ref<string | null>(null)
const budgetResetOpen = ref(false)
const budgetResetUser = ref<UserRecord | null>(null)
const budgetResetOperations = useSessionStorage<Record<string, string>>('cpr-user-budget-reset-operations', {})
const deleteOpen = ref(false)
const pendingDelete = ref<UserRecord | null>(null)
const search = ref('')
const open = ref(false)
const editing = ref(false)
const resetOpen = ref(false)
const resetUser = ref<UserRecord | null>(null)
const password = ref('')
const confirmPassword = ref('')
const { groups, loading: groupsLoading, loadGroups } = useAccountGroupCatalog({ immediate: false })
const form = ref<UserPolicy>(emptyForm())
const currentPage = ref(1)
const pageSize = ref(50)
const sort = ref<BaseTableSort>({ key: 'username', direction: 'asc' })
const busy = computed(() => saving.value || deleting.value || resettingBudget.value)
function emptyForm(): UserPolicy {
  return { username: '', role: 'user', enabled: true, allGroups: false, groupIds: [], dailyLimitUsd: '0', weeklyLimitUsd: '0', maxConcurrency: 0, requestsPerMinute: 0, password: '' }
}
function policy(user: UserRecord): UserPolicy {
  return { username: user.username, role: user.role, enabled: user.enabled, allGroups: user.allGroups, groupIds: user.groups.map(g => g.id), dailyLimitUsd: user.dailyLimitUsd, weeklyLimitUsd: user.weeklyLimitUsd, maxConcurrency: user.maxConcurrency, requestsPerMinute: user.requestsPerMinute }
}
const filteredUsers = computed(() => users.value
  .filter(user => user.username.toLowerCase().includes(search.value.toLowerCase()))
  .sort((a, b) => {
    const value = sort.value.key === 'enabled' ? Number(a.enabled) - Number(b.enabled) : a.username.localeCompare(b.username)
    return (sort.value.direction === 'asc' ? 1 : -1) * value
  }))
const totalPages = computed(() => Math.max(1, Math.ceil(filteredUsers.value.length / pageSize.value)))
const rows = computed(() => filteredUsers.value.slice((currentPage.value - 1) * pageSize.value, currentPage.value * pageSize.value))
const requestUsage = useRequestUsage('users', () => rows.value.map(user => user.id))
const pagination = computed(() => ({ currentPage: currentPage.value, pageSize: pageSize.value, total: filteredUsers.value.length }))
const columns = defineTableColumns<UserRecord>([
  { key: 'username', label: '用户名', kind: 'identity', size: 'xl', sortable: true },
  { key: 'role', label: '角色', kind: 'status', size: 'sm' },
  { key: 'enabled', label: '状态', kind: 'status', sortable: true },
  { key: 'groups', label: '分组', kind: 'status', size: 'lg' },
  { key: 'budget', label: '已用 / 限额', kind: 'text', size: '3xl', fixedWidth: true },
  { key: 'limits', label: '并发 / RPM', kind: 'text', size: 'lg' },
  { key: 'keyCount', label: 'Key 数量', kind: 'numeric', size: 'sm' },
  { key: 'actions', label: '操作', kind: 'actions', size: 'xl' },
])
async function load() {
  const requestId = request.start()
  try {
    const data = await getUsers({ signal: request.signal })
    if (request.isCurrent(requestId))
      users.value = data
  }
  catch (cause) {
    request.fail(requestId, cause)
  }
  finally {
    request.finish(requestId)
  }
}
function edit(user?: UserRecord) {
  editing.value = Boolean(user)
  form.value = user ? policy(user) : emptyForm()
  open.value = true
  void loadGroups()
}
async function save() {
  await saveAction.run(async () => {
    await saveUser({ ...form.value, allGroups: form.value.role === 'admin' && form.value.allGroups }, !editing.value)
    open.value = false
    toast.success('用户已保存')
    await load()
  })
}
async function toggle(user: UserRecord, enabled: boolean) {
  if (busy.value)
    return
  await saveAction.run(async () => {
    updatingUserId.value = user.id
    try {
      await saveUser({ ...policy(user), enabled }, false)
      await load()
    }
    finally {
      updatingUserId.value = null
    }
  })
}
function requestDelete(user: UserRecord) {
  pendingDelete.value = user
  deleteOpen.value = true
}
function requestBudgetReset(user: UserRecord) {
  budgetResetUser.value = user
  budgetResetOpen.value = true
}
async function submitBudgetReset() {
  if (!budgetResetUser.value || busy.value)
    return
  const id = budgetResetUser.value.id
  await resetBudgetAction.run(async () => {
    // 未确认结果的操作跨弹窗和页面刷新复用标识，防止重试再次清零。
    const operationId = Object.hasOwn(budgetResetOperations.value, id) ? budgetResetOperations.value[id] : createOperationId()
    budgetResetOperations.value = { ...budgetResetOperations.value, [id]: operationId }
    await resetUserBudget(id, operationId)
    const remaining = { ...budgetResetOperations.value }
    delete remaining[id]
    budgetResetOperations.value = remaining
    budgetResetOpen.value = false
    toast.success('用户额度已重置')
    await load()
  })
}
watch(budgetResetOpen, (value) => {
  if (!value)
    budgetResetUser.value = null
})
useBudgetRollover(users, async () => {
  if (!loading.value && !busy.value)
    await load()
})
async function removeUser() {
  if (!pendingDelete.value || busy.value)
    return
  const id = pendingDelete.value.id
  await deleteAction.run(async () => {
    await deleteUser(id)
    users.value = users.value.filter(user => user.id !== id)
    deleteOpen.value = false
    toast.success('用户已删除')
    await load()
  })
}
function changePageSize(value: number) {
  pageSize.value = value
  currentPage.value = 1
}
function changeSort(value: BaseTableSort | undefined) {
  sort.value = value ?? { key: 'username', direction: 'asc' }
  currentPage.value = 1
}
watch(search, () => {
  currentPage.value = 1
})
watch(totalPages, (value) => {
  currentPage.value = Math.min(currentPage.value, value)
})
watch(deleteOpen, (value) => {
  if (!value)
    pendingDelete.value = null
})
function reset(user: UserRecord) {
  resetUser.value = user
  password.value = ''
  confirmPassword.value = ''
  resetOpen.value = true
}
async function submitReset() {
  if (!resetUser.value || saving.value)
    return
  if (password.value !== confirmPassword.value) {
    toast.error('两次密码不一致')
    return
  }
  const id = resetUser.value.id
  await saveAction.run(async () => {
    await resetUserPassword(id, password.value)
    resetOpen.value = false
    toast.success('密码已重置')
  })
}
watch(open, (value) => {
  if (!value)
    form.value = emptyForm()
})
watch(resetOpen, (value) => {
  if (!value) {
    password.value = ''
    confirmPassword.value = ''
    resetUser.value = null
  }
})
onMounted(load)
</script>

<template>
  <div class="flex h-full min-h-0 w-full flex-col overflow-hidden">
    <BasePageHeader class="h-17" title="用户" />
    <BaseCard class="mt-5 flex h-[calc(100dvh-136px)] min-h-125 flex-col">
      <template #header>
        <div class="flex w-full flex-wrap items-center gap-3" role="group" aria-label="用户筛选与操作">
          <div class="min-w-0 flex-1 md:w-96 md:flex-none">
            <BaseInput v-model="search" class="w-full" aria-label="搜索用户名" placeholder="搜索用户名">
              <template #prefix>
                <Search class="size-4.5 text-cp-text-tertiary" />
              </template>
            </BaseInput>
          </div>
          <div class="flex shrink-0 items-center justify-end gap-2 md:ml-auto">
            <BaseIconButton variant="ghost" size="sm" label="刷新用户" :disabled="loading || busy" @click="load">
              <RefreshCw class="size-3.5 text-cp-link" />
            </BaseIconButton>
            <BaseButton variant="primary" :disabled="busy" @click="edit()">
              <template #icon>
                <Plus class="size-4" />
              </template>创建用户
            </BaseButton>
          </div>
        </div>
      </template>
      <template #body>
        <div class="flex h-full min-h-0 flex-col">
          <p v-if="failed" role="alert" class="m-0 mb-3 text-cp-error">
            用户加载失败
          </p>
          <BaseTable class="min-h-0 flex-1" :columns="columns" :rows="rows" :loading="loading" :sort="sort" empty-text="暂无用户" @sort-change="changeSort">
            <template #username="{ row }">
              <span class="text-cp font-bold text-cp-text">{{ row.username }}</span>
            </template>
            <template #role="{ row }">
              {{ row.role === 'admin' ? '管理员' : '普通用户' }}
            </template>
            <template #enabled="{ row }">
              <ApiKeyStatusBadge :api-key="row" />
            </template>
            <template #budget="{ row }">
              <BudgetCell :budget="row" :name="row.username" />
            </template>
            <template #limits="{ row }">
              <RequestLimitsCell :max-concurrency="row.maxConcurrency" :requests-per-minute="row.requestsPerMinute" :current-concurrency="requestUsage.get(row.id)?.currentConcurrency" :current-rpm="requestUsage.get(row.id)?.currentRpm" />
            </template>
            <template #groups="{ row }">
              <div class="grid w-full justify-items-center gap-1.5">
                <span v-if="row.allGroups" class="inline-flex h-6 items-center rounded-lg bg-cp-warning-container px-2 text-cp-xs font-bold text-cp-warning-on-container">全部账号</span>
                <AccountGroupMarks v-else :groups="row.groups" />
              </div>
            </template>
            <template #actions="{ row }">
              <div class="flex items-center justify-start gap-0.5">
                <BaseIconButton variant="ghost" size="sm" label="修改配置" :disabled="busy" @click.stop="edit(row)">
                  <Pencil class="size-3.5 text-cp-link" />
                </BaseIconButton>
                <BaseIconButton variant="ghost" size="sm" label="重置额度" :disabled="busy" :loading="resettingBudget && budgetResetUser?.id === row.id" @click.stop="requestBudgetReset(row)">
                  <RotateCcw class="size-3.5 text-cp-warning" />
                </BaseIconButton>
                <BaseIconButton variant="ghost" size="sm" label="修改密码" :disabled="busy" @click.stop="reset(row)">
                  <KeyRound class="size-3.5 text-cp-primary-text" />
                </BaseIconButton>
                <BaseIconButton variant="ghost" size="sm" :label="row.enabled ? '禁用用户' : '启用用户'" :loading="updatingUserId === row.id" :disabled="busy" @click.stop="toggle(row, !row.enabled)">
                  <Power class="size-3.5" :class="row.enabled ? 'text-cp-warning' : 'text-cp-success'" />
                </BaseIconButton>
                <BaseIconButton variant="ghost" size="sm" label="删除用户" :disabled="busy" @click.stop="requestDelete(row)">
                  <Trash2 class="size-3.5 text-cp-error" />
                </BaseIconButton>
              </div>
            </template>
          </BaseTable>
          <BaseTablePagination :pagination="pagination" :loading="loading || busy" @page-change="currentPage = $event" @page-size-change="changePageSize" />
        </div>
      </template>
    </BaseCard>
    <BaseConfirmModal v-model="budgetResetOpen" title="重置额度" :description="budgetResetUser?.username" confirm-text="重置额度" :loading="resettingBudget" @confirm="submitBudgetReset">
      <p class="m-0">
        将该用户的日、周已用金额归零，周周期从今天北京时间零点重新起算。
      </p>
      <p class="mb-0 mt-3">
        限额配置、Key 附加额度及历史使用记录保留。正在执行的请求完成后仍会累计费用。
      </p>
    </BaseConfirmModal>
    <BaseConfirmModal v-model="deleteOpen" title="删除用户" :description="pendingDelete?.username" confirm-text="删除用户" destructive :loading="deleting" @confirm="removeUser">
      删除后该用户无法登录，所属 Key 将被撤销。历史费用及用户名保留，不能同名重建。
    </BaseConfirmModal>
    <BaseModal v-model="open" :title="editing ? '修改配置' : '创建用户'" size="lg" :dismissible="!saving">
      <form id="user-form" class="grid gap-5" @submit.prevent="save">
        <BaseFormItem label="用户名" required>
          <BaseInput v-model="form.username" :disabled="editing || saving" aria-label="用户名" autocomplete="off" required maxlength="128" />
        </BaseFormItem>
        <BaseFormItem v-if="!editing" label="初始密码" required>
          <BaseInput v-model="form.password" type="password" aria-label="初始密码" autocomplete="new-password" required minlength="12" maxlength="1024" />
        </BaseFormItem>
        <BaseFormItem label="角色">
          <BaseSelect v-model="form.role" :options="[{ label: '普通用户', value: 'user' }, { label: '管理员', value: 'admin' }]" />
        </BaseFormItem>
        <div class="grid gap-5 sm:grid-cols-2">
          <BaseFormItem label="日限额（美元）">
            <BaseInput v-model="form.dailyLimitUsd" type="number" min="0" step="any" aria-label="日限额" required />
          </BaseFormItem>
          <BaseFormItem label="周限额（美元）">
            <BaseInput v-model="form.weeklyLimitUsd" type="number" min="0" step="any" aria-label="周限额" required />
          </BaseFormItem>
          <BaseNumberInput v-model="form.maxConcurrency" label="共享并发" :min="0" />
          <BaseNumberInput v-model="form.requestsPerMinute" label="共享 RPM" :min="0" />
        </div>
        <BaseSwitch v-if="form.role === 'admin'" v-model="form.allGroups" label="全部分组" />
        <BaseFormItem label="可用账号分组">
          <AccountGroupCheckboxGrid v-if="form.role !== 'admin' || !form.allGroups" v-model="form.groupIds" :groups="groups" :loading="groupsLoading" :disabled="saving" />
        </BaseFormItem>
      </form>
      <template #footer>
        <BaseButton :disabled="saving" @click="open = false">
          取消
        </BaseButton><BaseButton variant="primary" type="submit" form="user-form" :loading="saving">
          保存
        </BaseButton>
      </template>
    </BaseModal>
    <BaseModal v-model="resetOpen" title="修改密码" size="sm" :dismissible="!saving">
      <form id="reset-password" class="grid gap-5" @submit.prevent="submitReset">
        <p class="m-0 break-all text-cp-text">
          {{ resetUser?.username }}
        </p>
        <BaseFormItem label="新密码">
          <BaseInput v-model="password" type="password" aria-label="新密码" autocomplete="new-password" required minlength="12" maxlength="1024" />
        </BaseFormItem>
        <BaseFormItem label="确认密码">
          <BaseInput v-model="confirmPassword" type="password" aria-label="确认密码" autocomplete="new-password" required />
        </BaseFormItem>
      </form>
      <template #footer>
        <BaseButton variant="primary" form="reset-password" type="submit" :loading="saving">
          修改密码
        </BaseButton>
      </template>
    </BaseModal>
  </div>
</template>
