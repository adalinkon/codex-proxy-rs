<script setup lang="ts">
import type { BudgetAmounts } from '@/api/modules/users'
import { computed } from 'vue'
import BasePopover from '@/components/base/BasePopover.vue'
import { useUiClock } from '@/composables/useUiClock'
import { formatBudgetAmount as amount, formatBudgetResetCountdown } from '@/utils/budget'
import { formatDateTime } from '@/utils/date'

const props = defineProps<{ budget: BudgetAmounts, name: string, period?: '日' | '周', size?: 'sm' | 'lg', resetDisplay?: 'countdown' | 'datetime' }>()
const now = useUiClock()
const windows = computed(() => [
  { label: '日' as const, heading: '日用量', used: props.budget.dailyUsedUsd, limit: props.budget.dailyLimitUsd, reset: props.budget.dailyResetsAt },
  { label: '周' as const, heading: '周用量', used: props.budget.weeklyUsedUsd, limit: props.budget.weeklyLimitUsd, reset: props.budget.weeklyResetsAt },
].filter(window => !props.period || window.label === props.period))
</script>

<template>
  <BasePopover class="w-full min-w-0" trigger="hover-click" placement="right" :hover-delay="240">
    <template #trigger="{ open }">
      <button
        type="button"
        class="grid w-full min-w-0 cursor-pointer gap-1 rounded-sm border-0 bg-transparent p-0 text-left tabular-nums outline-none focus-visible:ring-2 focus-visible:ring-cp-control-outline"
        :class="size === 'lg' ? 'font-mono text-lg font-bold' : 'text-xs'"
        :aria-label="`查看 ${name} 的费用用量`"
        :aria-expanded="open"
        aria-haspopup="dialog"
      >
        <span
          v-for="window in windows"
          :key="window.label"
          class="grid min-w-0 items-baseline gap-x-2 gap-y-1"
          :class="resetDisplay === 'datetime' ? 'grid-cols-[1.25rem_minmax(0,1fr)] sm:grid-cols-[1.25rem_12rem_minmax(0,1fr)]' : 'grid-cols-[1rem_6.5rem_minmax(0,1fr)]'"
        >
          <span class="shrink-0 text-cp-text-tertiary">{{ window.label }}</span>
          <span class="min-w-0 max-w-full truncate" :class="Number(window.limit) > 0 && Number(window.used) >= Number(window.limit) ? 'text-cp-error' : 'text-cp-text'">
            ${{ amount(window.used) }} / {{ Number(window.limit) === 0 ? '∞' : `$${amount(window.limit)}` }}
          </span>
          <span
            class="min-w-0 text-xs font-normal text-cp-text-tertiary"
            :class="resetDisplay === 'datetime' ? 'col-start-2 sm:col-start-auto sm:pl-2' : 'whitespace-nowrap pl-2'"
            :title="window.reset ? `重置（北京时间）：${formatDateTime(window.reset, '—', 'Asia/Shanghai')}` : '重置时间暂不可用'"
          >
            <template v-if="resetDisplay === 'datetime'">重置：{{ window.reset ? formatDateTime(window.reset, '—', 'Asia/Shanghai') : '暂不可用' }}（北京时间）</template>
            <template v-else>{{ formatBudgetResetCountdown(window.reset, window.label, now.getTime()) }}</template>
          </span>
        </span>
      </button>
    </template>

    <section class="grid min-w-56 max-w-[calc(100vw-1rem)] gap-3 p-3" role="dialog" aria-label="费用用量详情（美元）">
      <div v-for="window in windows" :key="window.label" class="grid gap-1">
        <div class="flex items-baseline justify-between gap-6 text-cp-sm">
          <span class="shrink-0 text-cp-text-secondary">{{ window.heading }}</span>
          <span class="min-w-0 break-all text-right font-mono tabular-nums">
            <span :class="Number(window.limit) > 0 && Number(window.used) >= Number(window.limit) ? 'text-cp-error' : 'text-cp-text'">${{ window.used }}</span>
            <span class="text-cp-text-tertiary"> / {{ Number(window.limit) === 0 ? '∞' : `$${window.limit}` }}</span>
          </span>
        </div>
        <div class="flex items-baseline justify-between gap-3 text-cp-xs text-cp-text-tertiary">
          <span class="shrink-0">{{ window.reset ? '重置（北京时间）' : '重置' }}</span>
          <time v-if="window.reset" :datetime="window.reset" class="text-right font-mono tabular-nums">{{ formatDateTime(window.reset, '—', 'Asia/Shanghai') }}</time>
          <span v-else>暂不可用</span>
        </div>
        <progress v-if="Number(window.limit) > 0" class="h-1.5 w-full accent-cp-primary" :aria-label="window.heading" :value="Math.min(Number(window.used), Number(window.limit))" :max="Number(window.limit)" />
      </div>
    </section>
  </BasePopover>
</template>
