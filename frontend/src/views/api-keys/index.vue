<script setup lang="ts">
import type { UserRecord } from '@/api/modules/users'
import { computed, ref, watch } from 'vue'
import { getMyProfile, getUsers } from '@/api/modules/users'

import BaseCard from '@/components/base/BaseCard.vue'
import BaseCheckbox from '@/components/base/BaseCheckbox.vue'
import BaseConfirmModal from '@/components/base/BaseConfirmModal.vue'
import BasePageHeader from '@/components/base/BasePageHeader.vue'
import BaseTablePagination from '@/components/base/BaseTable/BaseTablePagination.vue'
import BaseTable from '@/components/base/BaseTable/index.vue'
import LastUsedAtCell from '@/components/LastUsedAtCell.vue'
import RequestLimitsCell from '@/components/RequestLimitsCell.vue'
import { usePageSelection } from '@/composables/usePageSelection'
import { useRequestUsage } from '@/composables/useRequestUsage'
import { useAuthStore } from '@/stores/modules/auth'
import ApiKeyActions from './components/ApiKeyActions.vue'
import ApiKeyBudgetCell from './components/ApiKeyBudgetCell.vue'
import ApiKeyCreateModal from './components/ApiKeyCreateModal.vue'
import ApiKeyFilters from './components/ApiKeyFilters.vue'
import ApiKeyIdentityCell from './components/ApiKeyIdentityCell.vue'
import ApiKeyPrefixCell from './components/ApiKeyPrefixCell.vue'
import ApiKeyScopeCell from './components/ApiKeyScopeCell.vue'
import ApiKeyStatusBadge from './components/ApiKeyStatusBadge.vue'
import ApiKeyUseModal from './components/ApiKeyUseModal.vue'
import { useApiKeyMutations } from './composables/useApiKeyMutations'
import { useApiKeysQuery } from './composables/useApiKeysQuery'
import { useApiKeyUse } from './composables/useApiKeyUse'
import { apiKeyColumns } from './constants'

const props = withDefaults(defineProps<{ scope?: 'admin' | 'user' }>(), { scope: 'admin' })
const selectedIds = ref<Set<string>>(new Set())
const {
  loading,
  apiKeys,
  loadApiKeys,
  searchQuery,
  sort,
  apiKeyPagination,
  handlePageChange,
  handlePageSizeChange,
  handleSortChange,
} = useApiKeysQuery(props.scope)
const requestUsage = useRequestUsage(props.scope === 'user' ? 'my-keys' : 'keys', () => apiKeys.value.map(key => key.id))

const auth = useAuthStore()
const owners = ref<UserRecord[]>([])
const loadingGroups = ref(false)
const ownerOptions = computed(() => owners.value.filter(user => user.enabled).map(user => ({ label: user.username, value: user.id })))
function canCreate(userId: string) {
  return !loadingGroups.value && owners.value.some(user => user.id === userId && user.enabled)
}

const {
  showFormModal,
  showDeleteModal,
  showSingleDeleteModal,
  showKeyModal,
  createdKey,
  createdKeyName,
  editingKey,
  pendingDeleteKey,
  savingKey,
  deletingKey,
  batchDeleting,
  updatingStatusKeyIds,
  revealingKeyIds,
  form,
  openCreate,
  openEdit,
  requestSave,
  requestDeleteKey,
  handleDelete,
  handleBatchDelete,
  handleToggleStatus,
  copyToClipboard,
  revealPlaintextKey,
  copyApiKey,
} = useApiKeyMutations({ selectedIds, reload: loadApiKeys, scope: props.scope, canCreate })
const groups = computed(() => owners.value.find(user => user.id === form.value.userId)?.groups ?? [])
const createUnavailable = computed(() => !canCreate(form.value.userId))

const { allSelected, indeterminate, selectedRowKeys, toggleSelection, toggleAll } = usePageSelection(
  apiKeys,
  selectedIds,
)

const {
  showUseKeyModal,
  selectedUseKey,
  openAiBaseUrl,
  importCreatedKeyToCcs,
  openUseKeyModal,
  importToCcs,
} = useApiKeyUse({
  createdKey,
  createdKeyName,
  revealPlaintextKey,
})

watch(showFormModal, async (open, _, onCleanup) => {
  if (!open)
    return
  const controller = new AbortController()
  onCleanup(() => controller.abort())
  owners.value = []
  loadingGroups.value = true
  if (!editingKey.value)
    form.value.userId = auth.user?.id ?? ''
  try {
    const result = props.scope === 'admin'
      ? await getUsers({ signal: controller.signal })
      : [await getMyProfile({ signal: controller.signal })]
    if (!controller.signal.aborted)
      owners.value = result
  }
  catch {
    // 请求层统一提示失败，创建保持禁用，避免使用过期的归属或分组。
  }
  finally {
    if (!controller.signal.aborted)
      loadingGroups.value = false
  }
})
watch(() => form.value.userId, () => {
  if (!editingKey.value)
    form.value.groupIds = []
})
</script>

<template>
  <div class="flex h-full min-h-0 w-full flex-col overflow-hidden">
    <BasePageHeader
      class="h-17"
      title="API 密钥"
      description="创建和管理 API 密钥，并设置每个密钥可以使用的账号"
    />

    <BaseCard
      class="mt-5 flex h-[calc(100dvh-136px)] min-h-125 flex-col"
    >
      <template #header>
        <ApiKeyFilters
          v-model:search="searchQuery"
          :batch-deleting="batchDeleting"
          :selected-count="selectedIds.size"
          @create="openCreate"
          @delete-selected="showDeleteModal = true"
        />
      </template>

      <template #body>
        <div class="flex h-full min-h-0 flex-col">
          <BaseTable
            class="min-h-0 flex-1"
            :columns="apiKeyColumns"
            :rows="apiKeys"
            :loading="loading"
            :selected-row-keys="selectedRowKeys"
            :sort="sort"
            empty-text="暂无 API Key"
            @sort-change="handleSortChange"
          >
            <template #header-selection>
              <BaseCheckbox
                :model-value="allSelected"
                :indeterminate="indeterminate"
                label="选择当前页密钥"
                @update:model-value="toggleAll"
              />
            </template>
            <template #selection="{ row }">
              <BaseCheckbox
                :model-value="selectedIds.has(row.id)"
                label="选择密钥"
                @update:model-value="toggleSelection(row.id)"
              />
            </template>
            <template #identity="{ row }">
              <ApiKeyIdentityCell :api-key="row" />
            </template>
            <template #prefix="{ row }">
              <ApiKeyPrefixCell
                :prefix="row.prefix"
                :revealing="revealingKeyIds.has(row.id)"
                @copy="copyApiKey(row)"
              />
            </template>
            <template #scope="{ row }">
              <ApiKeyScopeCell :api-key="row" />
            </template>
            <template #budget="{ row }">
              <ApiKeyBudgetCell :api-key="row" />
            </template>
            <template #limits="{ row }">
              <RequestLimitsCell :max-concurrency="row.maxConcurrency" :requests-per-minute="row.requestsPerMinute" :current-concurrency="requestUsage.get(row.id)?.currentConcurrency" :current-rpm="requestUsage.get(row.id)?.currentRpm" />
            </template>
            <template #enabled="{ row }">
              <ApiKeyStatusBadge :api-key="row" />
            </template>
            <template #lastUsedAt="{ row }">
              <LastUsedAtCell :value="row.lastUsedAt" />
            </template>
            <template #actions="{ row }">
              <ApiKeyActions
                :api-key="row"
                :deleting="deletingKey"
                :revealing="revealingKeyIds.has(row.id)"
                :updating-status="updatingStatusKeyIds.has(row.id)"
                @edit="openEdit"
                @delete="requestDeleteKey"
                @import-ccs="importToCcs"
                @toggle="handleToggleStatus"
                @use="openUseKeyModal"
              />
            </template>
          </BaseTable>
          <BaseTablePagination
            :pagination="apiKeyPagination"
            :loading="loading"
            @page-change="handlePageChange"
            @page-size-change="handlePageSizeChange"
          />
        </div>
      </template>
    </BaseCard>

    <ApiKeyCreateModal
      v-model="showFormModal"
      v-model:created-open="showKeyModal"
      v-model:form="form"
      :groups="groups"
      :group-loading="loadingGroups"
      :editing="Boolean(editingKey)"
      :policy-readonly="scope === 'user' && Boolean(editingKey)"
      :selecting-owner="scope === 'admin'"
      :users="ownerOptions"
      :create-unavailable="createUnavailable"
      :created-key="createdKey"
      :saving="savingKey"
      @copy="copyToClipboard"
      @save="requestSave"
      @import-ccs="importCreatedKeyToCcs"
    />

    <ApiKeyUseModal
      v-model="showUseKeyModal"
      :api-key="selectedUseKey"
      :api-base-url="openAiBaseUrl"
      @copy="copyToClipboard"
    />

    <BaseConfirmModal
      v-model="showDeleteModal"
      title="确认删除"
      description="删除后这些 API Key 将立即失效，此操作不可撤销"
      destructive
      confirm-text="确认删除"
      :loading="batchDeleting"
      @confirm="handleBatchDelete"
    >
      <p class="m-0">
        确定删除选中的 {{ selectedIds.size }} 个 API Key 吗？
      </p>
    </BaseConfirmModal>

    <BaseConfirmModal
      v-model="showSingleDeleteModal"
      title="删除 API Key"
      description="删除后该 API Key 将立即失效，此操作不可撤销"
      destructive
      confirm-text="确认删除"
      :loading="deletingKey"
      @confirm="handleDelete"
    >
      <p class="m-0">
        确定删除 {{ pendingDeleteKey?.name || pendingDeleteKey?.prefix || '该 API Key' }} 吗？
      </p>
    </BaseConfirmModal>
  </div>
</template>
