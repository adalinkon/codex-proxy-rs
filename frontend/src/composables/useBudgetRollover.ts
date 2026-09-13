import type { MaybeRefOrGetter } from 'vue'
import type { BudgetAmounts } from '@/api/modules/users'
import { useEventListener } from '@vueuse/core'
import { computed, onScopeDispose, toValue, watch } from 'vue'

export function useBudgetRollover(budgets: MaybeRefOrGetter<readonly BudgetAmounts[]>, refresh: () => Promise<unknown>) {
  const nextReset = computed(() => Math.min(...toValue(budgets)
    .flatMap(budget => [budget.dailyResetsAt, budget.weeklyResetsAt])
    .filter((value): value is string => Boolean(value))
    .map(value => Date.parse(value))
    .filter(Number.isFinite)))
  let timer: ReturnType<typeof setTimeout> | undefined
  let disposed = false

  function schedule(minDelay = 1000) {
    clearTimeout(timer)
    if (disposed || !Number.isFinite(nextReset.value))
      return
    timer = setTimeout(async () => {
      if (document.hidden)
        return
      try {
        await refresh()
      }
      finally {
        // 失败或页面忙时稍后重试，避免过期时间导致紧密请求循环。
        schedule(30_000)
      }
    }, Math.min(2_147_483_647, Math.max(minDelay, nextReset.value - Date.now() + 100)))
  }

  watch(nextReset, () => schedule(), { immediate: true })
  useEventListener(document, 'visibilitychange', () => !document.hidden && schedule())
  useEventListener(window, 'focus', () => schedule())
  onScopeDispose(() => {
    disposed = true
    clearTimeout(timer)
  })
}
