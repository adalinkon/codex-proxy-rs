<script setup lang="ts">
import type { UserRecord } from '@/api/modules/users'
import { FolderTree, KeyRound, LogOut, RefreshCw, UserRound, Wallet } from '@lucide/vue'
import { computed, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { changeMyPassword, getMyProfile } from '@/api/modules/users'
import AccountGroupMarks from '@/components/AccountGroupMarks.vue'
import BaseButton from '@/components/base/BaseButton.vue'
import BaseCard from '@/components/base/BaseCard.vue'
import BaseEmpty from '@/components/base/BaseEmpty.vue'
import BaseFormItem from '@/components/base/BaseForm/FormItem.vue'
import BaseIconButton from '@/components/base/BaseIconButton.vue'
import BaseInput from '@/components/base/BaseInput.vue'
import BaseModal from '@/components/base/BaseModal/index.vue'
import BaseMotionIcon from '@/components/base/BaseMotionIcon.vue'
import BasePageHeader from '@/components/base/BasePageHeader.vue'
import { toast } from '@/components/base/BaseToast'
import BudgetCell from '@/components/BudgetCell.vue'
import RequestLimitsCell from '@/components/RequestLimitsCell.vue'
import { useBudgetRollover } from '@/composables/useBudgetRollover'
import { useRequestState } from '@/composables/useRequestState'
import { useRequestUsage } from '@/composables/useRequestUsage'
import { useAuthStore } from '@/stores/modules/auth'
import ApiKeyStatusBadge from '@/views/api-keys/components/ApiKeyStatusBadge.vue'

const router = useRouter()
const auth = useAuthStore()
const profile = ref<UserRecord | null>(null)
const requestUsage = useRequestUsage('me', () => profile.value ? [profile.value.id] : [])
const request = useRequestState()
const { loading, error } = request
const passwordOpen = ref(false)
const savingPassword = ref(false)
const loggingOut = ref(false)
const currentPassword = ref('')
const newPassword = ref('')
const confirmation = ref('')
const windows = computed(() => profile.value
  ? [
      { name: '日' as const, limit: profile.value.dailyLimitUsd, used: profile.value.dailyUsedUsd },
      { name: '周' as const, limit: profile.value.weeklyLimitUsd, used: profile.value.weeklyUsedUsd },
    ]
  : [])

async function load() {
  const requestId = request.start()
  try {
    const data = await getMyProfile({ signal: request.signal })
    if (request.isCurrent(requestId))
      profile.value = data
  }
  catch (cause) {
    request.fail(requestId, cause)
  }
  finally {
    request.finish(requestId)
  }
}

async function logout() {
  if (loggingOut.value)
    return
  loggingOut.value = true
  try {
    await auth.logout()
    await router.replace('/login')
  }
  finally {
    loggingOut.value = false
  }
}

function clearPassword() {
  currentPassword.value = ''
  newPassword.value = ''
  confirmation.value = ''
}

async function changePassword() {
  if (savingPassword.value)
    return
  if (newPassword.value !== confirmation.value) {
    toast.error('两次密码不一致')
    return
  }
  savingPassword.value = true
  try {
    await changeMyPassword(currentPassword.value, newPassword.value)
    clearPassword()
    passwordOpen.value = false
    toast.success('密码已修改，请重新登录')
    await logout()
  }
  catch { /* 请求层展示错误，保留弹窗供用户更正。 */ }
  finally {
    savingPassword.value = false
  }
}

watch(passwordOpen, open => !open && clearPassword())
onMounted(load)
useBudgetRollover(() => profile.value ? [profile.value] : [], async () => {
  if (!loading.value && !savingPassword.value && !loggingOut.value)
    await load()
})
</script>

<template>
  <div class="w-full min-w-0">
    <BasePageHeader title="个人资料">
      <template #actions>
        <BaseIconButton class="text-cp-primary-text" size="md" label="刷新个人资料" :loading="loading" :disabled="loading || savingPassword || loggingOut" @click="load">
          <template #loading>
            <RefreshCw class="animate-spin motion-reduce:animate-none" :size="19" />
          </template>
          <RefreshCw :size="19" />
        </BaseIconButton>
      </template>
    </BasePageHeader>

    <div v-if="error" role="alert" class="mt-6 flex flex-wrap items-center justify-between gap-3 rounded-cp bg-cp-error-container px-4 py-3 text-cp-sm text-cp-error-on-container">
      <span>个人资料加载失败</span>
      <BaseButton size="sm" :disabled="loading" @click="load">
        重试
      </BaseButton>
    </div>

    <div class="mt-6 grid grid-cols-1 gap-6" :aria-busy="loading">
      <template v-if="profile">
        <BaseCard title="基本资料">
          <template #title>
            <span class="flex items-center gap-3">
              <BaseMotionIcon class="inline-flex size-8.5 shrink-0 items-center justify-center rounded-cp-lg bg-cp-blue-container text-cp-blue-on-container">
                <UserRound :size="18" />
              </BaseMotionIcon>
              基本资料
            </span>
          </template>
          <template #actions>
            <div class="flex items-center gap-1">
              <BaseIconButton variant="ghost" size="sm" label="修改密码" :disabled="savingPassword || loggingOut" @click="passwordOpen = true">
                <KeyRound class="size-4 text-cp-primary-text" />
              </BaseIconButton>
              <BaseIconButton variant="ghost" size="sm" label="退出登录" :loading="loggingOut" :disabled="savingPassword" @click="logout">
                <LogOut class="size-4 text-cp-error" />
              </BaseIconButton>
            </div>
          </template>
          <dl class="m-0 grid grid-cols-[6rem_minmax(0,1fr)] items-center gap-x-4 gap-y-4 text-cp-sm">
            <dt class="text-cp-text-secondary">
              用户名
            </dt>
            <dd class="m-0 break-all text-cp-lg font-heavy text-cp-text">
              {{ profile.username }}
            </dd>
            <dt class="text-cp-text-secondary">
              角色
            </dt>
            <dd class="m-0 text-cp-text">
              {{ profile.role === 'admin' ? '管理员' : '普通用户' }}
            </dd>
            <dt class="text-cp-text-secondary">
              状态
            </dt>
            <dd class="m-0">
              <ApiKeyStatusBadge :api-key="profile" />
            </dd>
            <dt class="text-cp-text-secondary">
              Key 数量
            </dt>
            <dd class="m-0 font-mono font-bold tabular-nums text-cp-text">
              {{ profile.keyCount }}
            </dd>
          </dl>
        </BaseCard>

        <BaseCard title="额度与限制">
          <template #title>
            <span class="flex items-center gap-3">
              <BaseMotionIcon class="inline-flex size-8.5 shrink-0 items-center justify-center rounded-cp-lg bg-cp-green-container text-cp-green-on-container">
                <Wallet :size="18" />
              </BaseMotionIcon>
              额度与限制
            </span>
          </template>
          <div class="grid gap-5">
            <div class="text-cp-sm font-emphasis text-cp-text-secondary">
              已用 / 限额
            </div>
            <div v-for="window in windows" :key="window.name" class="grid min-w-0 gap-2.5">
              <BudgetCell :budget="profile" :name="profile.username" :period="window.name" size="lg" reset-display="datetime" />
              <progress v-if="Number(window.limit) > 0" class="h-1.5 w-full accent-cp-primary" :aria-label="`${window.name}额度使用进度`" :value="Math.min(Number(window.used), Number(window.limit))" :max="Number(window.limit)" />
            </div>
            <div class="rounded-cp-lg bg-cp-fill-alter/70 p-3">
              <RequestLimitsCell size="md" :max-concurrency="profile.maxConcurrency" :requests-per-minute="profile.requestsPerMinute" :current-concurrency="requestUsage.get(profile.id)?.currentConcurrency" :current-rpm="requestUsage.get(profile.id)?.currentRpm" />
            </div>
          </div>
        </BaseCard>

        <BaseCard title="可用分组">
          <template #title>
            <span class="flex items-center gap-3">
              <BaseMotionIcon class="inline-flex size-8.5 shrink-0 items-center justify-center rounded-cp-lg bg-cp-cyan-container text-cp-cyan-on-container">
                <FolderTree :size="18" />
              </BaseMotionIcon>
              可用分组
            </span>
          </template>
          <BaseEmpty v-if="!profile.groups.length" title="无可用分组" :icon="FolderTree" size="sm" surface="none" />
          <ul v-else class="m-0 grid list-none gap-3 p-0">
            <li v-for="group in profile.groups" :key="group.id" class="flex min-w-0 items-center gap-3 text-cp-sm">
              <AccountGroupMarks :groups="[group]" class="shrink-0" />
              <span class="min-w-0 break-all font-emphasis text-cp-text">{{ group.name }}</span>
              <span v-if="!group.enabled" class="shrink-0 text-cp-text-tertiary">已禁用</span>
            </li>
          </ul>
        </BaseCard>
      </template>
      <template v-else-if="loading">
        <span class="sr-only" role="status">正在加载个人资料</span>
        <BaseCard v-for="title in ['基本资料', '额度与限制', '可用分组']" :key="title" :title="title" aria-hidden="true">
          <div class="grid gap-4 motion-safe:animate-pulse">
            <div class="h-4 w-40 max-w-full rounded-cp bg-cp-fill-tertiary" />
            <div class="h-4 w-64 max-w-full rounded-cp bg-cp-fill-tertiary" />
            <div class="h-4 w-48 max-w-full rounded-cp bg-cp-fill-tertiary" />
          </div>
        </BaseCard>
      </template>
    </div>

    <BaseModal v-model="passwordOpen" title="修改密码" size="sm" tone="info" :dismissible="!savingPassword">
      <template #icon>
        <KeyRound :size="20" />
      </template>
      <form id="profile-password-form" class="grid gap-5" @submit.prevent="changePassword">
        <BaseFormItem label="当前密码" required>
          <BaseInput v-model="currentPassword" type="password" aria-label="当前密码" autocomplete="current-password" required :disabled="savingPassword" />
        </BaseFormItem>
        <BaseFormItem label="新密码" required>
          <BaseInput v-model="newPassword" type="password" aria-label="新密码" autocomplete="new-password" minlength="12" maxlength="1024" required :disabled="savingPassword" />
        </BaseFormItem>
        <BaseFormItem label="确认新密码" required>
          <BaseInput v-model="confirmation" type="password" aria-label="确认新密码" autocomplete="new-password" required :disabled="savingPassword" />
        </BaseFormItem>
      </form>
      <template #footer>
        <BaseButton variant="secondary" :disabled="savingPassword" @click="passwordOpen = false">
          取消
        </BaseButton>
        <BaseButton variant="primary" type="submit" form="profile-password-form" :loading="savingPassword">
          修改密码
        </BaseButton>
      </template>
    </BaseModal>
  </div>
</template>
