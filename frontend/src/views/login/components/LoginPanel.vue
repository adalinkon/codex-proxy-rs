<script setup lang="ts">
import { Eye, EyeOff, KeyRound, Mail, Moon, Sun } from '@lucide/vue'
import { computed, shallowRef } from 'vue'

import AppBrandMark from '@/components/AppBrandMark.vue'
import BaseButton from '@/components/base/BaseButton.vue'
import BaseCard from '@/components/base/BaseCard.vue'
import BaseFormItem from '@/components/base/BaseForm/FormItem.vue'
import BaseIconButton from '@/components/base/BaseIconButton.vue'
import BaseInput from '@/components/base/BaseInput.vue'

type ThemeName = 'light' | 'dark'

const props = defineProps<{
  loading: boolean
  submitDisabled: boolean
  effectiveTheme: ThemeName
}>()

const emit = defineEmits<{
  submit: []
  toggleTheme: [event: MouseEvent]
}>()

const username = defineModel<string>('username', { required: true })
const password = defineModel<string>('password', { required: true })
const isPasswordVisible = shallowRef(false)
const passwordType = computed(() => isPasswordVisible.value ? 'text' : 'password')
const passwordToggleLabel = computed(() => isPasswordVisible.value ? '隐藏密码' : '显示密码')
const themeToggleLabel = computed(() => props.effectiveTheme === 'dark' ? '切换浅色模式' : '切换深色模式')
</script>

<template>
  <BaseCard
    as="form"
    padding="none"
    class="login-form w-full max-w-110 rounded-lg px-7.5 pt-10.5 pb-10 max-[400px]:px-5 max-[400px]:py-8"
    aria-labelledby="login-title"
    @submit.prevent="emit('submit')"
  >
    <header class="flex min-w-0 flex-wrap items-center justify-between gap-x-4 gap-y-4">
      <div class="flex min-w-0 items-center gap-3">
        <AppBrandMark class="block size-9.5 shrink-0 select-none" aria-hidden="true" />
        <span class="grid min-w-0 gap-1">
          <strong class="text-[17px] leading-tight font-semibold text-cp-text-heading">
            Codex Proxy RS
          </strong>
          <span class="font-mono text-[10px] leading-tight text-cp-text-secondary">
            ACCOUNT
          </span>
        </span>
      </div>
      <button
        class="login-theme-toggle"
        :class="{ 'is-dark': effectiveTheme === 'dark' }"
        type="button"
        role="switch"
        :aria-checked="effectiveTheme === 'dark'"
        aria-label="深色模式"
        :title="themeToggleLabel"
        @click="emit('toggleTheme', $event)"
      >
        <Sun :size="17" aria-hidden="true" />
        <span class="login-theme-knob" />
        <Moon :size="17" aria-hidden="true" />
      </button>
    </header>

    <h1 id="login-title" class="mt-7 mb-0 text-[34px] leading-tight font-bold text-cp-text-heading">
      账户登录
    </h1>

    <div class="mt-10 grid gap-4">
      <BaseFormItem control-id="login-username">
        <template #label>
          <span class="text-sm leading-tight font-bold text-cp-text">用户名</span>
        </template>
        <BaseInput
          id="login-username"
          v-model="username"
          class="pt-1"
          name="username"
          placeholder="用户名"
          autocomplete="username"
          :disabled="loading"
        >
          <template #prefix>
            <Mail :size="17" />
          </template>
        </BaseInput>
      </BaseFormItem>

      <BaseFormItem control-id="login-password">
        <template #label>
          <span class="text-sm leading-tight font-bold text-cp-text">密码</span>
        </template>
        <BaseInput
          id="login-password"
          v-model="password"
          class="pt-1"
          name="password"
          placeholder="密码"
          :type="passwordType"
          autocomplete="current-password"
          :disabled="loading"
        >
          <template #prefix>
            <KeyRound :size="17" />
          </template>
          <template #suffix>
            <BaseIconButton
              variant="ghost"
              size="sm"
              :label="passwordToggleLabel"
              :disabled="loading"
              :aria-pressed="isPasswordVisible"
              @mousedown.prevent
              @click="isPasswordVisible = !isPasswordVisible"
            >
              <EyeOff v-if="isPasswordVisible" :size="16" />
              <Eye v-else :size="16" />
            </BaseIconButton>
          </template>
        </BaseInput>
      </BaseFormItem>

      <BaseButton
        variant="primary"
        size="lg"
        type="submit"
        class="login-submit"
        :loading="loading"
        :disabled="submitDisabled"
      >
        {{ loading ? '登录中...' : '登录' }}
      </BaseButton>
    </div>
  </BaseCard>
</template>

<style scoped>
.login-form {
  --cp-control-height: 44px;
  --cp-input-bg: var(--cp-color-fill-quaternary);
  --cp-input-hover-bg: var(--cp-color-fill-tertiary);
  --cp-input-active-bg: var(--cp-color-fill-quaternary);
  --cp-color-text-quaternary: var(--cp-color-text-secondary);
  --cp-border-radius: 6px;
  --cp-border-radius-sm: 6px;

  background: var(--cp-card-bg);
  box-shadow: 0 18px 38px -20px var(--cp-login-panel-shadow-color);
}

.login-theme-toggle {
  position: relative;
  display: inline-grid;
  width: 66px;
  height: 32px;
  flex: 0 0 auto;
  grid-template-columns: 1fr 1fr;
  place-items: center;
  padding: 0;
  border: 0;
  border-radius: 20px;
  background: var(--cp-color-fill-quaternary);
  cursor: pointer;
  outline: none;
}

.login-theme-toggle:focus-visible {
  box-shadow: 0 0 0 2px var(--cp-control-outline);
}

.login-theme-toggle > svg {
  position: relative;
  z-index: 1;
}

.login-theme-toggle > svg:first-child {
  color: var(--cp-color-primary-text);
}

.login-theme-toggle > svg:last-child {
  color: var(--cp-color-text-secondary);
}

.login-theme-toggle.is-dark > svg:first-child {
  color: var(--cp-color-text-secondary);
}

.login-theme-toggle.is-dark > svg:last-child {
  color: var(--cp-color-primary-text);
}

.login-theme-knob {
  position: absolute;
  top: 5px;
  left: 5.5px;
  width: 22px;
  height: 22px;
  border-radius: 50%;
  background: var(--cp-color-bg-elevated);
  box-shadow: 0 0 10px var(--cp-login-toggle-shadow-color);
  transition: transform 0.2s ease;
}

.login-theme-toggle.is-dark .login-theme-knob {
  transform: translateX(33px);
}

.login-submit {
  width: 100%;
  height: 44px;
}

.login-submit:disabled {
  background: var(--cp-color-fill-quaternary);
  color: var(--cp-color-text-disabled);
  box-shadow: none;
}

@media (prefers-reduced-motion: reduce) {
  .login-theme-knob {
    transition: none;
  }
}
</style>
